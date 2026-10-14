"""Vectorised topology queries shared by nodes: per-vertex primitive, half-edges, unique edges, centroids.

All functions take a :class:`~minihou.geometry.Geometry` and return numpy arrays (int64 indices). A half-edge is the
directed edge from a vertex's point to the next vertex's point in the same primitive (cyclic for closed polygons; the
last vertex of an open curve has no half-edge, see :func:`has_next`).
"""
from __future__ import annotations

from typing import NamedTuple

import numpy as np


def prim_of_vertex(geo) -> np.ndarray:
    t = geo.topology
    return np.repeat(np.arange(t.nprims, dtype=np.int64), t.prim_sizes())


def next_vertex(geo) -> np.ndarray:
    """Index of the next vertex in the same primitive (the last wraps to the first)."""
    t = geo.topology
    offs = t.prim_offsets.astype(np.int64)
    nxt = np.arange(1, t.nvertices + 1, dtype=np.int64)
    ends = offs[1:] - 1
    sizes = np.diff(offs)
    ok = sizes > 0
    nxt[ends[ok]] = offs[:-1][ok]
    return nxt


def prev_vertex(geo) -> np.ndarray:
    t = geo.topology
    offs = t.prim_offsets.astype(np.int64)
    prv = np.arange(-1, t.nvertices - 1, dtype=np.int64)
    sizes = np.diff(offs)
    ok = sizes > 0
    prv[offs[:-1][ok]] = offs[1:][ok] - 1
    return prv


def has_next(geo) -> np.ndarray:
    """Bool per vertex: it starts a half-edge (False only for the last vertex of an open curve)."""
    t = geo.topology
    m = np.ones(t.nvertices, bool)
    if t.has_open:
        offs = t.prim_offsets.astype(np.int64)
        open_ne = (~t.closed) & (np.diff(offs) > 0)
        m[offs[1:][open_ne] - 1] = False
    return m


_NUMBA_EDGES = 20_000          # from this many vertices on, the numba kernels (same result, linear time) replace np.unique


def _edges_numba(geo):
    """:class:`Edges` exactly as the np.unique path gives them, or None without numba."""
    from . import accel
    k = accel.kernels()
    if k is None:
        return None
    t = geo.topology
    n = max(geo.npoints, 1)
    nv = t.nvertices
    vp = np.ascontiguousarray(t.vertex_point, np.int64)
    offs = np.ascontiguousarray(t.prim_offsets, np.int64)
    closed = np.ascontiguousarray(t.closed_mask(), np.uint8)
    lo, hi = np.empty(nv, np.int64), np.empty(nv, np.int64)
    if accel.call(k.edges_halves, vp, offs, closed, lo, hi) is accel.FAILED:
        return None
    r = accel.call(k.edges_bucket, lo, hi, n)
    if r is accel.FAILED:
        return None
    start, shi, sidx = r
    per = np.empty(n, np.int64)
    if accel.call(k.edges_sort_count, start, shi, sidx, n, per) is accel.FAILED:
        return None
    base = np.zeros(n, np.int64)
    np.cumsum(per[:-1], out=base[1:])
    ne = int(base[-1] + per[-1])
    pts, count, of_vertex = np.empty((ne, 2), np.int64), np.empty(ne, np.int64), np.full(nv, -1, np.int64)
    if accel.call(k.edges_number, start, shi, sidx, n, base, pts, count, of_vertex) is accel.FAILED:
        return None
    return Edges(pts, of_vertex, count)


class Edges(NamedTuple):
    points: np.ndarray        # (E, 2) point indices, sorted within each edge
    of_vertex: np.ndarray     # (V,) edge id of each half-edge (vertex -> next vertex); -1 for the end of an open curve
    count: np.ndarray         # (E,) how many half-edges use the edge (1 = boundary, 2 = manifold interior)


def edges(geo) -> Edges:
    """Unique undirected edges of all primitives (open curves contribute their segments, not a closing edge)."""
    from . import rust
    if rust.enabled() and geo.npoints:
        return rust.edges(geo)
    if geo.topology.nvertices >= _NUMBA_EDGES and geo.npoints:
        fast = _edges_numba(geo)
        if fast is not None:
            return fast
    vp = geo.topology.vertex_point.astype(np.int64)
    a, b = vp, vp[next_vertex(geo)]
    real = has_next(geo)
    lo, hi = np.minimum(a, b)[real], np.maximum(a, b)[real]
    n = max(geo.npoints, 1)
    key = lo * n + hi
    uniq, inv, count = np.unique(key, return_inverse=True, return_counts=True)
    of_vertex = np.full(len(vp), -1, np.int64)
    of_vertex[real] = inv.reshape(-1)
    return Edges(np.stack([uniq // n, uniq % n], 1), of_vertex, count)


def prim_centroids(geo) -> np.ndarray:
    """Mean of each primitive's points (float64, (prims, 3)); empty primitives give 0."""
    t = geo.topology
    pv = prim_of_vertex(geo)
    P = geo.P.astype(np.float64)[t.vertex_point]
    sizes = np.maximum(t.prim_sizes(), 1).astype(np.float64)
    return np.stack([np.bincount(pv, weights=P[:, c], minlength=t.nprims) for c in range(3)], 1) / sizes[:, None]


def segment_sum(values: np.ndarray, index: np.ndarray, n: int) -> np.ndarray:
    """Sum rows of ``values`` (1-D or 2-D) into ``n`` buckets by ``index`` (float64)."""
    v = values.reshape(len(values), -1 if len(values) else (values.shape[1] if values.ndim == 2 else 1))
    v = v.astype(np.float64)
    out = np.stack([np.bincount(index, weights=v[:, c], minlength=n) for c in range(v.shape[1])], 1)
    return out[:, 0] if values.ndim == 1 else out


def boundary_points(geo) -> np.ndarray:
    """Bool per point: on a polygon edge used by exactly one polygon (curve segments are not boundaries)."""
    e = edges(geo)
    on_poly = (e.of_vertex >= 0) & geo.topology.closed_mask()[prim_of_vertex(geo)]
    cnt = np.bincount(e.of_vertex[on_poly], minlength=len(e.points))
    m = np.zeros(geo.npoints, bool)
    m[e.points[cnt == 1].ravel()] = True
    return m


def boundary_prims(geo) -> np.ndarray:
    """Bool per primitive: has an edge no other primitive shares."""
    e = edges(geo)
    on = (e.of_vertex >= 0) & (e.count[np.maximum(e.of_vertex, 0)] == 1)
    return np.bincount(prim_of_vertex(geo), weights=on, minlength=geo.nprims) > 0
