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
SETTLE_TIME = 0.1                      # seconds after a (re)start in which glue cannot break: touching pieces settle first
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
    parts: tuple = ()          # (local positions, triangles) per convex part; more than one for a glued cluster


def convex_part(geo, prims, points, label, hull=False):
    """Validate one connected piece as a closed convex shell; returns its triangles over ``points`` (sorted point ids).
    ``hull``: the piece may be nearly convex (the solver collides with its convex hull anyway)."""
    vp, offsets = geo.topology.vertex_point, geo.topology.prim_offsets
    P = geo.P[points].astype(np.float64)
    local = P - P.mean(0)
    extent = float(np.ptp(P, axis=0).max())
    eps = max(extent * 2e-5, 1e-8)
    if len(points) < 4 or np.linalg.matrix_rank(local, tol=eps) < 3:
        raise ValueError(f"RBD piece {label} has no solid volume")
    edges, triangles = {}, []
    for p in prims:
        face = np.searchsorted(points, vp[offsets[p]:offsets[p + 1]])
        if len(face) < 3 or len(np.unique(face)) != len(face):
            raise ValueError(f"RBD piece {label} has a degenerate face")
        Q = local[face]
        n = np.cross(Q, np.roll(Q, -1, axis=0)).sum(0)
        length = np.linalg.norm(n)
        if length < eps * eps:
            raise ValueError(f"RBD piece {label} has a zero-area face")
        n /= length
        if np.max(np.abs((Q - Q[0]) @ n)) > eps:
            raise ValueError(f"RBD piece {label} has non-planar faces; triangulate first")
        if not hull and np.max((local - Q[0]) @ n) > eps:
            raise ValueError(f"RBD piece {label} is concave or inward wound; convex closed pieces only")
        for a, b in zip(face, np.roll(face, -1)):
            key = (min(a, b), max(a, b))
            edges.setdefault(key, []).append((a, b))
        triangles.extend((face[0], face[k], face[k + 1]) for k in range(1, len(face) - 1))
    if any(len(e) != 2 or e[0] != e[1][::-1] for e in edges.values()):
        raise ValueError(f"RBD piece {label} is open/non-manifold; use a closed solid (box for floor)")
    return np.asarray(triangles)


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
    compound = geo.prim.get("rbd_compound")
    use_hull = geo.prim.get("rbd_hull")
    defaults = dict(active=1, mass=1., friction=.6, restitution=0., linear_damping=.05,
                    angular_damping=.1, v=(0., 0., 0.), w=(0., 0., 0.))
    result, used = [], np.zeros(geo.npoints, bool)
    vp, offsets = geo.topology.vertex_point, geo.topology.prim_offsets
    for ident in np.unique(ids):
        prims = np.flatnonzero(ids == ident)
        comps = np.unique(components[prims])
        if len(comps) != 1 and (compound is None or not np.all(compound[prims])):
            raise ValueError(f"RBD ID {ident} names disconnected pieces; use unique IDs/id_offset when merging "
                             f"(or configure with 'compound' to move a glued cluster as one body)")
        corners = np.concatenate([np.arange(offsets[p], offsets[p + 1]) for p in prims])
        points = np.unique(vp[corners])
        if used[points].any():
            raise ValueError("RBD pieces share points; split shared points before configuring separate bodies")
        used[points] = True
        P = geo.P[points].astype(np.float64)
        center = P.mean(0)
        local = P - center
        parts = []
        for comp in comps:
            cprims = prims[components[prims] == comp]
            cpoints = np.unique(vp[np.concatenate([np.arange(offsets[p], offsets[p + 1]) for p in cprims])])
            tri = convex_part(geo, cprims, cpoints, ident, hull=use_hull is not None and bool(np.all(use_hull[cprims])))
            parts.append((geo.P[cpoints].astype(np.float64) - center, tri))
        triangles = parts[0][1] if len(parts) == 1 else np.zeros((0, 3), np.int64)
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
        if values['active'] not in (0, 1):
            raise ValueError('rbd_active must be 0 or 1')
        if any(not 0 <= values[k] <= 100 for k in ('linear_damping', 'angular_damping')):
            raise ValueError("RBD damping must be 0..100 per second")
        result.append(Piece(geo, int(ident), points, prims, corners, center, local,
                            triangles, bool(values['active']) and not static, values, tuple(parts)))
    if not used.all():
        raise ValueError("RBD does not support loose points alongside solid pieces")
    return result


