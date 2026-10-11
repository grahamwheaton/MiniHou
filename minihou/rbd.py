"""Owned MuJoCo worlds and immutable pose caches; no Blender or global physics world.

Only convex, closed, consistently wound polygon shells are accepted. Render topology
never goes through the physics backend. MuJoCo owns collision hulls/inertia; MiniHou
applies body transforms to the original positions and all normal domains.
"""
from __future__ import annotations

from collections import OrderedDict
from contextlib import contextmanager
from contextvars import ContextVar
from dataclasses import dataclass
import hashlib
import json
import math
import xml.etree.ElementTree as ET

import numpy as np

from . import timectx

BACKEND_VERSION = "3.15.0"
_OWNER = ContextVar("minihou_rbd_owner", default=None)


@contextmanager
def owned(manager, node_id):
    token = _OWNER.set((manager, node_id))
    try:
        yield
    finally:
        _OWNER.reset(token)


def backend():
    try:
        import mujoco
    except ImportError:
        raise ValueError("RBD needs the optional physics extra: pip install 'minihou[physics]' (MuJoCo 3.15.0)") from None
    if mujoco.__version__ != BACKEND_VERSION:
        raise ValueError(f"RBD repeatability requires mujoco=={BACKEND_VERSION}; found {mujoco.__version__}")
    return mujoco


def digest(geos, settings):
    h = hashlib.blake2b(digest_size=20)
    h.update(json.dumps([settings, timectx.fps(), BACKEND_VERSION], sort_keys=True).encode())
    for geo in geos:
        if geo is None:
            h.update(b"none")
            continue
        h.update(geo.topology.digest.encode())
        for dom in ("point", "vertex", "prim"):
            for name, a in sorted(geo.table(dom).items()):
                h.update(f"{dom}/{name}/{a.dtype}/{a.shape}".encode())
                h.update(a.tobytes())
        h.update(json.dumps(geo.detail, sort_keys=True, default=str).encode())
    return h.hexdigest()


@dataclass
class Piece:
    geo: object
    id: int
    points: np.ndarray
    prims: np.ndarray
    corners: np.ndarray
    center: np.ndarray
    local: np.ndarray
    triangles: np.ndarray
    active: bool
    values: dict


def pieces(geo, static=False):
    if geo is None or (geo.npoints == 0 and geo.nprims == 0):
        return []
    from .nodes.attrib_tools import piece_labels
    if geo.topology.has_open or not geo.nprims:
        raise ValueError("RBD requires closed solid polygon meshes, not points, curves or open sheets")
    if not np.isfinite(geo.P).all():
        raise ValueError("RBD positions must be finite")
    if geo.detail.get("instance_objects"):
        raise ValueError("RBD needs actual mesh pieces, not Blender visual instances; use Copy to Points")
    _, components = piece_labels(geo)
    ids = geo.prim.get("rbd_id")
    if ids is None:
        if not static:
            raise ValueError("connect RBD Configure before RBD Solver")
        ids = components
    if ids.shape != (geo.nprims,) or ids.dtype.kind not in "iu":
        raise ValueError("rbd_id must be a scalar primitive integer attribute")
    defaults = dict(active=1, mass=1., friction=.6, restitution=0., linear_damping=.05,
                    angular_damping=.1, v=(0., 0., 0.), w=(0., 0., 0.))
    result, used = [], np.zeros(geo.npoints, bool)
    vp, offsets = geo.topology.vertex_point, geo.topology.prim_offsets
    for ident in np.unique(ids):
        prims = np.flatnonzero(ids == ident)
        if len(np.unique(components[prims])) != 1:
            raise ValueError(f"RBD ID {ident} names disconnected pieces; use unique IDs/id_offset when merging")
        corners = np.concatenate([np.arange(offsets[p], offsets[p + 1]) for p in prims])
        points = np.unique(vp[corners])
        if used[points].any():
            raise ValueError("RBD pieces share points; split shared points before configuring separate bodies")
        used[points] = True
        P = geo.P[points].astype(np.float64)
        center = P.mean(0)
        local = P - center
        extent = float(np.ptp(P, axis=0).max())
        eps = max(extent * 2e-5, 1e-8)
        if len(points) < 4 or np.linalg.matrix_rank(local, tol=eps) < 3:
            raise ValueError(f"RBD piece {ident} has no solid volume")
        edges, triangles = {}, []
        for p in prims:
            face = np.searchsorted(points, vp[offsets[p]:offsets[p + 1]])
            if len(face) < 3 or len(np.unique(face)) != len(face):
                raise ValueError(f"RBD piece {ident} has a degenerate face")
            Q = local[face]
            n = np.cross(Q, np.roll(Q, -1, axis=0)).sum(0)
            length = np.linalg.norm(n)
            if length < eps * eps:
                raise ValueError(f"RBD piece {ident} has a zero-area face")
            n /= length
            if np.max(np.abs((Q - Q[0]) @ n)) > eps:
                raise ValueError(f"RBD piece {ident} has non-planar faces; triangulate first")
            if np.max((local - Q[0]) @ n) > eps:
                raise ValueError(f"RBD piece {ident} is concave or inward wound; convex closed pieces only")
            for a, b in zip(face, np.roll(face, -1)):
                key = (min(a, b), max(a, b))
                edges.setdefault(key, []).append((a, b))
            triangles.extend((face[0], face[k], face[k + 1]) for k in range(1, len(face) - 1))
        if any(len(e) != 2 or e[0] != e[1][::-1] for e in edges.values()):
            raise ValueError(f"RBD piece {ident} is open/non-manifold; use a closed solid (box for floor)")
        values = {}
        for name, default in defaults.items():
            a = geo.prim.get("rbd_" + name)
            value = np.asarray(default) if a is None else a[prims[0]]
            if a is not None and not np.all(a[prims] == value):
                raise ValueError(f"rbd_{name} must be constant within piece {ident}")
            if np.shape(value) != np.shape(default) or not np.isfinite(value).all():
                raise ValueError(f"invalid rbd_{name} on piece {ident}")
            values[name] = value
        if values['mass'] <= 0 or not 0 <= values['friction'] <= 10 or not 0 <= values['restitution'] <= 1:
            raise ValueError("RBD mass must be positive; friction 0..10 and restitution 0..1")
        if any(not 0 <= values[k] <= 100 for k in ('linear_damping', 'angular_damping')):
            raise ValueError("RBD damping must be 0..100 per second")
        result.append(Piece(geo, int(ident), points, prims, corners, center, local,
                            np.asarray(triangles), bool(values['active']) and not static, values))
    if not used.all():
        raise ValueError("RBD does not support loose points alongside solid pieces")
    return result


