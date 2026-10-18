"""Blender curves, point clouds and meshes as engine arrays, and several of them as one geometry (bpy-free, so it is unit-tested).

Everything is keyed like ``geo`` messages: ``point/P``, ``topo/vertex_point`` (corner -> point), ``topo/prim_offsets``,
``topo/prim_closed`` (1 polygon, 0 open curve; absent: all polygons) and ``<domain>/<name>`` attributes, with the domains
``point``, ``vertex`` (corners), ``edge`` and ``prim``. Edge rows use ``topo/edge_pairs``.
"""
from __future__ import annotations

from typing import Dict, List, Optional, Sequence, Tuple

import numpy as np

from . import curve_eval
from .protocol import TOPO_EDGE_PAIRS, TOPO_KEYS, TOPO_PRIM_CLOSED, TOPO_PRIM_OFFSETS, TOPO_VERTEX_POINT, attr_key, split_key

Arrays = Dict[str, np.ndarray]


def _carry(sampling: "curve_eval.Sampling", arr: np.ndarray) -> np.ndarray:
    """A per-control-point attribute on the evaluated points (integers and flags take the nearest control point's value)."""
    a = np.asarray(arr)
    out = sampling.blend(a.astype(np.float32) if a.dtype.kind in "biu" else a)
    if a.dtype.kind in "biu":
        out = np.rint(out).astype(a.dtype if a.dtype.kind != "b" else np.int32)
    return out


def curve_arrays(position: np.ndarray, offsets: np.ndarray, *, curve_type=None, cyclic=None, resolution=None, handle_left=None,
                 handle_right=None, nurbs_weight=None, nurbs_order=None, knots_mode=None, point_attrs: Optional[Arrays] = None,
                 curve_attrs: Optional[Arrays] = None, control: bool = False) -> Tuple[Arrays, dict]:
    """Curves as open (or, for cyclic curves, closed) primitives. Each curve is sampled like Blender draws it (``control``: its control
    points instead). ``point_attrs`` ride along per control point (``radius``, ``tilt``, ...), ``curve_attrs`` become primitive
    attributes. Returns ``(arrays, info)``."""
    position = np.asarray(position, np.float32).reshape(-1, 3)
    offsets = np.asarray(offsets, np.int64)
    if control:
        S = curve_eval.control_polygon(position, offsets, np.zeros(len(offsets) - 1, bool) if cyclic is None else cyclic)
    else:
        S = curve_eval.evaluate(position, offsets, curve_type, cyclic, resolution, handle_left, handle_right, nurbs_weight,
                                nurbs_order, knots_mode)
    counts = S.counts
    keep = counts >= 2                                      # a curve of one point is just a point
    starts = S.offsets[:-1]
    arrays: Arrays = {attr_key("point", "P"): np.ascontiguousarray(S.P)}
    sizes = counts[keep]
    if len(sizes):
        first = np.repeat(starts[keep], sizes)
        local = np.arange(int(sizes.sum()), dtype=np.int64) - np.repeat(np.cumsum(sizes) - sizes, sizes)
        arrays[TOPO_VERTEX_POINT] = (first + local).astype(np.int32)
    else:
        arrays[TOPO_VERTEX_POINT] = np.zeros(0, np.int32)
    arrays[TOPO_PRIM_OFFSETS] = np.concatenate([[0], np.cumsum(sizes)]).astype(np.int32)
    arrays[TOPO_PRIM_CLOSED] = (S.closed[keep] & (sizes >= 3)).astype(np.uint8)
    for name, a in (point_attrs or {}).items():
        arrays[attr_key("point", name)] = _carry(S, a)
    for name, a in (curve_attrs or {}).items():
        arrays[attr_key("prim", name)] = np.ascontiguousarray(np.asarray(a)[keep])
    info = {"curves": int(len(counts)), "cyclic": int(S.closed.sum()), "points": int(len(S.P))}
    return arrays, info