def numbers(a):
    return " ".join(format(float(x), '.12g') for x in np.asarray(a).ravel())


CONSTRAINT_KINDS = ("glue", "pin", "hinge", "spring")


def validate_link_values(v):
    kind = v["kind"]
    if kind not in CONSTRAINT_KINDS:
        raise ValueError("unknown RBD constraint kind")
    for key in ("anchor", "anchor_b", "axis"):
        if np.shape(v[key]) != (3,) or not np.isfinite(v[key]).all():
            raise ValueError(f"constraint {key} must be a finite three-component vector")
    for key in ("lower", "upper", "stiffness", "damping", "rest_length", "strength", "break_torque"):
        if not np.isfinite(v[key]):
            raise ValueError(f"constraint {key} must be finite")
    if any(v[key] < 0 for key in ("stiffness", "damping", "strength", "break_torque")):
        raise ValueError("constraint stiffness, damping and break thresholds must be non-negative")
    if kind == "hinge" and np.linalg.norm(v["axis"]) < 1e-9:
        raise ValueError("hinge axis cannot be zero")
    if v["limited"] and kind not in ("hinge", "spring"):
        raise ValueError("limits apply only to hinge or spring; disable Limited for pin/glue")
    if v["limited"] and kind in ("hinge", "spring"):
        if v["lower"] >= v["upper"]:
            raise ValueError("constraint Upper must exceed Lower")
        if kind == "hinge" and not v["lower"] <= 0 <= v["upper"]:
            raise ValueError("hinge limits must include the initial zero angle")
        if kind == "spring" and v["lower"] < 0:
            raise ValueError("spring minimum length cannot be negative")
    if kind == "spring" and (v["rest_length"] < 0 and v["rest_length"] != -1):
        raise ValueError("spring Rest Length must be -1 (initial distance) or non-negative")
    if kind == "spring" and np.linalg.norm(np.asarray(v["anchor"]) - v["anchor_b"]) < 1e-9:
        raise ValueError("spring anchors must start at different positions")
    if kind in ("pin", "spring") and v["break_torque"]:
        raise ValueError("pin and spring have no direct torque threshold; use Strength in newtons")


@dataclass
class Link:
    a: int
    b: object
    kind: str
    strength: float
    length: float
    values: dict


def link_table(constraints, pieces):
    """Primitive links; missing kind remains the original breakable glue contract."""
    if constraints is None or constraints.nprims == 0:
        return []
    for name in ("rbd_a", "rbd_b"):
        a = constraints.prim.get(name)
        if a is None or a.ndim != 1 or a.dtype.kind not in "iu":
            raise ValueError(f"RBD constraints need an integer primitive attribute {name!r}")
    legacy_index = {p.id: i for i, p in enumerate(pieces)}
    index, ambiguous = {}, set()
    for i, p in enumerate(pieces):
        if "rbd_id" not in p.geo.prim:
            continue  # unconfigured static collider IDs are internal, not new link targets
        if p.id in index:
            ambiguous.add(p.id)
        index[p.id] = i
    def get(name, k, default):
        a = constraints.prim.get(name)
        if a is None:
            return default
        value = a[k]
        if np.shape(value) != np.shape(default):
            raise ValueError(f"constraint {name} has the wrong attribute shape")
        return value
    out = []
    for k in range(constraints.nprims):
        raw_kind = get("rbd_constraint", k, 0)
        if raw_kind not in range(len(CONSTRAINT_KINDS)):
            raise ValueError(f"constraint {k} has unknown rbd_constraint code {raw_kind}")
        kind = CONSTRAINT_KINDS[int(raw_kind)]
        # Old glue used last-ID-wins, including inferred static collider IDs. Preserve
        # that exact contract; new authored links use explicit, unambiguous configured IDs.
        strict = bool(get("rbd_schema", k, 0)) or kind != "glue"
        lookup = index if strict else legacy_index
        requested = [int(constraints.prim["rbd_a"][k])]
        if not get("rbd_world", k, 0):
            requested.append(int(constraints.prim["rbd_b"][k]))
        if strict and ambiguous.intersection(requested):
            raise ValueError("constraint body ID is ambiguous; configure unique IDs across bodies and colliders")
        try:
            a = lookup[requested[0]]
            b = None if get("rbd_world", k, 0) else lookup[requested[1]]
        except KeyError as e:
            raise ValueError(f"constraint {k} names rbd_id {e.args[0]}, which no body has") from None
        if a == b:
            if strict:
                raise ValueError("a constraint needs two different body IDs (or enable World)")
            continue  # legacy cluster links inside a single body do not constrain anything
        defaults = dict(anchor=(0., 0., 0.), anchor_b=(0., 0., 1.), axis=(0., 1., 0.),
                        limited=0, lower=0., upper=90., stiffness=100., damping=1., rest_length=-1.,
                        strength=0., break_torque=0., collide_connected=0)
        v = {key: get(key, k, default) for key, default in defaults.items()}
        v["kind"] = kind
        v["legacy_glue"] = not strict
        # Historical glue treats zero/negative/non-finite strength as unbreakable.
        if kind == "glue" and (not np.isfinite(v["strength"]) or v["strength"] < 0):
            v["strength"] = 0.
        validate_link_values(v)
        if kind == "hinge" and not pieces[a].active:
            raise ValueError("hinge Body A must be active; swap A and B if the other body should rotate")
        out.append(Link(a, b, kind, float(v["strength"]), float(get("length", k, 0.)), v))
    return out


