"""Proxies for very large scenes: a lighter stand-in for the real thing, so a graph stays interactive.

Two independent layers:

**Display proxy** -- :func:`decimate` turns any geometry into one that fits a budget of triangles and points. Closed polygons
are reduced by vertex clustering (points that fall in the same cell of a 3-D grid merge; the cell size is searched so the result
lands just under the budget), open curves keep every k-th curve, loose points are sampled evenly. The whole shape survives
(the old preview just cut off the first part of the triangle list). Attributes follow: floats average over a cluster, everything
else takes the first member. The editor's viewport and the Blender link both use it; nothing is cooked differently.

**Cook proxy** -- a *level* between 0 and 1 (1 = full) that node parameters scale with. A parameter marked ``proxy="count"``
(scatter count, points per square metre) is multiplied by the level; ``"res2"`` (grid columns and rows, sphere bands) by its
square root, because the work grows with the square; ``"res3"`` by its cube root; the ``"step"`` kinds divide a size by the same
factors (voxel size, spacing, resample length). The scaled values are what the cache key is computed from, so a cook at 10% never
serves a full result and the other way round, and nodes that do not use a proxy parameter keep one key at every level.

The level is a context variable (like the frame in :mod:`timectx`), so systems and loops cooked inside a node see it too.
"""
from __future__ import annotations

import contextlib
import math
from contextvars import ContextVar
from dataclasses import dataclass
from typing import Dict, Iterator, Optional, Tuple

import numpy as np

# -- the cook level -----------------------------------------------------------------------------------------------------
# kind -> exponent of the level the parameter is multiplied by (negative: a size, which grows as the level drops)
KINDS: Dict[str, float] = {"count": 1.0, "res2": 0.5, "res3": 1.0 / 3.0, "step1": -1.0, "step2": -0.5, "step3": -1.0 / 3.0}
LEVELS = (1.0, 0.5, 0.25, 0.1, 0.05, 0.01)       # the presets the editor offers; any value in [MIN_LEVEL, 1] works
MIN_LEVEL = 0.001
AUTO_LEVEL = 0.1                                 # what "auto" cooks first when the full cook is slow
AUTO_SLOW_MS = 800.0                             # a full cook slower than this makes auto mode cook the proxy first
# nodes that cook a sub-graph at run time: their result depends on the level even if none of their own parameters do
NESTED = ("for_each", "solver")

_LEVEL: ContextVar[float] = ContextVar("minihou_proxy", default=1.0)


def level() -> float:
    return _LEVEL.get()


@contextlib.contextmanager
def using(lvl: Optional[float]) -> Iterator[None]:
    """Cook at ``lvl`` inside the block (None keeps the surrounding level)."""
    if lvl is None:
        yield
        return
    token = _LEVEL.set(clamp(lvl))
    try:
        yield
    finally:
        _LEVEL.reset(token)


def clamp(lvl) -> float:
    lvl = float(lvl)
    if not math.isfinite(lvl):
        raise ValueError("proxy level must be a finite number")
    return min(1.0, max(MIN_LEVEL, lvl))


def scale_value(param, value, lvl: float):
    """``value`` of ``param`` (which has a ``proxy`` kind) at proxy level ``lvl``."""
    if lvl >= 1.0 or not param.proxy:
        return value
    f = lvl ** KINDS[param.proxy]
    if param.kind == "int":
        if value <= 0:
            return value
        v = int(round(value * f))
        v = max(v, 1, int(param.min) if param.min is not None else 1)
        if f <= 1.0:
            v = min(v, value)                                    # a proxy never asks for more than the real value
        return v
    v = float(value) * f
    if param.max is not None:
        v = min(v, float(param.max))
    if param.min is not None and f < 1.0:
        v = max(v, min(float(param.min), float(value)))
    return v


def scale_params(nt, params: dict, lvl: Optional[float] = None) -> dict:
    """``params`` (already coerced) for ``nt`` at the proxy level (default: the current one). The same dict when nothing scales."""
    lvl = level() if lvl is None else lvl
    if lvl >= 1.0 or not nt.proxy_params:
        return params
    out = dict(params)
    for p in nt.proxy_params:
        out[p.name] = scale_value(p, out[p.name], lvl)
    return out


def key_part(nt) -> Optional[float]:
    """Extra cache-key material for node type ``nt`` at the current level, or None when the node's own (scaled) parameters already
    say everything: only sub-graph nodes (systems, for-each, solver) need the level itself."""
    lvl = level()
    if lvl >= 1.0:
        return None
    if nt.category == "system" or nt.name in NESTED:
        return round(lvl, 6)
    return None