def point_arrays(position: np.ndarray, point_attrs: Optional[Arrays] = None) -> Arrays:
    """A point cloud: points with attributes and no primitives."""
    position = np.ascontiguousarray(np.asarray(position, np.float32).reshape(-1, 3))
    arrays: Arrays = {attr_key("point", "P"): position, TOPO_VERTEX_POINT: np.zeros(0, np.int32),
                      TOPO_PRIM_OFFSETS: np.zeros(1, np.int32), TOPO_PRIM_CLOSED: np.zeros(0, np.uint8)}
    for name, a in (point_attrs or {}).items():
        arrays[attr_key("point", name)] = np.ascontiguousarray(a)
    return arrays


def _sizes(part: Arrays) -> Tuple[int, int, int]:
    npts = len(part[attr_key("point", "P")])
    nv = len(part.get(TOPO_VERTEX_POINT, ()))
    offs = part.get(TOPO_PRIM_OFFSETS)
    return npts, nv, (len(offs) - 1 if offs is not None and len(offs) else 0)


def combine(parts: Sequence[Arrays]) -> Arrays:
    """One geometry from several (a mesh, curves and points of one object). Point numbers are shifted so the parts stay separate;
    attributes a part lacks are zero there."""
    parts = [p for p in parts if p]
    if len(parts) == 1:
        return parts[0]
    sizes = [_sizes(p) for p in parts]
    pbase = np.concatenate([[0], np.cumsum([s[0] for s in sizes])])
    vbase = np.concatenate([[0], np.cumsum([s[1] for s in sizes])])
    out: Arrays = {}
    out[TOPO_VERTEX_POINT] = np.concatenate([p.get(TOPO_VERTEX_POINT, np.zeros(0, np.int32)).astype(np.int64) + pbase[i]
                                            for i, p in enumerate(parts)]).astype(np.int32)
    out[TOPO_PRIM_OFFSETS] = np.concatenate([[0]] + [
        (p.get(TOPO_PRIM_OFFSETS, np.zeros(1, np.int32))[1:].astype(np.int64) + vbase[i]) for i, p in enumerate(parts)]).astype(np.int32)
    if any(TOPO_PRIM_CLOSED in p for p in parts):
        out[TOPO_PRIM_CLOSED] = np.concatenate([
            p[TOPO_PRIM_CLOSED] if TOPO_PRIM_CLOSED in p else np.ones(s[2], np.uint8) for p, s in zip(parts, sizes)]).astype(np.uint8)
    if any(TOPO_EDGE_PAIRS in p for p in parts):
        out[TOPO_EDGE_PAIRS] = np.concatenate([
            p.get(TOPO_EDGE_PAIRS, np.zeros((0, 2), np.int32)) + pbase[i] for i, p in enumerate(parts)]).astype(np.int32)
    names: Dict[str, None] = {}
    for p in parts:
        for key in p:
            if key not in TOPO_KEYS:
                names[key] = None
    for key in names:
        dom, _ = split_key(key)
        n_of = {"point": 0, "vertex": 1, "prim": 2}.get(dom)
        ref = next(p[key] for p in parts if key in p)
        chunks = []
        for p, s in zip(parts, sizes):
            if key in p:
                chunks.append(p[key])
            else:
                count = len(p.get(TOPO_EDGE_PAIRS, ())) if dom == "edge" else s[n_of]
                chunks.append(np.zeros((count,) + ref.shape[1:], ref.dtype))
        out[key] = np.concatenate(chunks)
    return out


def check(arrays: Arrays) -> None:
    """Raise ValueError when the arrays do not describe one consistent geometry (a guard for the combiner and for tests)."""
    npts, nv, nprim = _sizes(arrays)
    if nv and (arrays[TOPO_VERTEX_POINT].max() >= npts or arrays[TOPO_VERTEX_POINT].min() < 0):
        raise ValueError("a corner refers to a point that does not exist")
    offs = arrays.get(TOPO_PRIM_OFFSETS)
    if offs is not None and len(offs) and int(offs[-1]) != nv:
        raise ValueError("primitive offsets do not end at the corner count")
    for key, a in arrays.items():
        if key in TOPO_KEYS:
            continue
        dom, _ = split_key(key)
        want = len(arrays.get(TOPO_EDGE_PAIRS, ())) if dom == "edge" else {"point": npts, "vertex": nv, "prim": nprim}[dom]
        if len(a) != want:
            raise ValueError(f"{key} has {len(a)} rows, expected {want}")
