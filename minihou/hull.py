"""3-D convex hull of a point set: outward-facing triangles over a subset of the points.

SciPy's Qhull when installed; otherwise an incremental hull in numpy (each new point removes the faces it sees and
joins the horizon to itself; the visibility test is vectorised over faces, so it is fast for hulls of up to a few
thousand faces however many points are inside).
"""
from __future__ import annotations

import numpy as np

try:
    from scipy.spatial import ConvexHull as _ConvexHull
except Exception:  # noqa: BLE001
    _ConvexHull = None


def backend() -> str:
    return "scipy" if _ConvexHull is not None else "numpy"


def _planes(P, F):
    n = np.cross(P[F[:, 1]] - P[F[:, 0]], P[F[:, 2]] - P[F[:, 0]])
    ln = np.maximum(np.linalg.norm(n, axis=1, keepdims=True), 1e-300)
    n = n / ln
    return n, np.einsum("ij,ij->i", n, P[F[:, 0]])


def _orient_outward(P, F):
    """Flip triangles whose normal points towards the hull's interior."""
    inside = P[np.unique(F)].mean(0)
    n, d = _planes(P, F)
    flip = (n @ inside - d) > 0
    F = F.copy()
    F[flip] = F[flip][:, [0, 2, 1]]
    return F


def convex_hull(P, use_scipy: bool = True) -> np.ndarray:
    """Triangles (T, 3), indices into P, counter-clockwise seen from outside. Empty if the points are not
    full-dimensional (all coplanar, collinear or identical)."""
    P = np.asarray(P, np.float64).reshape(-1, 3)
    empty = np.zeros((0, 3), np.int64)
    if len(P) < 4:
        return empty
    uniq, first = np.unique(P, axis=0, return_index=True)
    if len(uniq) < 4:
        return empty
    Q = uniq - uniq.mean(0)
    scale = float(np.ptp(Q, 0).max())
    if _ConvexHull is not None and use_scipy:
        try:
            F = _ConvexHull(Q).simplices.astype(np.int64)
        except Exception:  # noqa: BLE001 - QhullError: flat or degenerate input
            return empty
    else:
        F = _incremental(Q, scale)
    if not len(F):
        return empty
    return first[_orient_outward(Q, F)]


def _incremental(Q, scale):
    eps = 1e-10 * scale
    n = len(Q)
    # a non-degenerate starting tetrahedron: extremes along x, then farthest from that line, then from that plane
    i0, i1 = int(np.argmin(Q[:, 0])), int(np.argmax(Q[:, 0]))
    if i0 == i1:
        return np.zeros((0, 3), np.int64)
    d = Q[i1] - Q[i0]
    d /= np.linalg.norm(d)
    off = Q - Q[i0]
    line_dist = np.linalg.norm(off - np.outer(off @ d, d), axis=1)
    i2 = int(np.argmax(line_dist))
    if line_dist[i2] <= eps:
        return np.zeros((0, 3), np.int64)
    nrm = np.cross(Q[i1] - Q[i0], Q[i2] - Q[i0])
    nrm /= np.linalg.norm(nrm)
    plane_dist = off @ nrm
    i3 = int(np.argmax(np.abs(plane_dist)))
    if abs(plane_dist[i3]) <= eps:
        return np.zeros((0, 3), np.int64)
    F = np.array([[i0, i1, i2], [i0, i3, i1], [i1, i3, i2], [i2, i3, i0]], np.int64)
    F = _orient_outward(Q, F)
    done = np.zeros(n, bool)
    done[[i0, i1, i2, i3]] = True
    order = np.argsort(-np.linalg.norm(Q, axis=1), kind="stable")          # far points first: the hull settles early
    N, D = _planes(Q, F)
    for p in order.tolist():
        if done[p]:
            continue
        vis = (N @ Q[p] - D) > eps
        if not vis.any():
            continue
        VF = F[vis]
        edges = {(int(a), int(b)) for a, b, c in VF for a, b in ((a, b), (b, c), (c, a))}
        horizon = [(a, b) for a, b in edges if (b, a) not in edges]
        new = np.array([(a, b, p) for a, b in horizon], np.int64)
        keep = ~vis
        F = np.concatenate([F[keep], new])
        nN, nD = _planes(Q, new)
        N, D = np.concatenate([N[keep], nN]), np.concatenate([D[keep], nD])
    return F
