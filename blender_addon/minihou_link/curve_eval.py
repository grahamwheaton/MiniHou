"""Turn Blender's curve control data into the polylines Blender itself draws (bpy-free, so it can be tested without Blender).

A curves object stores *control* data: points, and per curve a type (Catmull-Rom, poly, Bezier or NURBS), whether it is cyclic, a
resolution, and for Bezier the handles, for NURBS the weights, order and knot mode. :func:`evaluate` samples each curve the way
Blender does (same number of points, same positions), so a curve comes into the engine looking like it does in the viewport.

Point attributes (radius, tilt, anything else on the points) are carried along by :class:`Sampling`, which remembers for every
output point the control points it blends and with which weights.
"""
from __future__ import annotations

from typing import Dict, List, Optional, Tuple

import numpy as np

CATMULL_ROM, POLY, BEZIER, NURBS = 0, 1, 2, 3          # Blender's CurveType enum
KNOT_NORMAL, KNOT_ENDPOINT, KNOT_BEZIER, KNOT_ENDPOINT_BEZIER = 0, 1, 2, 3


class Sampling:
    """Where the evaluated points came from: ``counts`` points per curve, ``closed`` per curve, ``P`` positions, and a sparse
    blend (``rows``, ``cols``, ``weights``) from control points to evaluated points, so :meth:`blend` can carry any attribute."""

    def __init__(self, P: np.ndarray, counts: np.ndarray, closed: np.ndarray, rows: np.ndarray, cols: np.ndarray, weights: np.ndarray,
                 curve_of_point: np.ndarray):
        self.P, self.counts, self.closed = P, counts, closed
        self.rows, self.cols, self.weights = rows, cols, weights
        self.curve_of_point = curve_of_point                 # evaluated point -> curve number

    @property
    def offsets(self) -> np.ndarray:
        return np.concatenate([[0], np.cumsum(self.counts)]).astype(np.int64)

    def blend(self, values: np.ndarray) -> np.ndarray:
        """Carry a per-control-point attribute (n,) or (n, k) to the evaluated points."""
        v = np.asarray(values)
        wide = v.reshape(len(v), -1).astype(np.float64)
        out = np.zeros((len(self.P), wide.shape[1]), np.float64)
        np.add.at(out, self.rows, wide[self.cols] * self.weights[:, None])
        out = out.astype(v.dtype if v.dtype.kind == "f" else np.float32)
        return out.reshape((len(self.P),) + v.shape[1:])

    def take(self, per_curve: np.ndarray) -> np.ndarray:
        """Carry a per-curve value to the evaluated points."""
        return np.asarray(per_curve)[self.curve_of_point]


def knots(n: int, order: int, mode: int, cyclic: bool) -> np.ndarray:
    """The knot vector for ``n`` control points (``n + order - 1`` of them when cyclic: the first ``order - 1`` points repeat)."""
    m = n + (order - 1 if cyclic else 0)
    length = m + order
    i = np.arange(length, dtype=np.float64)
    if cyclic or mode == KNOT_NORMAL:
        return i
    if mode == KNOT_ENDPOINT:
        return np.clip(i - (order - 1), 0, m - order + 1)
    k = order - 1
    if mode == KNOT_BEZIER:                                       # Blender's legacy formulae (order 3 and 4; others as order 4's idea)
        if order == 4:
            return np.floor(0.34 + i / 3.0)
        if order == 3:
            kk = 0.6
            out = np.empty(length)
            for a in range(length):
                if order <= a <= n:
                    kk += 0.5
                out[a] = np.floor(kk)
            return out
        return np.floor(i / k)
    top = np.floor((m - 1) / k)                                   # endpoint bezier: the ends clamped, bezier joins in between
    out = np.where(i < order, 0.0, np.floor((i - 1) / k))
    return np.minimum(out, top)


def _basis(kn: np.ndarray, order: int, m: int, t: np.ndarray) -> np.ndarray:
    """Dense B-spline basis ``N[j, i]`` of ``order`` for parameters ``t`` over ``m`` control points (Cox - de Boor)."""
    d = order - 1
    t = np.asarray(t, np.float64)
    lo, hi = kn[d], kn[m]
    t = np.clip(t, lo, hi)
    N = np.zeros((len(t), len(kn) - 1))
    for i in range(len(kn) - 1):
        N[:, i] = ((t >= kn[i]) & (t < kn[i + 1])).astype(np.float64)
    at_end = t >= hi                                              # the last parameter belongs to the last non-empty span
    if at_end.any():
        last = np.flatnonzero(kn[:-1] < kn[1:])
        last = last[last < m][-1] if len(last[last < m]) else m - 1
        N[at_end] = 0.0
        N[at_end, last] = 1.0
    for p in range(1, order):
        M = np.zeros((len(t), len(kn) - 1 - p))
        for i in range(len(kn) - 1 - p):
            a = kn[i + p] - kn[i]
            b = kn[i + p + 1] - kn[i + 1]
            left = (t - kn[i]) / a * N[:, i] if a > 0 else 0.0
            right = (kn[i + p + 1] - t) / b * N[:, i + 1] if b > 0 else 0.0
            M[:, i] = left + right
        N = M
    return N[:, :m]