# -- the display proxy --------------------------------------------------------------------------------------------------
@dataclass
class Reduced:
    """A reduced stand-in for a geometry and how it maps back."""
    geo: object                      # the Geometry (the same object when nothing needed reducing)
    point_src: np.ndarray            # for each new point: the original point it stands for
    prim_src: np.ndarray             # for each new primitive: the original primitive
    reduced: bool
    points_in: int
    tris_in: int


def fan_triangles(geo, mask: Optional[np.ndarray] = None) -> Tuple[np.ndarray, np.ndarray]:
    """Fan triangles for the closed polygons ``mask`` selects (default: all closed ones with 3+ corners): corner (vertex) indices
    ``(T, 3)`` int32 and the owning primitive ``(T,)`` int32. Exact for convex polygons, fast for any."""
    t = geo.topology
    offs = t.prim_offsets.astype(np.int64)
    sizes = np.diff(offs)
    sel = t.closed_mask() & (sizes >= 3)
    if mask is not None:
        sel &= mask
    counts = np.where(sel, sizes - 2, 0)
    total = int(counts.sum())
    if total == 0:
        return np.zeros((0, 3), np.int32), np.zeros(0, np.int32)
    first = np.repeat(offs[:-1].astype(np.int32), counts)
    k = np.arange(total, dtype=np.int32) - np.repeat((np.cumsum(counts) - counts).astype(np.int32), counts)
    tri = np.empty((total, 3), np.int32)
    tri[:, 0] = first
    k += first
    tri[:, 1] = k + 1
    tri[:, 2] = k + 2
    owner = np.repeat(np.arange(t.nprims, dtype=np.int32), counts)
    return tri, owner


def draw_stats(t, npoints: int) -> Tuple[int, int, int]:
    """``(fan triangles of the closed polygons, vertices of the open curves, points no primitive uses)`` of a topology. Cached on
    the topology once its arrays are frozen (cooked geometry), so a deformer downstream, which shares its input's topology, gets
    them for free."""
    hit = getattr(t, "_draw_stats", None)
    if hit is not None and hit[0] == npoints:
        return hit[1:]
    from . import rust
    if rust.enabled():
        ntri, open_verts, loose = rust.draw_stats(t, npoints)
    else:
        sizes = np.diff(t.prim_offsets)
        closed = t.closed_mask()
        ntri = int(np.maximum(sizes - 2, 0)[closed].sum(dtype=np.int64)) if len(sizes) else 0
        open_verts = int(sizes[~closed & (sizes >= 2)].sum(dtype=np.int64)) if len(sizes) else 0
        used = np.zeros(npoints, bool)
        if t.nvertices:
            used[t.vertex_point] = True
        loose = npoints - int(np.count_nonzero(used))
    if not t.vertex_point.flags.writeable:
        t._draw_stats = (npoints, ntri, open_verts, loose)
    return ntri, open_verts, loose


def triangle_count(geo) -> int:
    t = geo.topology
    if not t.nprims:
        return 0
    return draw_stats(t, geo.npoints)[0]


def _cells(P: np.ndarray, lo: np.ndarray, inv_size: float, res: int) -> np.ndarray:
    """Cell number (one int per point) of float32 positions on a ``res``^3 grid starting at ``lo``."""
    c = ((P - lo) * np.float32(inv_size)).astype(np.int32)
    np.minimum(c, res - 1, out=c)
    ids = c[:, 0].astype(np.int64)
    ids *= res
    ids += c[:, 1]
    ids *= res
    ids += c[:, 2]
    return ids


def _occupied(ids: np.ndarray) -> int:
    s = np.sort(ids)
    return 1 + int((s[1:] != s[:-1]).sum()) if len(s) else 0


def _label(ids: np.ndarray) -> Tuple[np.ndarray, int]:
    """(cluster number per point, clusters): equal ids share a number."""
    order = np.argsort(ids)
    s = ids[order]
    flag = np.empty(len(s), np.int64)
    flag[0] = 0
    np.not_equal(s[1:], s[:-1], out=flag[1:])
    np.cumsum(flag, out=flag)
    inv = np.empty(len(s), np.int64)
    inv[order] = flag
    return inv, int(flag[-1]) + 1