def numbers(a):
    return " ".join(format(float(x), '.12g') for x in np.asarray(a).ravel())


class World:
    def __init__(self, bodies, colliders, settings):
        self.settings = dict(settings)
        self.bodies, self.colliders = bodies, colliders
        self.pieces = pieces(bodies) + pieces(colliders, static=True)
        if len(self.pieces) > 2000 or sum(len(p.points) for p in self.pieces) > 200000:
            raise ValueError("RBD initial limit: 2000 bodies / 200000 collision vertices; simplify inputs")
        self.mj = backend()
        self.dt = 1. / (timectx.fps() * settings['substeps'])
        root = ET.Element('mujoco', model='MiniHou_RBD')
        ET.SubElement(root, 'compiler', angle='radian', autolimits='true', alignfree='false')
        ET.SubElement(root, 'option', timestep=str(self.dt), gravity=numbers(settings['gravity']),
                      iterations=str(settings['iterations']), tolerance='1e-10', integrator='implicitfast',
                      cone='elliptic', jacobian='sparse')
        assets, wb = ET.SubElement(root, 'asset'), ET.SubElement(root, 'worldbody')
        for i, p in enumerate(self.pieces):
            ET.SubElement(assets, 'mesh', name=f'm{i}', vertex=numbers(p.local),
                          face=' '.join(str(int(x)) for x in p.triangles.ravel()), inertia='convex')
            body = ET.SubElement(wb, 'body', name=f'b{i}', pos=numbers(p.center))
            if p.active:
                ET.SubElement(body, 'freejoint', name=f'j{i}', align='false')
            bounce = float(p.values['restitution'])
            loge = math.log(max(bounce, 1e-6))
            ratio = 1. if bounce == 0 else max(.05, -loge / math.sqrt(math.pi**2 + loge**2))
            ET.SubElement(body, 'geom', type='mesh', mesh=f'm{i}', mass=str(float(p.values['mass'])),
                          friction=numbers([p.values['friction'], .005, .0001]), condim='3',
                          margin=str(settings['margin']), solref=numbers([max(.004, 2*self.dt), ratio]),
                          solimp='.99 .99 .001', solmix='1')
        self.model = self.mj.MjModel.from_xml_string(ET.tostring(root, encoding='unicode'))
        self.data = self.mj.MjData(self.model)
        self.body_ids = [self.mj.mj_name2id(self.model, self.mj.mjtObj.mjOBJ_BODY, f'b{i}') for i in range(len(self.pieces))]
        self.dofs = []
        for i, p in enumerate(self.pieces):
            if p.active:
                j = self.mj.mj_name2id(self.model, self.mj.mjtObj.mjOBJ_JOINT, f'j{i}')
                d = int(self.model.jnt_dofadr[j])
                self.data.qvel[d:d+3] = p.values['v']
                self.data.qvel[d+3:d+6] = p.values['w']
                self.dofs.append((d, p))
        self.initial_qvel = self.data.qvel.copy()
        self.frame = settings['start_frame']
        self.frames = OrderedDict()
        self.cache_bytes = 0
        self.steps = 0
        self.mj.mj_forward(self.model, self.data)

    def restart(self):
        self.mj.mj_resetData(self.model, self.data)
        self.data.qvel[:] = self.initial_qvel
        self.mj.mj_forward(self.model, self.data)
        self.frame = self.settings['start_frame']

    def pose(self, target):
        if target in self.frames:
            self.frames.move_to_end(target)
            return self.frames[target]
        if target < self.frame:
            self.restart()  # rebuild all contact/warmstart state; never resume from a pose snapshot
        while self.frame < target:
            for _ in range(self.settings['substeps']):
                for d, p in self.dofs:
                    self.data.qvel[d:d+3] *= math.exp(-float(p.values['linear_damping']) * self.dt)
                    self.data.qvel[d+3:d+6] *= math.exp(-float(p.values['angular_damping']) * self.dt)
                self.mj.mj_step(self.model, self.data)
                self.steps += 1
                if not np.isfinite(self.data.qpos).all() or any(w.number for w in self.data.warning):
                    raise ValueError("RBD numerical instability; reduce velocities/scale extremes or increase substeps")
            self.frame += 1
        self.mj.mj_forward(self.model, self.data)
        pos = self.data.xpos[self.body_ids].copy()
        rot = self.data.xmat[self.body_ids].reshape(-1, 3, 3).copy()
        vel = np.zeros((len(self.pieces), 6))
        for i, bid in enumerate(self.body_ids):
            self.mj.mj_objectVelocity(self.model, self.data, self.mj.mjtObj.mjOBJ_BODY, bid, vel[i], 0)
        value = (pos, rot, vel)
        size = sum(a.nbytes for a in value)
        budget = self.settings['cache_mb'] * 1024**2
        if size <= budget:
            while self.frames and self.cache_bytes + size > budget:
                _, old = self.frames.popitem(last=False)
                self.cache_bytes -= sum(a.nbytes for a in old)
            self.frames[target] = value
            self.cache_bytes += size
        return value

    def output(self, target):
        pos, rotations, velocities = self.pose(target)
        out = self.bodies.copy()
        P = out.P.copy()
        out.point['P'] = P
        for domain in ('point', 'vertex', 'prim'):
            normal = out.table(domain).get('N')
            if normal is not None:
                if normal.shape != (out.table(domain).size, 3):
                    raise ValueError("RBD normal N must be a three-component vector")
                out.table(domain)['N'] = normal.copy()
        v = np.zeros((out.nprims, 3), np.float32)
        w = np.zeros_like(v)
        for i, p in enumerate(self.pieces):
            if p.geo is not self.bodies:
                continue
            R = rotations[i]
            P[p.points] = p.local @ R.T + pos[i]
            for dom, indices in [('point', p.points), ('vertex', p.corners), ('prim', p.prims)]:
                if 'N' in out.table(dom):
                    out.table(dom)['N'][indices] = p.geo.table(dom)['N'][indices] @ R.T
            w[p.prims], v[p.prims] = velocities[i, :3], velocities[i, 3:]
        out.prim['rbd_v'], out.prim['rbd_w'] = v, w
        if self.settings['include_colliders'] and self.colliders is not None:
            from .nodes.merge import merge
            out = merge([out, self.colliders])
        out.detail['rbd'] = dict(backend='mujoco', version=BACKEND_VERSION, frame=target,
                                 bodies=len(self.pieces))
        return out

    def close(self):
        self.frames.clear()
        self.cache_bytes = 0
        self.model = self.data = None


