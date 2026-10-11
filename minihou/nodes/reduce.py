"""PolyReduce (Houdini's PolyReduce): simplify a mesh by quadric-error edge collapses (Garland & Heckbert 1997).

Each point carries the sum of the area-weighted plane quadrics of its faces (plus stiff planes along open borders, so
outlines keep their shape). Collapsing an edge moves its two points to the position that minimises the summed error.
Instead of one collapse at a time, rounds collapse many edges at once, all vectorised: an edge is collapsed when it is
the cheapest edge at *both* its points (so collapses never share a point), the link condition holds (the result stays
manifold) and no surrounding triangle flips or degenerates with all of the round's moves applied. Rounds repeat until
the target is reached or nothing can collapse. The result is triangles.
"""
import numpy as np

from ..geometry import rows
from ..sdk import Bool, Float, Int, node
from . import _build as B
from ._util import fan_triangulate


def _plane_quadrics(P, T):
    """Per triangle: area-weighted quadric (10 unique terms of the 4x4 matrix) of its plane."""
    n = np.cross(P[T[:, 1]] - P[T[:, 0]], P[T[:, 2]] - P[T[:, 0]])
    area2 = np.linalg.norm(n, axis=1)
    n = n / np.maximum(area2, 1e-300)[:, None]
    d = -np.einsum("ij,ij->i", n, P[T[:, 0]])
    return _quadric(n, d, area2 / 2.0)


def _quadric(n, d, w):
    a, b, c = n[:, 0], n[:, 1], n[:, 2]
    return w[:, None] * np.stack([a * a, a * b, a * c, a * d, b * b, b * c, b * d, c * c, c * d, d * d], 1)


def _solve(Q, fallback_a, fallback_b):
    """Optimal positions for summed quadrics Q (m, 10); ill-conditioned systems fall back to the best of the two ends
    and the midpoint."""
    A = np.stack([np.stack([Q[:, 0], Q[:, 1], Q[:, 2]], 1), np.stack([Q[:, 1], Q[:, 4], Q[:, 5]], 1),
                  np.stack([Q[:, 2], Q[:, 5], Q[:, 7]], 1)], 1)
    rhs = -np.stack([Q[:, 3], Q[:, 6], Q[:, 8]], 1)
    det = np.linalg.det(A)
    scale = np.maximum(np.abs(A).max(axis=(1, 2)) ** 3, 1e-300)
    good = np.abs(det) > 1e-9 * scale
    x = np.empty_like(rhs)
    if good.any():
        x[good] = np.linalg.solve(A[good], rhs[good][..., None])[..., 0]
    cands = np.stack([fallback_a, fallback_b, (fallback_a + fallback_b) / 2.0], 1)
    errs = np.stack([_error(Q, cands[:, k]) for k in range(3)], 1)
    best = cands[np.arange(len(Q)), np.argmin(errs, 1)]
    return np.where(good[:, None], x, best)


def _error(Q, v):
    x, y, z = v[:, 0], v[:, 1], v[:, 2]
    return (Q[:, 0] * x * x + 2 * Q[:, 1] * x * y + 2 * Q[:, 2] * x * z + 2 * Q[:, 3] * x + Q[:, 4] * y * y
            + 2 * Q[:, 5] * y * z + 2 * Q[:, 6] * y + Q[:, 7] * z * z + 2 * Q[:, 8] * z + Q[:, 9])


def _edges(T):
    """Unique edges (E, 2) of triangles T and the edge of each triangle side (T, 3)."""
    he = np.stack([T, np.roll(T, -1, axis=1)], 2).reshape(-1, 2)
    lo, hi = np.minimum(he[:, 0], he[:, 1]), np.maximum(he[:, 0], he[:, 1])
    E, inv = np.unique(np.stack([lo, hi], 1), axis=0, return_inverse=True)
    return E, inv.reshape(-1, 3)