def _eval_nurbs(P: np.ndarray, w: np.ndarray, order: int, mode: int, cyclic: bool, res: int):
    """``(points, weights matrix over the control points)`` for one NURBS curve."""
    n = len(P)
    order = int(max(2, min(order, n))) if n >= 2 else 2
    if n < order:
        return P.copy(), np.eye(n)
    m = n + (order - 1 if cyclic else 0)
    idx = np.arange(m) % n if cyclic else np.arange(n)
    kn = knots(n, order, mode, cyclic)
    lo, hi = kn[order - 1], kn[m]
    spans = int(np.count_nonzero(np.diff(kn[order - 1:m + 1]) > 1e-12))
    steps = max(res * spans, 1)
    t = lo + (hi - lo) * (np.arange(steps if cyclic else steps + 1) / steps)
    B = _basis(kn, order, m, t) * w[idx][None, :]
    B /= np.maximum(B.sum(1, keepdims=True), 1e-300)
    W = np.zeros((len(t), n))
    for j in range(m):
        W[:, idx[j]] += B[:, j]
    return W @ P, W


def _eval_catmull(P: np.ndarray, cyclic: bool, res: int):
    n = len(P)
    seg = n if cyclic else n - 1
    k = np.arange(seg)
    if cyclic:
        ia, ib, ic, id_ = (k - 1) % n, k, (k + 1) % n, (k + 2) % n
    else:
        ia, ib, ic, id_ = np.maximum(k - 1, 0), k, k + 1, np.minimum(k + 2, n - 1)
    t = (np.arange(res) / res)[None, :, None]
    t2, t3 = t * t, t * t * t
    a, b, c, d = (P[i][:, None, :] for i in (ia, ib, ic, id_))
    pts = 0.5 * ((2 * b) + (-a + c) * t + (2 * a - 5 * b + 4 * c - d) * t2 + (-a + 3 * b - 3 * c + d) * t3)
    # weights over control points: the same formula applied to unit vectors
    wa = 0.5 * (-t + 2 * t2 - t3)
    wb = 0.5 * (2 - 5 * t2 + 3 * t3)
    wc = 0.5 * (t + 4 * t2 - 3 * t3)
    wd = 0.5 * (-t2 + t3)
    pts = pts.reshape(-1, 3)
    rows = np.arange(seg * res)
    rows4 = np.concatenate([rows] * 4)
    cols = np.concatenate([np.repeat(ia, res), np.repeat(ib, res), np.repeat(ic, res), np.repeat(id_, res)])
    wts = np.concatenate([np.broadcast_to(x, (seg, res, 1)).reshape(-1) for x in (wa, wb, wc, wd)])
    if not cyclic:
        pts = np.concatenate([pts, P[-1:]])
        rows4 = np.concatenate([rows4, np.full(1, seg * res)])
        cols = np.concatenate([cols, [n - 1]])
        wts = np.concatenate([wts, [1.0]])
    return pts, rows4, cols, wts


