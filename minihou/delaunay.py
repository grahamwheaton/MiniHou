"""2-D Delaunay triangulation of a point set.

``triangulate`` uses SciPy's Qhull when SciPy is installed and otherwise an incremental Bowyer-Watson in plain Python
(points inserted in a space-filling order so the walk to each point's triangle stays short; fine up to a few tens of
thousands of points). Both return counter-clockwise triangles over the *distinct* points; duplicates and points that
cannot be part of a triangle (all collinear) simply do not appear.
"""
from __future__ import annotations

from typing import List

import numpy as np

try:                                       # optional: much faster, and handles millions of points
    from scipy.spatial import Delaunay as _Delaunay
except Exception:  # noqa: BLE001
    _Delaunay = None


def backend() -> str:
    return "scipy" if _Delaunay is not None else "python"


def _area2(xy, tri):
    a, b, c = xy[tri[:, 0]], xy[tri[:, 1]], xy[tri[:, 2]]
    return (b[:, 0] - a[:, 0]) * (c[:, 1] - a[:, 1]) - (b[:, 1] - a[:, 1]) * (c[:, 0] - a[:, 0])


def triangulate(xy, use_scipy: bool = True) -> np.ndarray:
    """Counter-clockwise Delaunay triangles (T, 3) of the points xy (n, 2), as indices into xy."""
    xy = np.asarray(xy, np.float64).reshape(-1, 2)
    if len(xy) < 3:
        return np.zeros((0, 3), np.int64)
    # work on distinct points (the first of each coincident group), centred for precision
    uniq, first = np.unique(xy, axis=0, return_index=True)
    if len(uniq) < 3:
        return np.zeros((0, 3), np.int64)
    pts = uniq - uniq.mean(0)
    if _Delaunay is not None and use_scipy:
        try:
            tri = _Delaunay(pts).simplices.astype(np.int64)
        except Exception:  # noqa: BLE001 - QhullError: all points collinear, etc.
            return np.zeros((0, 3), np.int64)
    else:
        tri = _bowyer_watson(pts)
    if not len(tri):
        return tri
    area = _area2(pts, tri)
    flip = area < 0
    tri[flip] = tri[flip][:, [0, 2, 1]]
    scale = max(float(np.ptp(pts, 0).max()), 1e-300)
    tri = tri[np.abs(area) > 1e-12 * scale * scale]          # drop slivers on the hull (collinear corners)
    return first[tri]


def _space_filling_order(pts: np.ndarray) -> np.ndarray:
    """A boustrophedon sweep over a grid of about sqrt(n/2) strips: consecutive points are close."""
    lo, hi = pts.min(0), pts.max(0)
    ext = np.maximum(hi - lo, 1e-300)
    strips = max(int(np.sqrt(len(pts) / 2.0)), 1)
    s = np.minimum(((pts[:, 1] - lo[1]) / ext[1] * strips).astype(np.int64), strips - 1)
    x = (pts[:, 0] - lo[0]) / ext[0]
    return np.lexsort((np.where(s % 2 == 0, x, -x), s))


def _bowyer_watson(pts: np.ndarray) -> np.ndarray:
    n = len(pts)
    ext = float(np.ptp(pts, 0).max()) or 1.0
    big = 1e5 * ext
    P: List[tuple] = [(float(x), float(y)) for x, y in pts]
    P += [(-big, -big), (big, -big), (0.0, big)]                  # super triangle, counter-clockwise
    V: List[List[int]] = [[n, n + 1, n + 2]]                      # triangle vertices (counter-clockwise)
    N: List[List[int]] = [[-1, -1, -1]]                           # N[t][i]: triangle across edge (V[t][i], V[t][i+1])
    alive = [True]
    last = 0

    def orient(a, b, c):
        return (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])

    def in_circle(t, d):
        a, b, c = (P[v] for v in V[t])
        ax, ay, bx, by, cx, cy = a[0] - d[0], a[1] - d[1], b[0] - d[0], b[1] - d[1], c[0] - d[0], c[1] - d[1]
        return ((ax * ax + ay * ay) * (bx * cy - cx * by) - (bx * bx + by * by) * (ax * cy - cx * ay)
                + (cx * cx + cy * cy) * (ax * by - bx * ay)) > 0.0

    for pi in _space_filling_order(pts).tolist():
        p = P[pi]
        # walk to the triangle containing p (fall back to a scan if the walk cycles on degenerate input)
        t, steps = last, 0
        while True:
            steps += 1
            if steps > 4 * len(V) + 16:
                t = next(k for k in range(len(V)) if alive[k] and in_circle(k, p))
                break
            moved = False
            for i in range(3):
                if orient(P[V[t][i]], P[V[t][(i + 1) % 3]], p) < 0.0 and N[t][i] >= 0:
                    t, moved = N[t][i], True
                    break
            if not moved:
                break
        # cavity: connected triangles whose circumcircle holds p
        cavity, stack = {t}, [t]
        while stack:
            c = stack.pop()
            for nb in N[c]:
                if nb >= 0 and nb not in cavity and in_circle(nb, p):
                    cavity.add(nb)
                    stack.append(nb)
        boundary = [(V[c][i], V[c][(i + 1) % 3], N[c][i], c) for c in cavity for i in range(3) if N[c][i] not in cavity]
        for c in cavity:
            alive[c] = False
        by_first, by_second, new_ids = {}, {}, []
        for a, b, outer, dead in boundary:
            k = len(V)
            V.append([a, b, pi])
            N.append([outer, -1, -1])
            alive.append(True)
            by_first[a], by_second[b] = k, k
            new_ids.append(k)
            if outer >= 0:
                o = N[outer]
                o[o.index(dead)] = k
        for k in new_ids:
            a, b, _ = V[k]
            N[k][1] = by_first.get(b, -1)                 # across (b, p): the new triangle that starts at b
            N[k][2] = by_second.get(a, -1)                # across (p, a): the new triangle that ends at a
        last = new_ids[0]
    keep = [V[k] for k in range(len(V)) if alive[k] and max(V[k]) < n]
    return np.array(keep, np.int64).reshape(-1, 3)