class Manager:
    """Per-evaluator worlds, at most eight resident nodes. No backend imports until used."""
    def __init__(self):
        self.worlds = OrderedDict()

    def solve(self, node_id, bodies, colliders, settings):
        if not math.isfinite(timectx.fps()) or timectx.fps() <= 0 or not math.isfinite(timectx.frame()):
            raise ValueError("RBD frame must be finite and FPS positive")
        signature = digest((bodies, colliders), settings)
        existing = self.worlds.get(node_id)
        if existing is None or existing[0] != signature:
            if existing:
                existing[1].close()
                del self.worlds[node_id]
            world = World(bodies, colliders, settings)
            self.worlds[node_id] = (signature, world)
            while len(self.worlds) > 8:
                self.worlds.popitem(last=False)[1][1].close()
        else:
            world = existing[1]
            self.worlds.move_to_end(node_id)
        target = max(settings['start_frame'], math.floor(timectx.frame()))
        if target - settings['start_frame'] > 100000:
            raise ValueError("RBD frame range exceeds 100000 frames")
        return world.output(target)

    def clear(self):
        for _, world in self.worlds.values():
            world.close()
        self.worlds.clear()


def solve(bodies, colliders, settings):
    owner = _OWNER.get()
    if owner is None:
        manager = Manager()
        try:
            return manager.solve('direct', bodies, colliders, settings)
        finally:
            manager.clear()
    return owner[0].solve(owner[1], bodies, colliders, settings)