def part_volume(local, triangles):
    T = local[triangles]
    return abs(float(np.einsum("ij,ij->i", T[:, 0], np.cross(T[:, 1], T[:, 2])).sum() / 6.0))


class World:
    def __init__(self, bodies, colliders, settings, constraints=None):
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
        body_elements = []
        for i, p in enumerate(self.pieces):
            body = ET.SubElement(wb, 'body', name=f'b{i}', pos=numbers(p.center))
            body_elements.append(body)
            if p.active:
                ET.SubElement(body, 'freejoint', name=f'j{i}', align='false')
            bounce = float(p.values['restitution'])
            loge = math.log(max(bounce, 1e-6))
            ratio = 1. if bounce == 0 else max(.05, -loge / math.sqrt(math.pi**2 + loge**2))
            volumes = [part_volume(local, tri) for local, tri in p.parts] if len(p.parts) > 1 else [1.]
            for k, (local, tri) in enumerate(p.parts):         # a glued cluster is one body with a mesh per convex part
                ET.SubElement(assets, 'mesh', name=f'm{i}_{k}', vertex=numbers(local),
                              face=' '.join(str(int(x)) for x in tri.ravel()), inertia='convex')
                share = volumes[k] / sum(volumes) if sum(volumes) > 0 else 1. / len(volumes)
                ET.SubElement(body, 'geom', type='mesh', mesh=f'm{i}_{k}', mass=str(float(p.values['mass']) * share),
                              friction=numbers([p.values['friction'], .005, .0001]), condim='3',
                              margin=str(settings['margin']), solref=numbers([max(.004, 2*self.dt), ratio]),
                              solimp='.99 .99 .001', solmix='1')
        self.links = link_table(constraints, self.pieces)
        self.pairs = {}
        self.link_equalities, self.link_tendons, self.link_joints = [], [], []
        eq = ET.SubElement(root, 'equality') if self.links else None
        skip = ET.SubElement(root, 'contact') if self.links else None
        tendon = ET.SubElement(root, 'tendon') if any(l.kind == 'spring' for l in self.links) else None
        for k, link in enumerate(self.links):
            a, b, v = link.a, link.b, link.values
            eqname, tenname, jointname = None, None, None
            other = f'b{b}' if b is not None else 'world'
            anchor = np.asarray(v['anchor'], float)
            soft = dict(solref=numbers([max(.008, 2 * self.dt), 1.]), solimp='.999999 .999999 .001')
            if link.kind == 'glue':
                eqname = f'g{k}'
                ET.SubElement(eq, 'weld', name=eqname, body1=f'b{a}', body2=other,
                              solref=numbers([max(.03, 4 * self.dt), 1.]), solimp='.95 .99 .001')
            elif link.kind == 'pin':
                eqname = f'g{k}'
                # Site pairs remove any ambiguity over connect's body-local anchor convention.
                ET.SubElement(body_elements[a], 'site', name=f'pa{k}', pos=numbers(anchor-self.pieces[a].center), size='.001')
                parent = body_elements[b] if b is not None else wb
                ET.SubElement(parent, 'site', name=f'pb{k}', pos=numbers(anchor-(self.pieces[b].center if b is not None else 0)), size='.001')
                ET.SubElement(eq, 'connect', name=eqname, site1=f'pa{k}', site2=f'pb{k}', **soft)
            elif link.kind == 'hinge':
                eqname, jointname = f'g{k}', f'h{k}'
                parent = body_elements[b] if b is not None else wb
                helper = ET.SubElement(parent, 'body', name=f'hinge{k}',
                    pos=numbers(anchor-(self.pieces[b].center if b is not None else 0)))
                # A revolute joint is a tree edge in MuJoCo. A tiny auxiliary rotor plus
                # a removable weld gives graph connections and breakage without reparenting.
                mass = max(1e-9, float(self.pieces[a].values['mass']) * 1e-6)
                ET.SubElement(helper, 'inertial', pos='0 0 0', mass=str(mass), diaginertia=numbers([mass * 1e-3]*3))
                axis = np.asarray(v['axis'], float); axis /= np.linalg.norm(axis)
                armature = float(self.pieces[a].values['mass']) * .001 * max(
                    float(np.sum((self.pieces[a].center-anchor)**2)), .01)
                ET.SubElement(helper, 'joint', name=jointname, type='hinge', axis=numbers(axis),
                    limited='true' if v['limited'] else 'false', range=numbers(np.radians([v['lower'], v['upper']])),
                    damping=str(float(v['damping'])), armature=str(armature),
                    solreflimit=numbers([max(.008, 2*self.dt), 1.]), solimplimit='.999999 .999999 .001')
                ET.SubElement(eq, 'weld', name=eqname, body1=f'hinge{k}', body2=f'b{a}', **soft)
            else:
                tenname = f't{k}'
                for suffix, bodyid, position in [('a', a, anchor), ('b', b, np.asarray(v['anchor_b'], float))]:
                    parent = body_elements[bodyid] if bodyid is not None else wb
                    ET.SubElement(parent, 'site', name=f's{suffix}{k}',
                        pos=numbers(position-(self.pieces[bodyid].center if bodyid is not None else 0)), size='.001')
                spatial = ET.SubElement(tendon, 'spatial', name=tenname, stiffness=str(float(v['stiffness'])),
                    damping=str(float(v['damping'])), springlength=str(float(v['rest_length'])),
                    limited='true' if v['limited'] else 'false', range=numbers([v['lower'], v['upper']]),
                    solreflimit=numbers([max(.008, 2*self.dt), 1.]), solimplimit='.999999 .999999 .001')
                ET.SubElement(spatial, 'site', site=f'sa{k}'); ET.SubElement(spatial, 'site', site=f'sb{k}')
            self.link_equalities.append(eqname)
            self.link_tendons.append(tenname)
            self.link_joints.append(jointname)
            if b is not None and not v['collide_connected']:
                key = (min(a, b), max(a, b))
                if key not in self.pairs:
                    ET.SubElement(skip, 'exclude', body1=f'b{key[0]}', body2=f'b{key[1]}')
                    self.pairs[key] = (len(self.pairs), [])
                self.pairs[key][1].append(k)
        self.model = self.mj.MjModel.from_xml_string(ET.tostring(root, encoding='unicode'))
        def lookup(names, kind):
            return [self.mj.mj_name2id(self.model, kind, name) if name else -1 for name in names]
        self.link_equalities = lookup(self.link_equalities, self.mj.mjtObj.mjOBJ_EQUALITY)
        self.link_tendons = lookup(self.link_tendons, self.mj.mjtObj.mjOBJ_TENDON)
        self.link_joints = lookup(self.link_joints, self.mj.mjtObj.mjOBJ_JOINT)
        self.tendon_initial = {name: getattr(self.model, name).copy() for name in
                               ('tendon_stiffness', 'tendon_damping', 'tendon_limited')}
        self.joint_initial = {name: getattr(self.model, name).copy() for name in ('jnt_limited', 'dof_damping')}
        self.data = self.mj.MjData(self.model)
        self.body_ids = [self.mj.mj_name2id(self.model, self.mj.mjtObj.mjOBJ_BODY, f'b{i}') for i in range(len(self.pieces))]
        if any(not link.values['legacy_glue'] for link in self.links):
            # MuJoCo sorts exclusions by body signature, not XML insertion order.
            # Hinge helper bodies also mean piece index is not body ID minus one.
            for pair, (_slot, ids) in list(self.pairs.items()):
                ba, bb = sorted(self.body_ids[i] for i in pair)
                signature = (ba << 16) + bb
                slot = int(np.flatnonzero(self.model.exclude_signature == signature)[0])
                self.pairs[pair] = (slot, ids)
        self.dofs = []
        for i, p in enumerate(self.pieces):
            if p.active:
                j = self.mj.mj_name2id(self.model, self.mj.mjtObj.mjOBJ_JOINT, f'j{i}')
                d = int(self.model.jnt_dofadr[j])
                self.data.qvel[d:d+3] = p.values['v']
                self.data.qvel[d+3:d+6] = p.values['w']
                self.dofs.append((d, p))
        self.initial_qvel = self.data.qvel.copy()
        self.broken = np.zeros(len(self.links), bool)
        self.exclusions = self.model.exclude_signature.copy() if self.links else None
        self.since_start = 0
        extent = [max(float(np.ptp(p.local, axis=0).max()), 1e-6) for p in self.pieces]
        self.reach = [l.length if l.length > 0 else min(extent[l.a], extent[l.b] if l.b is not None else extent[l.a])
                      for l in self.links]
        self.frame = settings['start_frame']
        self.frames = OrderedDict()
        self.cache_bytes = 0
        self.steps = 0
        self.mj.mj_forward(self.model, self.data)

    def restart(self):
        self.mj.mj_resetData(self.model, self.data)
        self.data.qvel[:] = self.initial_qvel
        for name, value in (self.tendon_initial | self.joint_initial).items():
            getattr(self.model, name)[:] = value
        self.broken[:] = False
        if self.links:
            self.model.exclude_signature[:] = self.exclusions
        self.since_start = 0
        self.mj.mj_forward(self.model, self.data)
        self.frame = self.settings['start_frame']

    def snap_links(self):
        """Evaluate actual solver reactions and spring force after each substep; disable broken mechanisms."""
        if not self.links or self.since_start * self.dt < SETTLE_TIME:
            return
        d, m = self.data, self.model
        eqrows = np.flatnonzero(d.efc_type[:d.nefc] == int(self.mj.mjtConstraint.mjCNSTR_EQUALITY))
        limitrows = np.flatnonzero(d.efc_type[:d.nefc] == int(self.mj.mjtConstraint.mjCNSTR_LIMIT_TENDON))
        legacy_broke = False
        for k, link in enumerate(self.links):
            if legacy_broke and link.values["legacy_glue"]:
                continue  # preserve the historical one-glue-break-per-substep behavior
            if self.broken[k] or (link.strength <= 0 and link.values['break_torque'] <= 0):
                continue
            torque = 0.
            if link.kind == 'spring':
                tid = self.link_tendons[k]
                rest = m.tendon_lengthspring[tid]
                length = d.ten_length[tid]
                displacement = length - np.clip(length, rest[0], rest[1])
                passive = -m.tendon_stiffness[tid] * displacement - m.tendon_damping[tid] * d.ten_velocity[tid]
                # Limit reactions are also axial loads. Count their magnitudes conservatively
                # even if passive and stop forces momentarily oppose one another.
                stops = d.efc_force[limitrows[d.efc_id[limitrows] == tid]]
                force = abs(float(passive)) + float(np.abs(stops).sum())
            else:
                eid = self.link_equalities[k]
                reaction = d.efc_force[eqrows[d.efc_id[eqrows] == eid]]
                force = float(np.linalg.norm(reaction[:3]))
                if len(reaction) >= 6 and link.values['break_torque'] > 0:
                    # Convert the whole reaction through MuJoCo's actual Jacobian,
                    # then measure the moment at the hinge pivot (glue: body origin).
                    loads = np.zeros(d.nefc)
                    mine = eqrows[d.efc_id[eqrows] == eid]
                    loads[mine] = reaction
                    generalized = np.zeros(m.nv)
                    self.mj.mj_mulJacTVec(m, d, generalized, loads)
                    moving = link.a if self.pieces[link.a].active else link.b
                    if moving is not None and self.pieces[moving].active:
                        jid = self.mj.mj_name2id(m, self.mj.mjtObj.mjOBJ_JOINT, f'j{moving}')
                        dof = int(m.jnt_dofadr[jid])
                        bid = self.body_ids[moving]
                        R = d.xmat[bid].reshape(3, 3)
                        moment = R @ generalized[dof+3:dof+6]
                        if link.kind == 'hinge':
                            pivot = d.xpos[bid] + R @ (np.asarray(link.values['anchor'])-self.pieces[moving].center)
                            moment -= np.cross(pivot-d.xpos[bid], generalized[dof:dof+3])
                        torque = float(np.linalg.norm(moment))
                if link.kind == 'glue':
                    # Keep the historic glue strength metric exactly (raw angular multiplier / reach).
                    force += float(np.linalg.norm(reaction[3:6])) / self.reach[k]
            if ((link.strength > 0 and force > link.strength) or
                (link.values['break_torque'] > 0 and torque > link.values['break_torque'])):
                eid, tid = self.link_equalities[k], self.link_tendons[k]
                if eid >= 0:
                    d.eq_active[eid] = 0
                if tid >= 0:
                    m.tendon_stiffness[tid] = m.tendon_damping[tid] = 0.
                    m.tendon_limited[tid] = 0
                jid = self.link_joints[k]
                if jid >= 0:
                    m.jnt_limited[jid] = 0
                    m.dof_damping[m.jnt_dofadr[jid]] = 0.
                self.broken[k] = True
                legacy_broke |= link.values["legacy_glue"]
                for _key, (slot, ids) in self.pairs.items():
                    if k in ids and self.broken[ids].all():
                        m.exclude_signature[slot] = -1

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
                self.since_start += 1
                self.snap_links()
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
        value = (pos, rot, vel, self.broken.copy())
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
        pos, rotations, velocities, broken = self.pose(target)
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
        if self.links:
            for kind in CONSTRAINT_KINDS:
                ids = [k for k, l in enumerate(self.links) if l.kind == kind]
                if ids:
                    out.detail['rbd'][kind] = dict(links=len(ids), broken=int(broken[ids].sum()),
                        broken_links=[k for k in ids if broken[k]])
        return out

    def close(self):
        self.frames.clear()
        self.cache_bytes = 0
        self.model = self.data = None


