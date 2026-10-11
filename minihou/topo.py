"""Vectorised topology queries shared by nodes: per-vertex primitive, half-edges, unique edges, centroids.

All functions take a :class:`~minihou.geometry.Geometry` and return numpy arrays (int64 indices). A half-edge is the
directed edge from a vertex's point to the next vertex's point in the same primitive (cyclic).
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


class Edges(NamedTuple):
    points: np.ndarray        # (E, 2) point indices, sorted within each edge
    of_vertex: np.ndarray     # (V,) edge id of each half-edge (vertex -> next vertex)
    count: np.ndarray         # (E,) how many half-edges use the edge (1 = boundary, 2 = manifold interior)


def edges(geo) -> Edges:
    """Unique undirected edges of all primitives."""
    vp = geo.topology.vertex_point.astype(np.int64)
    a, b = vp, vp[next_vertex(geo)]
    lo, hi = np.minimum(a, b), np.maximum(a, b)
    key = lo * max(geo.npoints, 1) + hi
    uniq, inv, count = np.unique(key, return_inverse=True, return_counts=True)
    n = max(geo.npoints, 1)
    return Edges(np.stack([uniq // n, uniq % n], 1), inv.reshape(-1), count)


def prim_centroids(geo) -> np.ndarray:
    """Mean of each primitive's points (float64, (prims, 3)); empty primitives give 0."""
    t = geo.topology
    pv = prim_of_vertex(geo)
    P = geo.P.astype(np.float64)[t.vertex_point]
    sizes = np.maximum(t.prim_sizes(), 1).astype(np.float64)
    return np.stack([np.bincount(pv, weights=P[:, c], minlength=t.nprims) for c in range(3)], 1) / sizes[:, None]


def segment_sum(values: np.ndarray, index: np.ndarray, n: int) -> np.ndarray:
    """Sum rows of ``values`` (1-D or 2-D) into ``n`` buckets by ``index`` (float64)."""
    v = values.reshape(len(values), -1).astype(np.float64)
    out = np.stack([np.bincount(index, weights=v[:, c], minlength=n) for c in range(v.shape[1])], 1)
    return out[:, 0] if values.ndim == 1 else out


def boundary_points(geo) -> np.ndarray:
    """Bool per point: on an edge used by exactly one primitive."""
    e = edges(geo)
    m = np.zeros(geo.npoints, bool)
    m[e.points[e.count == 1].ravel()] = True
    return m


def boundary_prims(geo) -> np.ndarray:
    """Bool per primitive: has an edge no other primitive shares."""
    e = edges(geo)
    on = e.count[e.of_vertex] == 1
    return np.bincount(prim_of_vertex(geo), weights=on, minlength=geo.nprims) > 0