def _cluster(P: np.ndarray, tri_local: np.ndarray, max_tris: int) -> Tuple[np.ndarray, int]:
    """Cluster the float32 points ``P`` on a grid so that the triangles that stay non-degenerate are just under ``max_tris``.
    Returns (cluster number per point, clusters). The grid size is searched on a sample of the points (a sort per try), the answer
    is then checked on all of them."""
    n = len(P)
    lo, hi = P.min(0), P.max(0)
    extent = float((hi - lo).max())
    if extent <= 0.0:
        return np.zeros(n, np.int64), 1
    ratio = len(tri_local) / max(n, 1)                             # triangles per point: 2 for a closed or sheet-like mesh
    sample = P if n <= 1_000_000 else P[np.random.default_rng(7).integers(0, n, 1_000_000)]
    res = max(4, int(math.sqrt(max_tris / max(ratio, 1.0)) * 1.2))
    for _ in range(8):                                             # search: occupied cells on the sample ~ triangles / ratio
        occ = _occupied(_cells(sample, lo, res / extent, res))
        est = occ * ratio
        if max_tris * 0.7 <= est <= max_tris * 0.95:
            break
        res = max(2, int(res * math.sqrt(max_tris * 0.85 / max(est, 1.0)))) if est > 0 else res * 2
    for _ in range(6):                                             # check on everything; tighten if the sample undercounted
        inv, nclust = _label(_cells(P, lo, res / extent, res))
        ta, tb, tc = inv[tri_local[:, 0]], inv[tri_local[:, 1]], inv[tri_local[:, 2]]
        kept = int(((ta != tb) & (tb != tc) & (ta != tc)).sum())
        if kept <= max_tris or res <= 2:
            break
        res = max(2, min(res - 1, int(res * math.sqrt(max_tris * 0.9 / kept))))
    return inv, nclust


def _take(a: np.ndarray, idx: np.ndarray) -> np.ndarray:
    return np.ascontiguousarray(a[idx])


def _cluster_mean(a: np.ndarray, inv: np.ndarray, count: np.ndarray, first: np.ndarray, name: str) -> np.ndarray:
    """Per-cluster value of point attribute ``a``: the mean for floats, the first member's value for anything else."""
    if a.dtype.kind != "f":
        return _take(a, first)
    flat = a.reshape(len(a), -1).astype(np.float64)
    out = np.empty((len(count), flat.shape[1]), np.float64)
    for c in range(flat.shape[1]):
        out[:, c] = np.bincount(inv, weights=flat[:, c], minlength=len(count)) / count
    if name == "N" and out.shape[1] == 3:
        ln = np.linalg.norm(out, axis=1, keepdims=True)
        out = out / np.where(ln > 1e-12, ln, 1.0)
    return np.ascontiguousarray(out.reshape((len(count),) + a.shape[1:]), a.dtype)


