"""Ray casting and closest points on triangle meshes, vectorised over all queries with a uniform grid.

* :class:`TriGrid` registers triangles in the grid cells their bounding boxes overlap.
* :meth:`TriGrid.intersect` walks every ray through the grid at once (3-D DDA, one cell per step for all rays) and tests
  the triangles of the current cell (Moller-Trumbore). A hit inside the current cell is the nearest one, so the ray
  stops there.
* :meth:`TriGrid.closest` searches rings of cells around each query (like :func:`minihou.spatial.nearest`) with exact
  point-triangle distances, stopping once nothing unsearched can be closer.

Both are exact (tested against brute force). With numba installed, :mod:`minihou.nodes._kernels` runs the same two
traversals per query in compiled code (``grid_intersect``, ``grid_closest``); otherwise the numpy code below runs.
"""
from __future__ import annotations

from typing import Tuple

import numpy as np

CHUNK = 8192


class TriGrid:
    def __init__(self, tris: np.ndarray):
        tris = np.asarray(tris, np.float64).reshape(-1, 3, 3)
        self.tris = tris
        self.n = len(tris)
        if self.n == 0:
            return
        tlo, thi = tris.min(1), tris.max(1)
        lo, hi = tlo.min(0), thi.max(0)
        ext = np.maximum(hi - lo, 1e-9)
        big = float(ext.max())
        tri_ext = np.percentile((thi - tlo).max(1), 90)
        spans = ext >= 0.02 * big
        # cells about the size of the triangles: from the area per triangle (meshes are surfaces, so a volume-based
        # size would put many triangles in the few occupied cells), never smaller than most triangles
        area = 0.5 * np.linalg.norm(np.cross(tris[:, 1] - tris[:, 0], tris[:, 2] - tris[:, 0]), axis=1).sum()
        h = max(float(np.sqrt(2.0 * area / self.n)), float(tri_ext), big * 1e-5)
        hv = np.where(spans, h, ext * 1.0001 + 1e-9)
        limit = 16 * self.n + 1_000_000
        while True:
            dims = np.minimum(np.floor(ext / hv).astype(np.int64) + 1, 1024)
            if int(dims.prod()) <= limit:
                break
            hv = np.where(spans, hv * 1.25, hv)                               # too many cells: coarser
        hv = np.maximum(hv, ext / dims * 1.000001)                          # keep the grid covering everything
        self.lo, self.hi, self.hv, self.dims = lo, lo + hv * dims, hv, dims
        c0 = self._cell(tlo)
        c1 = self._cell(thi)
        span = c1 - c0 + 1
        cnt = span.prod(1)
        tri_id = np.repeat(np.arange(self.n), cnt)
        local = np.arange(cnt.sum()) - np.repeat(np.cumsum(cnt) - cnt, cnt)
        sp = span[tri_id]
        off = np.stack([local // (sp[:, 1] * sp[:, 2]), (local // sp[:, 2]) % sp[:, 1], local % sp[:, 2]], 1)
        keys = self._key(c0[tri_id] + off)
        order = np.argsort(keys, kind="stable")
        self.cell_tris = tri_id[order]
        ncells = int(dims.prod())
        sk = keys[order]
        self.cstart = np.searchsorted(sk, np.arange(ncells), "left")
        self.ccount = np.bincount(keys, minlength=ncells)

    def _cell(self, p):
        return np.clip(np.floor((p - self.lo) / self.hv).astype(np.int64), 0, self.dims - 1)

    def _key(self, c):
        return (c[..., 0] * self.dims[1] + c[..., 1]) * self.dims[2] + c[..., 2]

    def _candidates(self, qi, cells):
        """(query index, triangle index) pairs for the triangles registered in each query's cell."""
        k = self._key(cells)
        a, n = self.cstart[k], self.ccount[k]
        rep = np.repeat(qi, n)
        local = np.arange(n.sum()) - np.repeat(np.cumsum(n) - n, n)
        return rep, self.cell_tris[np.repeat(a, n) + local]

    # -- rays ---------------------------------------------------------------------------------------------------
    def intersect(self, orig, dirs, max_dist=np.inf) -> Tuple[np.ndarray, np.ndarray, np.ndarray]:
        """Nearest hit along each ray (both faces count). Returns (distance along the unit direction or inf,
        triangle index or -1, barycentric (u, v) of the hit)."""
        orig = np.asarray(orig, np.float64).reshape(-1, 3)
        d = np.asarray(dirs, np.float64).reshape(-1, 3)
        d = d / np.maximum(np.linalg.norm(d, axis=1, keepdims=True), 1e-30)
        m = len(orig)
        best_t, best_tri, best_uv = np.full(m, np.inf), np.full(m, -1, np.int64), np.zeros((m, 2))
        if self.n == 0 or m == 0:
            return best_t, best_tri, best_uv
        if self._kernel_intersect(orig, d, max_dist, best_t, best_tri, best_uv):
            return best_t, best_tri, best_uv
        for s in range(0, m, CHUNK):
            sl = slice(s, s + CHUNK)
            self._intersect_chunk(orig[sl], d[sl], max_dist, best_t[sl], best_tri[sl], best_uv[sl])
        return best_t, best_tri, best_uv

    def _arrays(self):
        """The grid as contiguous arrays for the compiled kernels."""
        if not hasattr(self, "_karr"):
            self._karr = (np.ascontiguousarray(self.tris), np.ascontiguousarray(self.cstart, np.int64),
                          np.ascontiguousarray(self.ccount, np.int64), np.ascontiguousarray(self.cell_tris, np.int64),
                          np.ascontiguousarray(self.lo, np.float64), np.ascontiguousarray(self.hi, np.float64),
                          np.ascontiguousarray(self.hv, np.float64), np.ascontiguousarray(self.dims, np.int64))
        return self._karr

    def _kernel_intersect(self, o, d, max_dist, bt, btri, buv) -> bool:
        from . import accel
        k = accel.kernels()
        if k is None:
            return False
        tris, cstart, ccount, cell_tris, lo, hi, hv, dims = self._arrays()
        r = accel.call(k.grid_intersect, np.ascontiguousarray(o), np.ascontiguousarray(d), tris, cstart, ccount,
                       cell_tris, lo, hi, hv, dims, float(max_dist), bt, btri, buv)
        return r is not accel.FAILED

    def _intersect_chunk(self, o, d, max_dist, bt, btri, buv):
        with np.errstate(divide="ignore", invalid="ignore"):
            inv = 1.0 / d
            t_lo = (self.lo - o) * inv
            t_hi = (self.hi - o) * inv
        tmin = np.nanmax(np.minimum(t_lo, t_hi), axis=1)
        tmax = np.nanmin(np.maximum(t_lo, t_hi), axis=1)
        t_enter = np.maximum(tmin, 0.0)
        active = np.flatnonzero((t_enter <= tmax) & (t_enter <= max_dist))
        if not len(active):
            return
        p = o[active] + d[active] * t_enter[active, None]
        cell = self._cell(p)
        step = np.where(d[active] >= 0, 1, -1)
        nxt_bound = self.lo + (cell + (step > 0)) * self.hv
        with np.errstate(divide="ignore", invalid="ignore"):
            t_next = np.where(d[active] != 0, (nxt_bound - o[active]) * inv[active], np.inf)
            t_delta = np.where(d[active] != 0, self.hv / np.abs(d[active]), np.inf)
        idx = active
        eps = 1e-9 * float(self.hv.max())
        for _ in range(int(self.dims.sum()) + 3):
            if not len(idx):
                break
            t_cell_out = t_next.min(1)
            rep, tri = self._candidates(np.arange(len(idx)), cell)
            if len(rep):
                t, u, v, ok = _moller_trumbore(o[idx[rep]], d[idx[rep]], self.tris[tri])
                ok &= (t >= 0) & (t <= max_dist)
                rep, tri, t, u, v = rep[ok], tri[ok], t[ok], u[ok], v[ok]
                if len(rep):
                    first = _first_min(rep, t)
                    q = idx[rep[first]]
                    better = t[first] < bt[q]
                    q, f = q[better], first[better]
                    bt[q], btri[q], buv[q] = t[f], tri[f], np.stack([u[f], v[f]], 1)
            # done: hit within this cell (nothing later can be nearer), left the grid, or beyond max distance
            done = (bt[idx] <= t_cell_out + eps) | (t_cell_out > max_dist)
            axis = np.argmin(t_next, axis=1)
            r = np.arange(len(idx))
            cell[r, axis] += step[r, axis]
            t_next[r, axis] += t_delta[r, axis]
            out = np.any((cell < 0) | (cell >= self.dims), axis=1)
            keep = ~(done | out)
            idx, cell, step, t_next, t_delta = idx[keep], cell[keep], step[keep], t_next[keep], t_delta[keep]

    def _kernel_closest(self, q, best_d, best_tri, best_p) -> bool:
        from . import accel
        k = accel.kernels()
        if k is None:
            return False
        tris, cstart, ccount, cell_tris, lo, _, hv, dims = self._arrays()
        r = accel.call(k.grid_closest, np.ascontiguousarray(q), tris, cstart, ccount, cell_tris, lo, hv, dims,
                       best_d, best_tri, best_p)
        return r is not accel.FAILED

    # -- closest points -----------------------------------------------------------------------------------------
    def closest(self, pts) -> Tuple[np.ndarray, np.ndarray, np.ndarray]:
        """Closest point on the mesh for each query: (point (m, 3), distance, triangle index)."""
        q = np.asarray(pts, np.float64).reshape(-1, 3)
        m = len(q)
        best_d, best_tri, best_p = np.full(m, np.inf), np.full(m, -1, np.int64), q.copy()
        if self.n == 0 or m == 0:
            return best_p, best_d, best_tri
        active_axes = self.dims > 1
        from .spatial import _shell, nearest, ring_bound
        # the nearest triangle corner bounds the answer from above: searches end as soon as rings pass it
        vi, vd = nearest(self.tris.reshape(-1, 3), q)
        best_d[:] = vd
        best_tri[:] = vi // 3
        best_p[:] = self.tris.reshape(-1, 3)[vi]
        if self._kernel_closest(q, best_d, best_tri, best_p):
            return best_p, best_d, best_tri
        qc_all = self._cell(q)
        o_all = np.maximum(np.maximum(self.lo - q, q - self.hi), 0.0)
        if not hasattr(self, "_tlo"):
            self._tlo, self._thi = self.tris.min(1), self.tris.max(1)
        for s in range(0, m, CHUNK):
            sl = slice(s, s + CHUNK)
            qq, qc, oq = q[sl], qc_all[sl], o_all[sl]
            bd, btri, bp = best_d[sl], best_tri[sl], best_p[sl]
            todo = np.arange(len(qq))
            r = 0
            while len(todo):
                offs = _shell(r, active_axes)
                c = qc[todo][:, None, :] + offs[None]
                ok = np.all((c >= 0) & (c < self.dims), axis=2)
                # skip cells whose box is farther than the best distance so far (the corners of a ring mostly)
                box_lo = self.lo + c * self.hv
                gap = np.maximum(np.maximum(box_lo - qq[todo][:, None, :], qq[todo][:, None, :] - box_lo - self.hv), 0)
                ok &= (gap ** 2).sum(2) < bd[todo][:, None] ** 2
                qi = np.broadcast_to(todo[:, None], ok.shape)[ok]
                if len(qi):
                    rep, tri = self._candidates(qi, c[ok])
                    # cheap reject: a triangle whose bounding box is farther than the best so far cannot win
                    gap = np.maximum(np.maximum(self._tlo[tri] - qq[rep], qq[rep] - self._thi[tri]), 0.0)
                    near = (gap ** 2).sum(1) < bd[rep] ** 2
                    rep, tri = rep[near], tri[near]
                    if len(rep):
                        cp = _closest_on_tri(qq[rep], self.tris[tri])
                        dd = np.sqrt(((cp - qq[rep]) ** 2).sum(1))
                        first = _first_min(rep, dd)
                        w = rep[first]
                        better = dd[first] < bd[w]
                        w, f = w[better], first[better]
                        bd[w], btri[w], bp[w] = dd[f], tri[f], cp[f]
                bound = ring_bound(qq[todo], qc[todo], r, self.lo, self.hi, self.hv, self.dims, oq[todo] ** 2)
                done = bd[todo] ** 2 <= bound
                if not active_axes.any() or r > int(self.dims.max()) + 1:
                    done[:] = True
                todo = todo[~done]
                r += 1
        return best_p, best_d, best_tri


def _first_min(group, value):
    """Index (into the arrays) of the smallest value per run of equal ``group`` (runs must be contiguous)."""
    starts = np.flatnonzero(np.r_[True, group[1:] != group[:-1]])
    mins = np.minimum.reduceat(value, starts)
    sizes = np.diff(np.r_[starts, len(value)])
    hit = np.flatnonzero(value == np.repeat(mins, sizes))
    return hit[np.r_[True, group[hit][1:] != group[hit][:-1]]]


def _moller_trumbore(o, d, tri):
    v0, v1, v2 = tri[:, 0], tri[:, 1], tri[:, 2]
    e1, e2 = v1 - v0, v2 - v0
    pv = np.cross(d, e2)
    det = np.einsum("ij,ij->i", e1, pv)
    ok = np.abs(det) > 1e-14 * np.maximum(np.linalg.norm(e1, axis=1) * np.linalg.norm(e2, axis=1), 1e-300)
    inv = np.where(ok, 1.0 / np.where(ok, det, 1.0), 0.0)
    tv = o - v0
    u = np.einsum("ij,ij->i", tv, pv) * inv
    qv = np.cross(tv, e1)
    v = np.einsum("ij,ij->i", d, qv) * inv
    t = np.einsum("ij,ij->i", e2, qv) * inv
    ok &= (u >= -1e-12) & (v >= -1e-12) & (u + v <= 1 + 1e-12)
    return t, u, v, ok


def _closest_on_tri(p, tri):
    """Closest point on each triangle to each point (Ericson, Real-Time Collision Detection 5.1.5), vectorised."""
    a, b, c = tri[:, 0], tri[:, 1], tri[:, 2]
    ab, ac, ap = b - a, c - a, p - a
    dot = lambda x, y: np.einsum("ij,ij->i", x, y)  # noqa: E731
    d1, d2 = dot(ab, ap), dot(ac, ap)
    bp = p - b
    d3, d4 = dot(ab, bp), dot(ac, bp)
    cp = p - c
    d5, d6 = dot(ab, cp), dot(ac, cp)
    vc = d1 * d4 - d3 * d2
    vb = d5 * d2 - d1 * d6
    va = d3 * d6 - d5 * d4
    res = np.empty_like(p)
    done = np.zeros(len(p), bool)

    def put(mask, val):
        m = mask & ~done
        res[m] = val[m]
        done[:] = done | m

    put((d1 <= 0) & (d2 <= 0), a)                                            # vertex regions
    put((d3 >= 0) & (d4 <= d3), b)
    put((d6 >= 0) & (d5 <= d6), c)
    with np.errstate(divide="ignore", invalid="ignore"):
        v_ab = d1 / (d1 - d3)
        put((vc <= 0) & (d1 >= 0) & (d3 <= 0), a + ab * v_ab[:, None])       # edge regions
        w_ac = d2 / (d2 - d6)
        put((vb <= 0) & (d2 >= 0) & (d6 <= 0), a + ac * w_ac[:, None])
        w_bc = (d4 - d3) / ((d4 - d3) + (d5 - d6))
        put((va <= 0) & ((d4 - d3) >= 0) & ((d5 - d6) >= 0), b + (c - b) * w_bc[:, None])
        denom = 1.0 / (va + vb + vc)
        v, w = vb * denom, vc * denom
        put(np.ones(len(p), bool), a + ab * v[:, None] + ac * w[:, None])    # inside the face
    return res


_PARITY_DIRS = np.array([[0.5774, 0.4122, 0.7046], [-0.3313, 0.8661, 0.3742], [0.7421, -0.5167, -0.4271]])


def inside(grid: "TriGrid", pts, scale: float = 1.0) -> np.ndarray:
    """Bool per point: inside the closed mesh of ``grid``. Counts crossings along three skewed rays (parity) and takes
    the majority, so a ray grazing an edge or a vertex cannot flip the answer."""
    pts = np.asarray(pts, np.float64).reshape(-1, 3)
    votes = np.zeros(len(pts), np.int32)
    if grid.n == 0 or len(pts) == 0:
        return votes > 0
    eps = 1e-7 * max(scale, 1e-12)
    for d in _PARITY_DIRS / np.linalg.norm(_PARITY_DIRS, axis=1, keepdims=True):
        parity = np.zeros(len(pts), np.int32)
        idx = np.arange(len(pts))
        o = pts.copy()
        for _ in range(1024):                                # crossings per ray: bounded by the mesh
            if not len(idx):
                break
            t, _, _ = grid.intersect(o[idx], np.broadcast_to(d, (len(idx), 3)))
            hit = np.isfinite(t)
            idx, t = idx[hit], t[hit]
            parity[idx] ^= 1
            o[idx] += d * (t + eps)[:, None]
        votes += parity
    return votes >= 2