def evaluate(position: np.ndarray, offsets: np.ndarray, curve_type: Optional[np.ndarray] = None, cyclic: Optional[np.ndarray] = None,
             resolution: Optional[np.ndarray] = None, handle_left: Optional[np.ndarray] = None, handle_right: Optional[np.ndarray] = None,
             nurbs_weight: Optional[np.ndarray] = None, nurbs_order: Optional[np.ndarray] = None,
             knots_mode: Optional[np.ndarray] = None) -> Sampling:
    """Sample every curve like Blender does. ``position`` (n, 3) with ``offsets`` (curves + 1) delimiting each curve's points; the rest are
    per curve (``curve_type`` .. ``resolution``, ``nurbs_order``, ``knots_mode``) or per point (handles, ``nurbs_weight``). Anything
    missing takes Blender's default (poly curves, not cyclic, resolution 1, order 4, weights 1)."""
    position = np.asarray(position, np.float64).reshape(-1, 3)
    offsets = np.asarray(offsets, np.int64)
    ncurves = len(offsets) - 1
    ctype = np.full(ncurves, POLY, np.int64) if curve_type is None else np.asarray(curve_type, np.int64)
    cyc = np.zeros(ncurves, bool) if cyclic is None else np.asarray(cyclic, bool)
    res = np.ones(ncurves, np.int64) if resolution is None else np.maximum(np.asarray(resolution, np.int64), 1)
    hl = position if handle_left is None else np.asarray(handle_left, np.float64).reshape(-1, 3)
    hr = position if handle_right is None else np.asarray(handle_right, np.float64).reshape(-1, 3)
    wgt = np.ones(len(position)) if nurbs_weight is None else np.asarray(nurbs_weight, np.float64)
    order = np.full(ncurves, 4, np.int64) if nurbs_order is None else np.asarray(nurbs_order, np.int64)
    kmode = np.zeros(ncurves, np.int64) if knots_mode is None else np.asarray(knots_mode, np.int64)
    pts_parts: List[np.ndarray] = []
    rows_parts, cols_parts, w_parts = [], [], []
    counts = np.zeros(ncurves, np.int64)
    base = 0
    # poly curves, the common case for hair and imported data, need no per-curve work beyond copying
    for c in range(ncurves):
        a, b = int(offsets[c]), int(offsets[c + 1])
        n = b - a
        if n == 0:
            continue
        P = position[a:b]
        kind = int(ctype[c])
        r = int(res[c])
        if n == 1 or kind == POLY:
            out, rows, cols, wts = P, np.arange(n), np.arange(n) + a, np.ones(n)
        elif kind == BEZIER:
            seg = n if cyc[c] else n - 1
            i0 = np.arange(seg)
            i1 = (i0 + 1) % n
            p0, p1, p2, p3 = P[i0], hr[a:b][i0], hl[a:b][i1], P[i1]
            t = (np.arange(r) / r)[None, :, None]
            s = 1 - t
            out = (s ** 3 * p0[:, None] + 3 * s * s * t * p1[:, None] + 3 * s * t * t * p2[:, None] + t ** 3 * p3[:, None]).reshape(-1, 3)
            # attributes of a Bezier curve blend linearly between the two control points of the segment
            tt = np.broadcast_to(t[..., 0], (seg, r)).reshape(-1)
            ii0, ii1 = np.repeat(i0, r), np.repeat(i1, r)
            rows = np.concatenate([np.arange(seg * r)] * 2)
            cols = np.concatenate([ii0, ii1]) + a
            wts = np.concatenate([1 - tt, tt])
            if not cyc[c]:
                out = np.concatenate([out, P[-1:]])
                rows = np.concatenate([rows, [seg * r]])
                cols = np.concatenate([cols, [b - 1]])
                wts = np.concatenate([wts, [1.0]])
            else:
                pass
            m_out = len(out)
            rows_parts.append(rows + base)
            cols_parts.append(cols)
            w_parts.append(wts)
            pts_parts.append(out)
            counts[c] = m_out
            base += m_out
            continue
        elif kind == NURBS and n >= 2:
            out, W = _eval_nurbs(P, wgt[a:b], int(order[c]), int(kmode[c]), bool(cyc[c]), r)
            rr, cc = np.nonzero(W > 1e-12)
            rows, cols, wts = rr, cc + a, W[rr, cc]
        elif kind == CATMULL_ROM and n >= 2:
            out, rows, cols, wts = _eval_catmull(P, bool(cyc[c]), r)
            cols = cols + a
        else:
            out, rows, cols, wts = P, np.arange(n), np.arange(n) + a, np.ones(n)
        m_out = len(out)
        rows_parts.append(np.asarray(rows) + base)
        cols_parts.append(np.asarray(cols))
        w_parts.append(np.asarray(wts, np.float64))
        pts_parts.append(np.asarray(out, np.float64))
        counts[c] = m_out
        base += m_out
    P_out = np.concatenate(pts_parts).astype(np.float32) if pts_parts else np.zeros((0, 3), np.float32)
    rows = np.concatenate(rows_parts) if rows_parts else np.zeros(0, np.int64)
    cols = np.concatenate(cols_parts) if cols_parts else np.zeros(0, np.int64)
    wts = np.concatenate(w_parts) if w_parts else np.zeros(0)
    curve_of_point = np.repeat(np.arange(ncurves), counts)
    return Sampling(P_out, counts, cyc & (np.diff(offsets) > 2), rows, cols, wts, curve_of_point)


def control_polygon(position: np.ndarray, offsets: np.ndarray, cyclic: np.ndarray) -> Sampling:
    """The control points themselves, one primitive per curve (``evaluate`` with every curve treated as a poly curve)."""
    n = len(offsets) - 1
    return evaluate(position, offsets, np.full(n, POLY, np.int64), cyclic, np.ones(n, np.int64))