def decimate(geo, max_tris: int, max_points: int) -> Reduced:
    """``geo`` reduced to at most ``max_tris`` triangles and about ``max_points`` loose / curve points (see the module docs)."""
    from .geometry import AttrTable, Geometry, Topology
    t = geo.topology
    n = geo.npoints
    ntri, open_verts, nloose = draw_stats(t, n) if t.nprims else (0, 0, n)
    if ntri <= max_tris and open_verts + nloose <= max_points:
        return Reduced(geo, np.arange(n), np.arange(t.nprims), False, n, ntri)
    vp = t.vertex_point.astype(np.int64)
    offs = t.prim_offsets.astype(np.int64)
    sizes = np.diff(offs)
    closed = t.closed_mask()
    open_prims = np.flatnonzero(~closed & (sizes >= 2))
    used = np.zeros(n, bool)
    if len(vp):
        used[vp] = True
    loose = np.flatnonzero(~used)

    # -- the mesh part ----------------------------------------------------------------------------------------------
    mesh = (closed & (sizes >= 3))
    tri_v, tri_p = fan_triangles(geo)
    tri_pt = t.vertex_point[tri_v] if len(tri_v) else np.zeros((0, 3), np.int32)
    mark = np.zeros(n, bool)
    if len(tri_pt):
        mark[tri_pt.reshape(-1)] = True
    mpts = np.flatnonzero(mark)
    new_of_old = np.full(n, -1, np.int64)
    if len(mpts):
        local = np.zeros(n, np.int32)
        local[mpts] = np.arange(len(mpts), dtype=np.int32)
        tri_local = local[tri_pt]
        P = geo.P[mpts]
        if ntri > max_tris:
            inv, nclust = _cluster(P, tri_local, max_tris)
        else:
            inv, nclust = np.arange(len(mpts), dtype=np.int64), len(mpts)
        new_of_old[mpts] = inv
        count = np.bincount(inv, minlength=nclust).astype(np.float64)
        first = np.empty(nclust, np.int64)
        first[inv[::-1]] = np.arange(len(inv), dtype=np.int64)[::-1]           # the lowest index wins: the first member
        first_old = mpts[first]
        ta, tb, tc = inv[tri_local[:, 0]], inv[tri_local[:, 1]], inv[tri_local[:, 2]]
        keep = (ta != tb) & (tb != tc) & (ta != tc)
        new_tri = np.empty((int(keep.sum()), 3), np.int32)
        new_tri[:, 0], new_tri[:, 1], new_tri[:, 2] = ta[keep], tb[keep], tc[keep]
        tri_v_kept = tri_v[keep]
        tri_p_kept = tri_p[keep]
    else:
        inv, nclust = np.zeros(0, np.int64), 0
        count, first, first_old = np.zeros(0), np.zeros(0, np.int64), np.zeros(0, np.int64)
        new_tri = np.zeros((0, 3), np.int64)
        tri_v_kept, tri_p_kept = np.zeros((0, 3), np.int32), np.zeros(0, np.int32)

    # -- curves and loose points ------------------------------------------------------------------------------------
    keep_open = open_prims
    if open_verts > max_points and len(open_prims):
        step = int(math.ceil(open_verts / max(max_points, 1)))
        keep_open = open_prims[::step]
    budget_left = max(max_points - int(sizes[keep_open].sum() if len(keep_open) else 0), 0)
    keep_loose = loose
    if len(loose) > budget_left:
        keep_loose = loose[np.linspace(0, len(loose) - 1, budget_left).astype(np.int64)] if budget_left else loose[:0]
    open_vertex_ids = (np.concatenate([np.arange(offs[p], offs[p + 1]) for p in keep_open])
                       if len(keep_open) and len(keep_open) < 20000 else
                       (np.flatnonzero(np.repeat(np.isin(np.arange(t.nprims), keep_open), sizes)) if len(keep_open) else np.zeros(0, np.int64)))
    open_old_pts = vp[open_vertex_ids] if len(open_vertex_ids) else np.zeros(0, np.int64)
    extra_old = np.unique(np.concatenate([open_old_pts, keep_loose]))
    extra_old = extra_old[new_of_old[extra_old] < 0]                # points shared with the mesh already have a cluster
    new_of_old[extra_old] = nclust + np.arange(len(extra_old))
    npts = nclust + len(extra_old)

    # -- the new geometry -------------------------------------------------------------------------------------------
    src_old = np.concatenate([first_old, extra_old]).astype(np.int64)
    out = Geometry(npts)
    for name, a in geo.point.items():
        head = _cluster_mean(a[mpts], inv, count, first, name) if nclust else a[:0]
        out.point[name] = np.concatenate([head, a[extra_old]]) if len(extra_old) or nclust else a[:0]
    new_vp_open = new_of_old[open_old_pts] if len(open_old_pts) else np.zeros(0, np.int64)
    sizes_open = sizes[keep_open] if len(keep_open) else np.zeros(0, np.int64)
    ntri_out = len(new_tri)
    vertex_point = np.concatenate([new_tri.reshape(-1), new_vp_open]).astype(np.int32)
    prim_offsets = np.concatenate([[0], np.cumsum(np.concatenate([np.full(ntri_out, 3, np.int64), sizes_open]))]).astype(np.int32)
    closed_out = np.concatenate([np.ones(ntri_out, bool), np.zeros(len(sizes_open), bool)])
    out.topology = Topology(vertex_point, prim_offsets, closed_out)
    out.vertex = AttrTable(out.topology.nvertices)
    out.prim = AttrTable(out.topology.nprims)
    prim_src = np.concatenate([tri_p_kept, keep_open]).astype(np.int64)
    vert_src = np.concatenate([tri_v_kept.reshape(-1), open_vertex_ids]).astype(np.int64)
    for name, a in geo.prim.items():
        out.prim[name] = _take(a, prim_src)
    for name, a in geo.vertex.items():
        out.vertex[name] = _take(a, vert_src)
    out.detail = dict(geo.detail)
    out.freeze()
    return Reduced(out, src_old, prim_src, True, n, ntri)
