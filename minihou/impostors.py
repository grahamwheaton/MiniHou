"""Impostors for instances: what the viewport draws for the same shape repeated many times.

A scene of a hundred thousand trees is a hundred thousand copies of a few shapes. Reducing the whole thing by clustering (what
:func:`minihou.proxy.decimate` does for any geometry) turns every tree into the same blob; keeping the copies is too heavy for the browser.
The viewer instead gets each *shape* once (at three levels of detail) and one transform per instance, and picks the level per instance from
how big it is on screen (see ``preview.js``). Far away an instance is a bounding box, then a dot.

Two kinds of instances are found:

* ``points``: points with an ``instance`` attribute (the ``instance`` node, USD point instancers, PCG), with ``rot`` / ``scale`` / ``orient`` /
  ``N`` as :func:`minihou.nodes.copy_to_points.copy_to_points` reads them. The shapes are the Blender objects named in the detail attribute
  ``instance_objects`` when the live link supplies them, else a box (the size is ``instance_bounds`` of the detail when a USD file gave them,
  else a unit box times the instance's scale: ``approximate``). The points themselves stay in the preview (selecting them works).
* ``blocks``: copies that were really made (``copy_to_points``, ``for_each`` pieces...). Geometry that is a block of points / vertices /
  primitives repeated end to end, whose every repetition is an affine copy (within a small tolerance) of the first one, is recognised
  without any help from the nodes: nothing is stored on the geometry, so nothing can go stale. The first block is the shape and a least-squares fit
  gives each instance's transform. This only replaces the geometry in a view that would be reduced anyway; at full detail (selecting,
  painting) the real geometry is sent as before.

Never changes the cooked geometry: this is display only.
"""
from __future__ import annotations

import math
import threading
import weakref
from dataclasses import dataclass, field
from typing import Dict, List, Optional, Tuple

import numpy as np

MIN_INSTANCES = 8                 # fewer repetitions than this are not worth special drawing
MIN_BLOCK_POINTS = 8              # nor are blocks this small (a quad soup is not "instances")
MIN_BLOCK_PRIMS = 4
FIT_TOL = 2e-3                    # a block is an instance of the first when its points are within this fraction of its size of the affine copy
MAX_SENT = 1_500_000              # most instances sent to the viewer (past that an even sample is drawn)
LOD_TRIS = (4000, 300)            # triangles of the detailed and of the coarse level of a shape


@dataclass
class Proto:
    name: str
    bounds: np.ndarray                      # (2, 3) local-space min / max
    geo: Optional[object] = None            # the shape, when we have it


@dataclass
class InstanceSet:
    kind: str                               # "points" | "blocks"
    protos: List[Proto]
    M: np.ndarray                           # (N, 3, 4) float32: local point p -> M @ [p, 1]
    proto: np.ndarray                       # (N,) int32 index into protos
    color: Optional[np.ndarray] = None      # (N, 3) float32
    approximate: bool = False               # a shape we only know the size of (or not even that)
    total: int = 0                          # instances before sampling
    sampled: bool = False
    replaces: bool = False                  # the real geometry is not sent along (a block-kind view)
    full_tris: int = 0                      # triangles the real instances would draw


