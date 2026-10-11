"""Spatial queries: nearest neighbours and connected components.

``nearest`` uses SciPy's cKDTree when SciPy is installed (optional, ``pip install scipy``) and otherwise a numpy
uniform-grid search with the same results (exact nearest neighbour, ties broken by lower index). Both paths are tested
against brute force. ``components`` labels connected pieces with vectorised union-find (no SciPy needed).
"""
from __future__ import annotations

import os
from typing import Tuple

import numpy as np

try:
    if os.environ.get("MINIHOU_NO_SCIPY"):
        raise ImportError("disabled by MINIHOU_NO_SCIPY")
    from scipy.spatial import cKDTree as _KDTree
except Exception:  # noqa: BLE001 - optional
    _KDTree = None

CHUNK = 16384


def backend() -> str:
    return "scipy" if _KDTree is not None else "numpy grid"


def nearest(src: np.ndarray, query: np.ndarray, use_scipy: bool = True) -> Tuple[np.ndarray, np.ndarray]:
    """For each query point, the index of the nearest source point and the distance (float64). Empty source: -1/inf."""
    src = np.asarray(src, np.float64).reshape(-1, 3)
    query = np.asarray(query, np.float64).reshape(-1, 3)
    if len(src) == 0:
        return np.full(len(query), -1, np.int64), np.full(len(query), np.inf)
    if len(query) == 0:
        return np.zeros(0, np.int64), np.zeros(0)
    if _KDTree is not None and use_scipy:
        d, i = _KDTree(src).query(query, k=1)
        return np.asarray(i, np.int64), np.asarray(d, np.float64)
    return _grid_nearest(src, query)


def _cell_sizes(src) -> np.ndarray:
    """Per-axis cell size: about two source points per cell in the dimensions the points actually span (a surface
    gets a single layer of cells across its thickness, a line a single row)."""
    ext = np.maximum(src.max(0) - src.min(0), 0.0)
    big = max(float(ext.max()), 1e-9)
    spans = ext >= 0.05 * big
    k = int(spans.sum())
    if k == 0 or float(ext.max()) <= 0:
        return np.ones(3)                                                     # all points coincide
    h = max((float(np.prod(ext[spans])) * 2.0 / len(src)) ** (1.0 / k), big * 1e-6)
    return np.where(spans, h, np.maximum(ext * 1.0001, 1e-9) + h * 0)     # flat axes: one cell


def _grid_nearest(src, query):
    hv = _cell_sizes(src)
    lo, hi = src.min(0), src.max(0)
    dims = np.minimum(np.floor((hi - lo) / hv).astype(np.int64) + 1, 1 << 20)
    cell = lambda p: np.clip(np.floor((p - lo) / hv).astype(np.int64), 0, dims - 1)  # noqa: E731
    key = lambda c: (c[..., 0] * dims[1] + c[..., 1]) * dims[2] + c[..., 2]  # noqa: E731
    skey = key(cell(src))
    order = np.argsort(skey, kind="stable")
    skey_sorted = skey[order]
    ncells = int(np.prod(dims))
    if ncells <= 8 * len(src) + 1_000_000:                                  # dense table: O(1) cell lookups
        cstart = np.searchsorted(skey_sorted, np.arange(ncells), "left")
        ccount = np.bincount(skey, minlength=ncells)
        lookup = lambda k: (cstart[k], ccount[k])  # noqa: E731
    else:
        lookup = lambda k: (np.searchsorted(skey_sorted, k, "left"),  # noqa: E731
                            np.searchsorted(skey_sorted, k, "right") - np.searchsorted(skey_sorted, k, "left"))
    best_i = np.full(len(query), -1, np.int64)
    best_d = np.full(len(query), np.inf)
    qc_all = cell(query)
    o_all = np.maximum(np.maximum(lo - query, query - hi), 0.0)            # how far outside the grid, per axis
    active = dims > 1                                                       # axes along which cells can differ
    for start in range(0, len(query), CHUNK):
        sl = slice(start, start + CHUNK)
        q, qc, bi, bd, oq = query[sl], qc_all[sl], best_i[sl], best_d[sl], o_all[sl]
        todo = np.arange(len(q))
        r = 0
        while len(todo):
            offs = _shell(r, active)
            c = qc[todo][:, None, :] + offs[None, :, :]                  # (Q, ring, 3): a whole ring at once
            ok = np.all((c >= 0) & (c < dims), axis=2)
            box_lo = lo + c * hv                                         # skip cells farther than the best so far
            gap = np.maximum(np.maximum(box_lo - q[todo][:, None, :], q[todo][:, None, :] - box_lo - hv), 0)
            ok &= (gap ** 2).sum(2) < bd[todo][:, None] ** 2
            qi = np.broadcast_to(todo[:, None], ok.shape)[ok]
            if len(qi):
                a, n = lookup(key(c[ok]))
                has = n > 0
                qi, a, n = qi[has], a[has], n[has]
                if len(qi):
                    rep_q = np.repeat(qi, n)
                    local = np.arange(n.sum()) - np.repeat(np.cumsum(n) - n, n)
                    cand = order[np.repeat(a, n) + local]
                    d = ((q[rep_q] - src[cand]) ** 2).sum(1)
                    # candidates are grouped by query: per-group minimum, then its first occurrence
                    starts = np.flatnonzero(np.r_[True, rep_q[1:] != rep_q[:-1]])
                    mins = np.minimum.reduceat(d, starts)
                    sizes = np.diff(np.r_[starts, len(d)])
                    hit = np.flatnonzero(d == np.repeat(mins, sizes))
                    first = hit[np.r_[True, rep_q[hit][1:] != rep_q[hit][:-1]]]
                    wq, wd, wi = rep_q[first], np.sqrt(d[first]), cand[first]
                    better = (wd < bd[wq]) | ((wd == bd[wq]) & (wi < bi[wq]))
                    bd[wq[better]] = wd[better]
                    bi[wq[better]] = wi[better]
            t = todo
            bound = ring_bound(q[t], qc[t], r, lo, hi, hv, dims, oq[t] ** 2)
            done = bd[t] ** 2 <= bound
            if not active.any() or r > int(dims.max()) + 1:
                done[:] = True
            todo = t[~done]
            r += 1
        best_i[sl], best_d[sl] = bi, bd
    return best_i, best_d