class Manager:
    """Per-evaluator worlds, at most eight resident nodes. No backend imports until used."""
    def __init__(self):
        self.worlds = OrderedDict()

    def solve(self, node_id, bodies, colliders, settings, constraints=None):
        if not math.isfinite(timectx.fps()) or timectx.fps() <= 0 or not math.isfinite(timectx.frame()):
            raise ValueError("RBD frame must be finite and FPS positive")
        signature = digest((bodies, colliders, constraints), settings)
        existing = self.worlds.get(node_id)
        if existing is None or existing[0] != signature:
            if existing:
                existing[1].close()
                del self.worlds[node_id]
            world = World(bodies, colliders, settings, constraints)
            self.worlds[node_id] = (signature, world)
            while len(self.worlds) > 8:
                self.worlds.popitem(last=False)[1][1].close()
        else:
            world = existing[1]
            self.worlds.move_to_end(node_id)
        target = max(settings['start_frame'], math.floor(timectx.frame()))
        if target - settings['start_frame'] > 100000:
            raise ValueError("RBD frame range exceeds 100000 frames")
        try:
            return world.output(target)
        except Exception:
            world.close()
            self.worlds.pop(node_id, None)
            raise

    def clear(self):
        for _, world in self.worlds.values():
            world.close()
        self.worlds.clear()


def solve(bodies, colliders, settings, constraints=None):
    owner = _OWNER.get()
    if owner is None:
        manager = Manager()
        try:
            return manager.solve('direct', bodies, colliders, settings, constraints)
        finally:
            manager.clear()
    return owner[0].solve(owner[1], bodies, colliders, settings, constraints)