# ---------------------------------------------------------------------------------------------------------------- blocks
def _divisors_desc(g: int, lowest: int) -> List[int]:
    out = set()
    i = 1
    while i * i <= g:
        if g % i == 0:
            out.add(i)
            out.add(g // i)
        i += 1
    return sorted((d for d in out if d >= lowest), reverse=True)


def _fit_blocks(P: np.ndarray, T: int, S: int) -> Optional[np.ndarray]:
    """The (T, 3, 4) affine maps taking block 0's points to each block's, or None when some block is not one."""
    X0 = P[:S].astype(np.float64)
    A = np.hstack([X0, np.ones((S, 1))])                       # (S, 4)
    pinv = np.linalg.pinv(A)                                   # (4, S)
    M = np.empty((T, 3, 4), np.float32)
    step = max(1, 400_000 // max(S, 1))
    for a in range(0, T, step):
        b = min(T, a + step)
        Pk = P[a * S:b * S].reshape(b - a, S, 3).astype(np.float64)
        C = np.einsum("as,tsc->tac", pinv, Pk)                 # (t, 4, 3)
        fit = np.einsum("sa,tac->tsc", A, C)
        res = np.abs(fit - Pk).max(axis=(1, 2))
        ext = (Pk.max(axis=1) - Pk.min(axis=1)).max(axis=1)
        if (res > FIT_TOL * np.maximum(ext, 1e-9) + 1e-7).any():
            return None
        M[a:b] = np.transpose(C, (0, 2, 1))
    return M


def _block_geometry(geo, S: int, V: int, Pr: int):
    from .geometry import Geometry
    t = geo.topology
    closed = t.closed_mask()[:Pr].copy() if t.has_open else None
    out = Geometry.from_arrays(np.ascontiguousarray(geo.P[:S]), t.vertex_point[:V].copy(), t.prim_offsets[:Pr + 1].copy(), closed)
    out.freeze()
    return out


def _bounds_of(geo) -> np.ndarray:
    if not geo.npoints:
        return np.zeros((2, 3))
    return np.stack([geo.P.min(0), geo.P.max(0)]).astype(np.float64)


_BLOCKS: "weakref.WeakKeyDictionary" = weakref.WeakKeyDictionary()
_BLOCKS_LOCK = threading.Lock()
_NONE = object()


def blocks(geo) -> Optional[InstanceSet]:
    """The repeated blocks of ``geo`` as instances, or None (memoised per frozen geometry)."""
    with _BLOCKS_LOCK:
        hit = _BLOCKS.get(geo, _NONE)
    if hit is not _NONE:
        return hit
    found = _detect_blocks(geo)
    if geo.frozen:
        with _BLOCKS_LOCK:
            _BLOCKS[geo] = found
    return found


def _detect_blocks(geo) -> Optional[InstanceSet]:
    t = geo.topology
    n, nv, npr = geo.npoints, t.nvertices, t.nprims
    if npr < MIN_INSTANCES * MIN_BLOCK_PRIMS or n < MIN_INSTANCES * MIN_BLOCK_POINTS:
        return None
    vp = t.vertex_point.astype(np.int64)
    sizes = np.diff(t.prim_offsets.astype(np.int64))
    closed = t.closed_mask() if t.has_open else None
    for T in _divisors_desc(math.gcd(math.gcd(n, nv), npr), MIN_INSTANCES):
        S, V, Pr = n // T, nv // T, npr // T
        if S < MIN_BLOCK_POINTS or Pr < MIN_BLOCK_PRIMS or not V:
            continue
        if not (np.array_equal(sizes[:Pr], sizes[Pr:2 * Pr]) and np.array_equal(vp[V:2 * V] - S, vp[:V])):     # two blocks first: cheap
            continue
        if vp[:V].min() < 0 or vp[:V].max() >= S:
            continue
        if not np.array_equal(vp.reshape(T, V), vp[:V][None, :] + (np.arange(T, dtype=np.int64) * S)[:, None]):
            continue
        if not (sizes.reshape(T, Pr) == sizes[:Pr][None, :]).all():
            continue
        if closed is not None and not (closed.reshape(T, Pr) == closed[:Pr][None, :]).all():
            continue
        M = _fit_blocks(geo.P, T, S)
        if M is None:
            continue
        proto_geo = _block_geometry(geo, S, V, Pr)
        color = None
        cd = geo.point.get("Cd")
        if cd is not None and cd.ndim == 2 and cd.shape[1] >= 3 and len(cd) == n:
            color = np.clip(cd[:, :3].reshape(T, S, 3).astype(np.float64).mean(axis=1), 0.0, 1.0).astype(np.float32)
        from . import proxy
        return InstanceSet("blocks", [Proto("block", _bounds_of(proto_geo), proto_geo)], M, np.zeros(T, np.int32), color,
                           total=T, full_tris=T * proxy.triangle_count(proto_geo))
    return None


# ---------------------------------------------------------------------------------------------------------------- instance points
def points(geo, sources: Optional[Dict[str, object]] = None) -> Optional[InstanceSet]:
    """Instance points (``instance`` + detail ``instance_objects``) as an :class:`InstanceSet`; ``sources`` maps an object name to its geometry."""
    from .geometry import Geometry
    from .nodes._util import normalize_rows, rotation_from_z_to
    from .nodes.copy_to_points import _instance_matrices
    names = geo.detail.get("instance_objects")
    a = geo.point.get("instance")
    if not names or a is None or not geo.npoints:
        return None
    ids = np.asarray(a).reshape(-1).astype(np.int64)
    idx = np.flatnonzero((ids >= 1) & (ids <= len(names)))
    if not len(idx):
        return None
    total = len(idx)
    sampled = total > MAX_SENT
    if sampled:
        idx = idx[np.linspace(0, total - 1, MAX_SENT).astype(np.int64)]
    sub = Geometry.points_only(geo.P[idx])
    for key in ("orient", "rot", "up", "scale", "N"):
        v = geo.point.get(key)
        if v is not None and len(v) == geo.npoints:
            sub.point[key] = v[idx]
    n = len(idx)
    rich = _instance_matrices(sub, True, False)
    if rich is not None:
        R, s = rich
    elif "N" in sub.point:
        R, s = rotation_from_z_to(normalize_rows(sub.point["N"]).astype(np.float64)), np.ones((n, 3))
    else:
        R, s = np.tile(np.eye(3), (n, 1, 1)), np.ones((n, 3))
    M = np.empty((n, 3, 4), np.float32)
    M[:, :, :3] = R * s[:, None, :]
    M[:, :, 3] = geo.P[idx]
    bounds = geo.detail.get("instance_bounds")
    protos, approximate, from_sources = [], False, 0
    for i, name in enumerate(names):
        src = (sources or {}).get(str(name).lstrip("@"))
        if src is not None and getattr(src, "npoints", 0):
            protos.append(Proto(str(name), _bounds_of(src), src))
            from_sources += 1
            continue
        b = bounds[i] if isinstance(bounds, list) and len(bounds) == len(names) else None
        try:
            bb = np.asarray(b, np.float64).reshape(2, 3) if b is not None else None
        except (TypeError, ValueError):
            bb = None
        if bb is None or not np.isfinite(bb).all():
            bb = np.array([[-0.5] * 3, [0.5] * 3])
        protos.append(Proto(str(name), bb, None))
        approximate = True
    color = None
    cd = geo.point.get("Cd")
    if cd is not None and cd.ndim == 2 and cd.shape[1] >= 3 and len(cd) == geo.npoints:
        color = np.clip(cd[idx, :3], 0.0, 1.0).astype(np.float32)
    from . import proxy
    pi = (ids[idx] - 1).astype(np.int32)
    tris = [proxy.triangle_count(p.geo) if p.geo is not None else 12 for p in protos]
    counts = np.bincount(pi, minlength=len(protos))
    return InstanceSet("points", protos, M, pi, color, approximate=approximate, total=total, sampled=sampled,
                       full_tris=int(sum(int(c) * t for c, t in zip(counts, tris)) * (total / max(n, 1))))


def estimate_tris(geo, sources) -> int:
    """Triangles realising every instance point with its source geometry would draw (0 when there are none to realise)."""
    from . import proxy
    names, a = geo.detail.get("instance_objects"), geo.point.get("instance")
    if not names or a is None:
        return 0
    counts = np.bincount(np.clip(np.asarray(a).reshape(-1).astype(np.int64), 0, len(names)), minlength=len(names) + 1)
    return int(sum(int(counts[i]) * proxy.triangle_count(sources[str(nm).lstrip("@")])
                   for i, nm in enumerate(names, 1) if str(nm).lstrip("@") in sources))


# ---------------------------------------------------------------------------------------------------------------- what the viewer gets
def _box(bounds: np.ndarray) -> Tuple[np.ndarray, np.ndarray]:
    lo, hi = bounds
    pts = np.array([[x, y, z] for z in (lo[2], hi[2]) for y in (lo[1], hi[1]) for x in (lo[0], hi[0])], np.float32)       # index = x + 2y + 4z
    tri = np.array([[0, 2, 3], [0, 3, 1], [4, 5, 7], [4, 7, 6], [0, 1, 5], [0, 5, 4], [2, 6, 7], [2, 7, 3], [0, 4, 6], [0, 6, 2], [1, 3, 7], [1, 7, 5]], np.uint32)
    return pts, tri


def _level(geo, cap: int) -> Optional[Tuple[np.ndarray, np.ndarray, int]]:
    """(points, triangles, real triangle count) of ``geo`` reduced to ``cap`` triangles, None when it has no faces."""
    from . import preview, proxy
    full = proxy.triangle_count(geo)
    if not full:
        return None
    shown = geo
    if full > cap:
        shown = proxy.decimate(geo, cap, geo.npoints).geo
    tri, _ = preview._triangles(shown)
    if not len(tri):
        return None
    return np.ascontiguousarray(shown.P, np.float32), tri.astype(np.uint32), full


def pack(iset: InstanceSet, lod0: int = LOD_TRIS[0], lod1: int = LOD_TRIS[1]) -> Tuple[dict, Dict[str, np.ndarray]]:
    """``(header["instances"], arrays)`` for the wire format of :mod:`minihou.preview`: ``ip_P`` / ``ip_tri`` (every level of every shape,
    triangle indices already global), ``inst_M`` (12 floats per instance, row-major 3x4), ``inst_proto``, ``inst_sph`` (world centre and
    radius, for culling and level choice) and ``inst_col`` when the instances have colours."""
    P_all, T_all, meshes, protos_meta = [], [], [], []
    p_off = t_off = 0

    def add(pts, tri):
        nonlocal p_off, t_off
        P_all.append(pts.reshape(-1))
        T_all.append((tri + np.uint32(p_off)).reshape(-1))
        meshes.append({"p_off": p_off, "p_n": int(len(pts)), "t_off": t_off, "t_n": int(len(tri))})
        p_off += len(pts)
        t_off += len(tri)
        return len(meshes) - 1

    radius = np.zeros(len(iset.protos))
    centre = np.zeros((len(iset.protos), 3))
    for k, pr in enumerate(iset.protos):
        lo, hi = pr.bounds
        centre[k] = (lo + hi) / 2
        radius[k] = max(0.5 * float(np.linalg.norm(hi - lo)), 1e-9)
        box = add(*_box(pr.bounds))
        lods = [box, box, box]
        info = {"name": pr.name, "radius": float(radius[k]), "centre": centre[k].tolist(), "full_tris": 0}
        first = _level(pr.geo, lod0) if pr.geo is not None else None
        if first is not None:
            info["full_tris"] = first[2]
            lods[0] = add(first[0], first[1])
            lods[1] = lods[0]
            if len(first[1]) > lod1:
                second = _level(pr.geo, lod1)
                if second is not None:
                    lods[1] = add(second[0], second[1])
        info["lods"] = lods
        info["tris"] = [meshes[m]["t_n"] for m in lods]
        protos_meta.append(info)

    N = len(iset.M)
    M3 = iset.M[:, :, :3].astype(np.float64)
    scale = np.linalg.norm(M3, axis=1).max(axis=1)                          # the longest column: the biggest stretch
    cen = np.einsum("nij,nj->ni", M3, centre[iset.proto]) + iset.M[:, :, 3]
    sph = np.empty((N, 4), np.float32)
    sph[:, :3] = cen
    sph[:, 3] = radius[iset.proto] * scale
    arrays = {"ip_P": np.concatenate(P_all).astype(np.float32), "ip_tri": np.concatenate(T_all).astype(np.uint32),
              "inst_M": np.ascontiguousarray(iset.M, np.float32).reshape(-1), "inst_proto": np.ascontiguousarray(iset.proto, np.uint32),
              "inst_sph": np.ascontiguousarray(sph).reshape(-1)}
    if iset.color is not None:
        arrays["inst_col"] = np.ascontiguousarray(iset.color, np.float32).reshape(-1)
    head = {"kind": iset.kind, "count": int(N), "total": int(iset.total or N), "sampled": bool(iset.sampled), "approximate": bool(iset.approximate),
            "replaces": bool(iset.replaces), "protos": protos_meta, "meshes": meshes, "full_tris": int(iset.full_tris), "color": iset.color is not None,
            "lod_tris": [int(lod0), int(lod1), 12]}
    return head, arrays


def shell(geo) -> Tuple[dict, Dict[str, np.ndarray]]:
    """The preview header and empty arrays of a geometry whose drawing is entirely the instances (counts and bounds are the real ones)."""
    from . import preview, proxy
    n0, tris0 = geo.npoints, proxy.triangle_count(geo)
    lo, hi = (geo.P.min(0).astype(float).tolist(), geo.P.max(0).astype(float).tolist()) if geo.npoints else ([0.0] * 3, [0.0] * 3)
    arrays = {"P": np.zeros(0, np.float32), "tri": np.zeros(0, np.uint32), "tri_prim": np.zeros(0, np.uint32), "edge": np.zeros(0, np.uint32)}
    head = {"points": int(n0), "prims": int(geo.topology.nprims), "tris": 0, "edges": 0, "bounds": [lo, hi], "truncated": False,
            "proxy": {"active": True, "points": int(n0), "tris": int(tris0), "shown_points": 0, "shown_tris": 0, "full": False,
                      "max_tris": preview.MAX_TRIS, "max_points": preview.MAX_POINTS}}
    return head, arrays


def plan(geo, mode: str = "auto", full: bool = False, tri_cap: int = 0, sources: Optional[Dict[str, object]] = None) -> Optional[InstanceSet]:
    """The instances to draw for ``geo`` in a preview, or None for the ordinary drawing. ``mode``: ``never``, ``auto`` or ``always``; a
    ``full`` request (selection and painting need real geometry) never swaps real geometry for blocks; ``tri_cap`` is the view's triangle budget."""
    from . import preview, proxy
    if mode == "never":
        return None
    iset = points(geo, sources)
    if iset is not None:
        return iset
    if full or geo.topology.nprims < MIN_INSTANCES * MIN_BLOCK_PRIMS:
        return None
    cap = int(tri_cap) if tri_cap else preview.INSTANCE_TRIS
    if mode == "auto" and proxy.triangle_count(geo) <= cap and geo.npoints <= preview.INSTANCE_POINTS:
        return None                                                # small enough to draw as it is
    iset = blocks(geo)
    if iset is not None:
        iset = InstanceSet(**{**iset.__dict__, "replaces": True})
    return iset