def ring_bound(q, qc, r, lo, hi, hv, dims, o2_axes):
    """Squared lower bound on the distance from each query to any point outside the cells searched so far (the
    (2r+1)-cube of cells around its clamped cell qc). Along an axis, an unsearched point lies beyond a face of the
    searched box (faces at the grid's edge have nothing beyond them); along every other axis it is at least as far as
    the query is outside the grid."""
    box_lo = lo + (qc - r) * hv
    box_hi = lo + (qc + r + 1) * hv
    below = np.where(qc - r > 0, q - box_lo, np.inf)
    above = np.where(qc + r < dims - 1, box_hi - q, np.inf)
    gap = np.maximum(np.minimum(below, above), 0.0)
    total_o2 = o2_axes.sum(1)
    cand = gap ** 2 + (total_o2[:, None] - o2_axes)
    cand = np.where(dims[None, :] > 1, cand, np.inf)
    return cand.min(1)


_SHELLS = {}


def _shell(r: int, active) -> np.ndarray:
    """Integer offsets on the surface of the (2r+1)-cube, varying only along active axes."""
    key = (r, tuple(bool(x) for x in active))
    s = _SHELLS.get(key)
    if s is None:
        axes = [np.arange(-r, r + 1) if a else np.zeros(1, np.int64) for a in active]
        cube = np.stack(np.meshgrid(*axes, indexing="ij"), -1).reshape(-1, 3)
        s = cube[np.abs(cube).max(1) == r] if r else cube
        if len(_SHELLS) < 256:
            _SHELLS[key] = s
    return s


def components(n: int, a: np.ndarray, b: np.ndarray) -> np.ndarray:
    """Connected-component labels 0..k-1 (in order of each component's lowest element) for n nodes and edges a-b."""
    parent = np.arange(n, dtype=np.int64)
    a = np.asarray(a, np.int64)
    b = np.asarray(b, np.int64)
    while True:
        pa, pb = parent[a], parent[b]
        lo, hi = np.minimum(pa, pb), np.maximum(pa, pb)
        diff = lo != hi
        if not diff.any():
            break
        np.minimum.at(parent, hi[diff], lo[diff])         # hook the larger root under the smaller
        while True:                                        # path compression (pointer jumping)
            pp = parent[parent]
            if np.array_equal(pp, parent):
                break
            parent = pp
    _, labels = np.unique(parent, return_inverse=True)
    return labels.reshape(-1).astype(np.int32)


def pairs_within(P: np.ndarray, r: float):
    """All pairs (i < j) of points closer than ``r``: (i, j, distance). A grid of cell size r, each point checked
    against the 13 'forward' neighbour cells plus its own (each pair found once)."""
    P = np.asarray(P, np.float64).reshape(-1, 3)
    n = len(P)
    if n < 2 or r <= 0:
        return np.zeros(0, np.int64), np.zeros(0, np.int64), np.zeros(0)
    lo = P.min(0)
    cell = np.floor((P - lo) / r).astype(np.int64)
    dims = cell.max(0) + 1
    key = (cell[:, 0] * dims[1] + cell[:, 1]) * dims[2] + cell[:, 2]
    order = np.argsort(key, kind="stable")
    ks = key[order]
    offs = np.array([(dx, dy, dz) for dx in (-1, 0, 1) for dy in (-1, 0, 1) for dz in (-1, 0, 1)])
    forward = offs[[tuple(o) > (0, 0, 0) for o in offs.tolist()]]         # 13 half-space neighbours
    out_i, out_j = [], []
    for off in np.vstack([[0, 0, 0], forward]):
        c = cell + off
        ok = np.all((c >= 0) & (c < dims), axis=1)
        qi = np.flatnonzero(ok)
        k = (c[ok, 0] * dims[1] + c[ok, 1]) * dims[2] + c[ok, 2]
        a = np.searchsorted(ks, k, "left")
        cnt = np.searchsorted(ks, k, "right") - a
        rep = np.repeat(qi, cnt)
        local = np.arange(cnt.sum()) - np.repeat(np.cumsum(cnt) - cnt, cnt)
        cand = order[np.repeat(a, cnt) + local]
        if not off.any():
            keep = cand > rep                                              # same cell: each pair once
            rep, cand = rep[keep], cand[keep]
        out_i.append(rep)
        out_j.append(cand)
    i, j = np.concatenate(out_i), np.concatenate(out_j)
    d = np.linalg.norm(P[i] - P[j], axis=1)
    near = d < r
    return i[near], j[near], d[near]