def _common_neighbours(E, cand, npts):
    """Number of points adjacent to both ends of each candidate edge."""
    a, b = E[:, 0], E[:, 1]
    src = np.r_[a, b]
    dst = np.r_[b, a]
    order = np.argsort(src, kind="stable")
    src, dst = src[order], dst[order]
    start = np.searchsorted(src, np.arange(npts + 1))
    ends = E[cand]
    pieces_e, pieces_n = [], []
    for k in range(2):
        p = ends[:, k]
        cnt = start[p + 1] - start[p]
        rep = np.repeat(np.arange(len(cand)), cnt)
        off = np.repeat(start[p], cnt) + (np.arange(cnt.sum()) - np.repeat(np.cumsum(cnt) - cnt, cnt))
        pieces_e.append(rep)
        pieces_n.append(dst[off])
    key = np.r_[pieces_e[0], pieces_e[1]] * (npts + 1) + np.r_[pieces_n[0], pieces_n[1]]
    uk, counts = np.unique(key, return_counts=True)
    return np.bincount((uk[counts == 2] // (npts + 1)).astype(np.int64), minlength=len(cand))


@node("polyreduce", inputs=["geo"], category="model", params=[
    Float("percentage", 25.0, min=0.0, max=100.0, doc="Keep this percentage of the triangles"),
    Int("target", 0, min=0, doc="Or this many triangles (overrides the percentage when > 0)"),
    Bool("keep_borders", True, doc="Hold open borders in place (stiff quadrics along them)"),
])
def polyreduce(geo, percentage, target, keep_borders):
    """Reduce the polygon count while keeping the shape (Houdini's PolyReduce, quadric error metrics). Point
    attributes blend along each collapsed edge (others follow the kept point); vertex attributes follow their
    corners. Output: triangles."""
    B.require_polygons(geo, "polyreduce")
    tris, tri_prim = fan_triangulate(geo.topology)
    vp = geo.topology.vertex_point.astype(np.int64)
    T = vp[tris]
    want = int(target) if target > 0 else int(np.ceil(len(T) * percentage / 100.0))
    P = geo.P.astype(np.float64)
    npts = geo.npoints
    keep_t = np.ones(len(T), bool)
    corner_src = tris.copy()                               # the input vertex each triangle corner came from
    tq = _plane_quadrics(P, T)
    Q = np.stack([np.bincount(T.ravel(), weights=np.repeat(tq[:, k], 3), minlength=npts) for k in range(10)], 1)
    if keep_borders:
        E, side = _edges(T)
        cnt = np.bincount(side.ravel(), minlength=len(E))
        bt, bs = np.nonzero(cnt[side] == 1)                # border sides: a plane through the edge, normal to the face
        a, b = T[bt, bs], T[bt, (bs + 1) % 3]
        fn = np.cross(P[T[bt, 1]] - P[T[bt, 0]], P[T[bt, 2]] - P[T[bt, 0]])
        e = P[b] - P[a]
        n = np.cross(e, fn)
        n = n / np.maximum(np.linalg.norm(n, axis=1, keepdims=True), 1e-300)
        w = np.maximum(np.einsum("ij,ij->i", e, e), 1e-30) * 1e3
        bq = _quadric(n, -np.einsum("ij,ij->i", n, P[a]), w)
        for k in range(10):
            Q[:, k] += np.bincount(np.r_[a, b], weights=np.r_[bq[:, k], bq[:, k]], minlength=npts)
    blocked = np.zeros(0, np.int64)                         # edges (a * npts + b) whose collapse was refused
    progressed = False                                      # collapses since blocked was last cleared
    widen = 1                                               # stuck: let costlier edges in, 4x more each time
    floats = {n: rows(a).astype(np.float64) for n, a in geo.point.items() if n != "P" and a.dtype.kind == "f"}
    for _ in range(2000):
        live = np.flatnonzero(keep_t)
        if len(live) <= want:
            break
        Tl = T[live]
        E, side = _edges(Tl)
        if not len(E):
            break
        border_edge = np.bincount(side.ravel(), minlength=len(E)) == 1
        on_border = np.zeros(npts, bool)
        on_border[E[border_edge].ravel()] = True
        Qe = Q[E[:, 0]] + Q[E[:, 1]]
        v = _solve(Qe, P[E[:, 0]], P[E[:, 1]])
        cost = np.maximum(_error(Qe, v), 0.0)
        ekey = E[:, 0] * npts + E[:, 1]
        cost[np.isin(ekey, blocked)] = np.inf                 # refused before: let the neighbours go first
        # cheapest edge at both ends -> a matching. Ties (flat regions cost nothing) break by a hash of the edge:
        # breaking them by index would only let local-minimum points collapse, a handful per round
        tie = ((E[:, 0] * 0x9E3779B1 + E[:, 1] * 0x85EBCA77) & 0xFFFFFFFF) ^ (E[:, 0] >> 3)
        rank = np.empty(len(E), np.int64)
        rank[np.lexsort((tie, np.round(cost, 12)))] = np.arange(len(E))
        best = np.full(npts, np.iinfo(np.int64).max)
        np.minimum.at(best, E[:, 0], rank)
        np.minimum.at(best, E[:, 1], rank)
        # never more than needed, and only among the globally cheapest edges: costly collapses wait until the cheap
        # ones are done (or refused), as in the one-at-a-time algorithm
        need = max((len(live) - want + 1) // 2, 1)          # each collapse removes ~2 triangles
        fin = cost[np.isfinite(cost)]
        k = min(2 * need * widen, len(fin))
        thresh = np.partition(fin, k - 1)[k - 1] if k else -1.0
        cand = np.flatnonzero((best[E[:, 0]] == rank) & (best[E[:, 1]] == rank) & (cost <= thresh * (1 + 1e-9)))
        if not len(cand):
            if progressed and len(blocked):                 # the mesh changed since: refused edges may work now
                blocked, progressed = np.zeros(0, np.int64), False
                continue
            if k < len(fin):
                widen *= 4
                continue
            break
        cand = cand[np.argsort(cost[cand], kind="stable")[:need]]
        # link condition: interior edges share exactly 2 neighbours, border edges 1; an edge joining two border
        # points across the inside would pinch the mesh
        common = _common_neighbours(E, cand, npts)
        bd = border_edge[cand]
        ok = np.where(bd, common == 1, common == 2) & (bd | ~(on_border[E[cand, 0]] & on_border[E[cand, 1]]))
        blocked = np.union1d(blocked, ekey[cand[~ok]])
        cand = cand[ok]
        if not len(cand):
            continue
        # apply the moves, reject collapses whose surrounding triangles flip or degenerate, and check again with the
        # survivors only (a triangle shared with a rejected collapse has changed) until nothing more is rejected
        n_old = np.cross(P[Tl[:, 1]] - P[Tl[:, 0]], P[Tl[:, 2]] - P[Tl[:, 0]])
        while len(cand):
            newP = P.copy()
            to = np.arange(npts)
            newP[E[cand, 0]] = v[cand]
            newP[E[cand, 1]] = v[cand]
            to[E[cand, 1]] = E[cand, 0]
            owner = np.full(npts, -1, np.int64)
            owner[E[cand, 0]] = np.arange(len(cand))
            owner[E[cand, 1]] = np.arange(len(cand))
            Tn = to[Tl]
            dies = (Tn[:, 0] == Tn[:, 1]) | (Tn[:, 1] == Tn[:, 2]) | (Tn[:, 0] == Tn[:, 2])
            n_new = np.cross(newP[Tn[:, 1]] - newP[Tn[:, 0]], newP[Tn[:, 2]] - newP[Tn[:, 0]])
            lo_ = np.linalg.norm(n_old, axis=1) * np.linalg.norm(n_new, axis=1)
            bad = ~dies & (np.einsum("ij,ij->i", n_old, n_new) <= 0.2 * lo_)
            reject = np.zeros(len(cand), bool)
            for k in range(3):
                o = owner[Tl[bad, k]]
                reject[o[o >= 0]] = True
            if not reject.any():
                break
            blocked = np.union1d(blocked, ekey[cand[reject]])
            cand = cand[~reject]
        if not len(cand):
            continue
        a, b = E[cand, 0], E[cand, 1]
        e = P[b] - P[a]                                     # float attributes: blend by where v lies along the edge
        t = np.clip(np.einsum("ij,ij->i", v[cand] - P[a], e) / np.maximum(np.einsum("ij,ij->i", e, e), 1e-300), 0, 1)
        for arr in floats.values():
            arr[a] = arr[a] * (1.0 - t[:, None]) + arr[b] * t[:, None]
        P[a] = v[cand]
        Q[a] = Q[a] + Q[b]
        to = np.arange(npts)
        to[b] = a
        T[live] = to[Tl]
        dies = (T[live, 0] == T[live, 1]) | (T[live, 1] == T[live, 2]) | (T[live, 0] == T[live, 2])
        keep_t[live[dies]] = False
        progressed, widen = True, 1
    live = np.flatnonzero(keep_t)
    T = T[live]
    used = np.zeros(npts, bool)
    used[T.ravel()] = True
    new_index = np.cumsum(used) - 1
    point = B.gather(geo.point, np.flatnonzero(used))
    point["P"] = P[used].astype(np.float32)
    for n, arr in floats.items():
        val = arr[used]
        if n == "N":
            val = val / np.maximum(np.linalg.norm(val, axis=1, keepdims=True), 1e-30)
        point[n] = val.astype(np.float32).reshape((len(val),) + geo.point[n].shape[1:])
    out = B.make(point["P"], new_index[T].ravel(), np.arange(len(T) + 1) * 3, point,
                 B.gather(geo.vertex, corner_src[live].ravel()), B.gather(geo.prim, tri_prim[live]), geo.detail)
    return out
