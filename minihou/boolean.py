"""Mesh booleans on closed triangle meshes: union, intersection, difference (A - B).

1. **Symbolic intersection points.** Every point where the surfaces cross is where an edge of one mesh pierces a
   triangle of the other, so it is named by that (mesh, edge, triangle) and computed once, from the edge's canonical
   direction. Both triangles sharing the edge, and the triangle it pierces, therefore use the *same* point: the result
   is watertight by construction, with no welding by distance.
2. **Segments.** Each pair of crossing triangles (found through a grid of bounding boxes) contributes one segment,
   the overlap of the two triangles' intervals on the line where their planes meet. Its ends are two such points.
3. **Re-triangulation.** Each cut triangle is worked on in its own barycentric coordinates: its boundary (corners plus
   the points on its edges) is split by every chain of segments that runs from edge to edge, closed loops become
   holes bridged into their polygon, and the pieces are ear-clipped. Barycentric coordinates also interpolate vertex
   attributes exactly.
4. **Classification.** Triangles of one mesh joined across edges that are not cuts form regions; one ray-parity test
   (:func:`minihou.raycast.inside`) per region against the other mesh decides inside or outside.
5. **Selection.** union = A outside + B outside; intersect = A inside + B inside; subtract = A outside + B inside
   reversed.

Exactly coplanar or touching surfaces have no well-defined crossing. B is therefore grown by a tiny epsilon along its
point normals first: flush faces then cut cleanly (a subtract removes the flush skin, a union merges touching faces)
and no point of one mesh lies exactly on the other.
"""
from __future__ import annotations

from typing import Dict, List, Tuple

import numpy as np

from . import spatial


class BooleanError(ValueError):
    pass


# -- broad phase ------------------------------------------------------------------------------------------------
def candidate_pairs(PA, TA, PB, TB):
    """Pairs (i, j) of triangles of A and B whose bounding boxes overlap (uniform grid over both)."""
    la, ha = PA[TA].min(1), PA[TA].max(1)
    lb, hb = PB[TB].min(1), PB[TB].max(1)
    lo = np.minimum(la.min(0), lb.min(0))
    ext = np.maximum(np.maximum(ha.max(0), hb.max(0)) - lo, 1e-12)
    size = np.concatenate([ha - la, hb - lb]).mean(0).max() * 2.0
    size = max(size, float(ext.max()) / 256.0, 1e-12)
    dims = np.maximum(np.ceil(ext / size).astype(np.int64), 1)

    def cells(lo_, hi_):
        a = np.clip(((lo_ - lo) / size).astype(np.int64), 0, dims - 1)
        b = np.clip(((hi_ - lo) / size).astype(np.int64), 0, dims - 1)
        span = b - a + 1
        cnt = span.prod(1)
        owner = np.repeat(np.arange(len(a)), cnt)
        k = np.arange(cnt.sum()) - np.repeat(np.cumsum(cnt) - cnt, cnt)
        sx, sy = span[owner, 0], span[owner, 1]
        cx = a[owner, 0] + k % sx
        cy = a[owner, 1] + (k // sx) % sy
        cz = a[owner, 2] + k // (sx * sy)
        return owner, (cx * dims[1] + cy) * dims[2] + cz

    oa, ca = cells(la, ha)
    ob, cb = cells(lb, hb)
    order = np.argsort(cb, kind="stable")
    cb, ob = cb[order], ob[order]
    left = np.searchsorted(cb, ca, "left")
    right = np.searchsorted(cb, ca, "right")
    cnt = right - left
    i = np.repeat(oa, cnt)
    j = ob[np.repeat(left, cnt) + (np.arange(cnt.sum()) - np.repeat(np.cumsum(cnt) - cnt, cnt))]
    if not len(i):
        return i, j
    pair = np.unique(np.stack([i, j], 1), axis=0)
    i, j = pair[:, 0], pair[:, 1]
    ok = np.all((la[i] <= hb[j]) & (lb[j] <= ha[i]), axis=1)
    return i[ok], j[ok]


# -- narrow phase -----------------------------------------------------------------------------------------------
def _planes(P, T):
    n = np.cross(P[T[:, 1]] - P[T[:, 0]], P[T[:, 2]] - P[T[:, 0]])
    n = n / np.maximum(np.linalg.norm(n, axis=1, keepdims=True), 1e-300)
    return n, -np.einsum("ij,ij->i", n, P[T[:, 0]])


def _crossing_edges(d):
    """For signed distances d (m, 3) of a triangle's corners with one corner alone on its side: the two edges (as
    corner pairs) that cross the plane. Returns (k, a1, b1, a2, b2): the lone corner and both edges' corners."""
    s = np.sign(d)
    lone = np.where(s[:, 0] == s[:, 1], 2, np.where(s[:, 0] == s[:, 2], 1, 0))
    o1, o2 = (lone + 1) % 3, (lone + 2) % 3
    return lone, o1, o2


def intersect(PA, TA, PB, TB, ia, jb):
    """Segments between crossing triangle pairs. Returns, per segment: (tri of A, tri of B, key1, key2) where a key
    is (side, u, v, face): the edge u-v (u < v) of mesh `side` (0 = A, 1 = B) piercing triangle `face` of the other.
    Also the coordinates are computed later from the keys."""
    nA, dA = _planes(PA, TA)
    nB, dB = _planes(PB, TB)
    # distances of each triangle's corners to the other's plane
    distA = np.einsum("mkj,mj->mk", PA[TA[ia]], nB[jb]) + dB[jb][:, None]      # A corners vs B plane
    distB = np.einsum("mkj,mj->mk", PB[TB[jb]], nA[ia]) + dA[ia][:, None]      # B corners vs A plane
    cross = ~(np.all(distA > 0, 1) | np.all(distA < 0, 1) | np.all(distB > 0, 1) | np.all(distB < 0, 1))
    if np.any(cross & (np.any(distA == 0, 1) | np.any(distB == 0, 1))):
        raise BooleanError("a point lies exactly on the other surface; move one input very slightly")
    ia, jb, distA, distB = ia[cross], jb[cross], distA[cross], distB[cross]
    m = len(ia)
    if not m:
        return ia, jb, np.zeros((0, 4), np.int64), np.zeros((0, 4), np.int64)
    D = np.cross(nA[ia], nB[jb])

    def interval(P, T, tri, dist, side, other):
        lone, o1, o2 = _crossing_edges(dist)
        r = np.arange(len(tri))
        v = T[tri]
        pl, p1, p2 = v[r, lone], v[r, o1], v[r, o2]
        dl, d1, d2 = dist[r, lone], dist[r, o1], dist[r, o2]
        x1 = P[pl] + (dl / (dl - d1))[:, None] * (P[p1] - P[pl])
        x2 = P[pl] + (dl / (dl - d2))[:, None] * (P[p2] - P[pl])
        t1, t2 = np.einsum("ij,ij->i", x1, D), np.einsum("ij,ij->i", x2, D)
        k1 = np.stack([np.full(len(r), side), np.minimum(pl, p1), np.maximum(pl, p1), other], 1)
        k2 = np.stack([np.full(len(r), side), np.minimum(pl, p2), np.maximum(pl, p2), other], 1)
        swap = t1 > t2
        lo_t, hi_t = np.where(swap, t2, t1), np.where(swap, t1, t2)
        lo_k, hi_k = np.where(swap[:, None], k2, k1), np.where(swap[:, None], k1, k2)
        return lo_t, hi_t, lo_k, hi_k

    a0, a1, ak0, ak1 = interval(PA, TA, ia, distA, 0, jb)
    b0, b1, bk0, bk1 = interval(PB, TB, jb, distB, 1, ia)
    start = np.where((a0 >= b0)[:, None], ak0, bk0)
    end = np.where((a1 <= b1)[:, None], ak1, bk1)
    ok = np.maximum(a0, b0) < np.minimum(a1, b1)
    return ia[ok], jb[ok], start[ok], end[ok]


def point_coords(keys, PA, TA, PB, TB):
    """Coordinates of symbolic intersection points (side, u, v, face) and the edge parameter t (from u to v)."""
    keys = np.asarray(keys, np.int64).reshape(-1, 4)
    out = np.zeros((len(keys), 3))
    t = np.zeros(len(keys))
    for side, (P, T, Po, To) in enumerate(((PA, TA, PB, TB), (PB, TB, PA, TA))):
        m = keys[:, 0] == side
        if not m.any():
            continue
        u, v, f = keys[m, 1], keys[m, 2], keys[m, 3]
        n, d = _planes(Po, To[f])
        du = np.einsum("ij,ij->i", P[u], n) + d
        dv = np.einsum("ij,ij->i", P[v], n) + d
        tt = du / (du - dv)
        out[m] = P[u] + tt[:, None] * (P[v] - P[u])
        t[m] = tt
    return out, t


# -- re-triangulation -------------------------------------------------------------------------------------------
def _cross2(a, b, c):
    return (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])


def _area2(poly, xy):
    s = 0.0
    for k in range(len(poly)):
        a, b = xy[poly[k - 1]], xy[poly[k]]
        s += a[0] * b[1] - b[0] * a[1]
    return s


def _inside_poly(p, poly, xy):
    c = False
    for k in range(len(poly)):
        a, b = xy[poly[k - 1]], xy[poly[k]]
        if (a[1] > p[1]) != (b[1] > p[1]):
            x = a[0] + (p[1] - a[1]) * (b[0] - a[0]) / (b[1] - a[1])
            if x > p[0]:
                c = not c
    return c


def earclip(poly, xy) -> List[Tuple[int, int, int]]:
    """Triangulate a counter-clockwise simple polygon given as indices into xy. Repeated indices (bridges to holes)
    are allowed: containment tests skip points sharing an index or position with the ear's corners. Of the first
    few valid ears, the best shaped is cut each time (fewer slivers than the first one found)."""
    pts = {i: (float(xy[i][0]), float(xy[i][1])) for i in poly}
    idx = list(poly)
    tris = []
    guard = 0
    while len(idx) > 3 and guard < 4 * len(poly) + 16:
        guard += 1
        m = len(idx)
        best, found = None, 0
        for k in range(m):
            i0, i1, i2 = idx[k - 1], idx[k], idx[(k + 1) % m]
            (ax, ay), (bx, by), (cx, cy) = pts[i0], pts[i1], pts[i2]
            area = (bx - ax) * (cy - ay) - (by - ay) * (cx - ax)
            if area <= 0:
                continue
            bad = False
            for j in idx:
                if j == i0 or j == i1 or j == i2:
                    continue
                px, py = pts[j]
                if (px, py) in ((ax, ay), (bx, by), (cx, cy)):
                    continue
                if ((bx - ax) * (py - ay) - (by - ay) * (px - ax) >= 0 and
                        (cx - bx) * (py - by) - (cy - by) * (px - bx) >= 0 and
                        (ax - cx) * (py - cy) - (ay - cy) * (px - cx) >= 0):
                    bad = True
                    break
            if bad:
                continue
            e = max((bx - ax) ** 2 + (by - ay) ** 2, (cx - bx) ** 2 + (cy - by) ** 2, (ax - cx) ** 2 + (ay - cy) ** 2)
            q = area / max(e, 1e-300)
            if best is None or q > best[0]:
                best = (q, k)
            found += 1
            if found >= 8:
                break
        if best is None:
            break
        k = best[1]
        tris.append((idx[k - 1], idx[k], idx[(k + 1) % m]))
        del idx[k]
    if len(idx) == 3:
        tris.append(tuple(idx))
    elif len(idx) > 3:                                     # degenerate leftovers: fan (rare; keeps the mesh closed)
        for k in range(1, len(idx) - 1):
            tris.append((idx[0], idx[k], idx[k + 1]))
    return tris


def _eberly(poly, h, xy):
    """Position in the CCW polygon of a point visible from hole point h (the hole's rightmost point): cast a ray
    along +x to the nearest edge and take its right end, or the reflex vertex inside the triangle they span that is
    closest in angle to the ray (Eberly; as in earcut). None if the ray meets nothing."""
    hx, hy = float(xy[h][0]), float(xy[h][1])
    n = len(poly)
    best, edge, ix = np.inf, None, None
    for k in range(n):
        a, b = xy[poly[k]], xy[poly[(k + 1) % n]]
        if (a[1] > hy) != (b[1] > hy):
            x = a[0] + (hy - a[1]) * (b[0] - a[0]) / (b[1] - a[1])
            if hx <= x < hx + best:
                best, edge, ix = x - hx, k, x
    if edge is None:
        return None
    k0, k1 = edge, (edge + 1) % n
    ppos = k0 if xy[poly[k0]][0] >= xy[poly[k1]][0] else k1
    px, py = float(xy[poly[ppos]][0]), float(xy[poly[ppos]][1])
    if (px, py) != (ix, hy):
        tri = [(hx, hy), (ix, hy), (px, py)]
        if _cross2(*tri) < 0:
            tri = [tri[0], tri[2], tri[1]]
        best_key = None
        for j in range(n):
            v = poly[j]
            vx, vy = float(xy[v][0]), float(xy[v][1])
            if _cross2(xy[poly[j - 1]], xy[v], xy[poly[(j + 1) % n]]) >= 0 or (vx, vy) == (hx, hy):
                continue                                    # only reflex vertices can block the view
            if (_cross2(tri[0], tri[1], (vx, vy)) > 0 and _cross2(tri[1], tri[2], (vx, vy)) > 0 and
                    _cross2(tri[2], tri[0], (vx, vy)) > 0):
                dx, dy = vx - hx, vy - hy
                key = (abs(dy) / max(np.hypot(dx, dy), 1e-300), dx * dx + dy * dy)
                if best_key is None or key < best_key:
                    best_key, ppos = key, j
    return ppos


def _crosses(p, q, a, b):
    """Segments p-q and a-b cross at a point inside both."""
    d1, d2 = _cross2(p, q, a), _cross2(p, q, b)
    d3, d4 = _cross2(a, b, p), _cross2(a, b, q)
    return d1 * d2 < 0 and d3 * d4 < 0


def _two_bridges(poly, hole, xy, d):
    """Bridges along +d and -d (unit 2-D direction) from the hole's extreme points in those directions: (target
    position along +d, target position along -d, hole index of the +d end, hole index of the -d end) or None."""
    R = np.array([[d[0], -d[1]], [d[1], d[0]]])           # rotates d onto +x (keeps orientation)
    rxy = xy @ R
    n, m = len(poly), len(hole)
    hr = max(range(m), key=lambda k: (rxy[hole[k]][0], -rxy[hole[k]][1]))
    hl = min(range(m), key=lambda k: (rxy[hole[k]][0], -rxy[hole[k]][1]))
    pr = _eberly(poly, hole[hr], rxy)
    pl = _eberly(poly[::-1], hole[hl], rxy * np.array([-1.0, 1.0]))
    if pr is None or pl is None or hl == hr:
        return None
    pl = n - 1 - pl
    return (pr, pl, hr, hl) if pl != pr else None


_DIRECTIONS = [(1.0, 0.0), (0.0, 1.0), (0.7071067811865476, 0.7071067811865476),
               (0.7071067811865476, -0.7071067811865476), (0.9238795325112867, 0.3826834323650898),
               (0.3826834323650898, -0.9238795325112867)]


def holes_into(poly, holes, xy):
    """Simple polygons covering the CCW polygon minus the CCW holes inside it. Each hole is cut out with two bridges
    in opposite directions from its extreme points (Eberly's visibility test for each), which leaves two simple
    polygons and no slits; directions are tried until neither bridge meets a hole still to come. If none works the
    remaining holes are joined with right bridges only (zero-width slits, as in earcut; safe in decreasing max x)
    and that polygon is ear-clipped into triangles."""
    holes = sorted(holes, key=lambda h: -max(xy[i][0] for i in h))
    pieces = [list(poly)]
    for hi_, hole in enumerate(holes):
        host = next((k for k, q in enumerate(pieces) if _inside_poly(xy[hole[0]], q, xy)), None)
        if host is None:
            raise BooleanError("a hole lies outside its polygon")
        q = pieces[host]
        later = [e for h2 in holes[hi_ + 1:] for e in zip(h2, h2[1:] + h2[:1])]
        done = False
        for d in _DIRECTIONS:
            br = _two_bridges(q, hole, xy, d)
            if br is None:
                continue
            pr, pl, hr, hl = br
            if any(_crosses(xy[h], xy[t], xy[a], xy[b_]) for (h, t) in ((hole[hr], q[pr]), (hole[hl], q[pl]))
                   for (a, b_) in later):
                continue
            n, m = len(q), len(hole)
            piece1 = [q[(pr + k) % n] for k in range(((pl - pr) % n) + 1)] + \
                     [hole[(hl - k) % m] for k in range(((hl - hr) % m) + 1)]
            piece2 = [q[(pl + k) % n] for k in range(((pr - pl) % n) + 1)] + \
                     [hole[(hr - k) % m] for k in range(((hr - hl) % m) + 1)]
            a1, a2, aq, ah = _area2(piece1, xy), _area2(piece2, xy), _area2(q, xy), _area2(hole, xy)
            if a1 > 0 and a2 > 0 and abs(a1 + a2 - (aq - ah)) <= 1e-9 * abs(aq):
                pieces[host] = piece1
                pieces.append(piece2)
                done = True
                break
        if done:
            continue
        # fallback: this hole and every later one in the same piece go in through right bridges, then ear-clip
        rest = [h2 for h2 in holes[hi_:] if _inside_poly(xy[h2[0]], q, xy)]
        others = [h2 for h2 in holes[hi_ + 1:] if h2 not in rest]
        slit = list(q)
        for h2 in rest:                                     # already in decreasing max x
            m = len(h2)
            hr2 = max(range(m), key=lambda k: (xy[h2[k]][0], -xy[h2[k]][1]))
            pr2 = _eberly(slit, h2[hr2], xy)
            if pr2 is None:
                raise BooleanError("could not bridge a hole in a cut triangle")
            slit = slit[:pr2 + 1] + [h2[(hr2 - k) % m] for k in range(m)] + [h2[hr2], slit[pr2]] + slit[pr2 + 1:]
        pieces[host:host + 1] = [list(t) for t in earclip(slit, xy)]
        for h2 in others:                                   # holes of other pieces: carry on with them
            pieces = _place_in_any(pieces, h2, xy)
        return pieces
    return pieces


def _place_in_any(pieces, hole, xy):
    for k, q in enumerate(pieces):
        if _inside_poly(xy[hole[0]], q, xy):
            return pieces[:k] + holes_into(q, [hole], xy) + pieces[k + 1:]
    raise BooleanError("a hole lies outside its polygon")


def split_triangle(xy, boundary, segs):
    """Polygons covering the triangle whose CCW boundary (corners and on-edge points, as indices into xy) is cut by
    segments (pairs of indices): chains from edge to edge split polygons, closed loops become holes (cut out of their
    polygon) and polygons of their own. Points on the boundary have one segment; interior points two."""
    adj: Dict[int, List[int]] = {}
    for a, b in segs:
        adj.setdefault(a, []).append(b)
        adj.setdefault(b, []).append(a)
    on_boundary = set(boundary)
    seen = set()
    chains, loops = [], []
    for start in adj:
        if start in on_boundary and start not in seen and len(adj[start]) == 1:
            chain, prev, cur = [start], None, start
            seen.add(start)
            while True:
                nxt = [n for n in adj[cur] if n != prev]
                if not nxt:
                    break
                prev, cur = cur, nxt[0]
                chain.append(cur)
                seen.add(cur)
                if cur in on_boundary:
                    break
            chains.append(chain)
    for start in adj:
        if start in seen:
            continue
        loop, prev, cur = [start], None, start
        seen.add(start)
        while True:
            nxt = [n for n in adj[cur] if n != prev and (n not in seen or n == start)]
            if not nxt:
                break
            prev, cur = cur, nxt[0]
            if cur == start:
                break
            loop.append(cur)
            seen.add(cur)
        if len(loop) >= 3:
            loops.append(loop)
    polys = [list(boundary)]
    for ch in chains:
        p, q = ch[0], ch[-1]
        for k, poly in enumerate(polys):
            if p in poly and q in poly:
                r = poly.index(p)
                poly = poly[r:] + poly[:r]
                s = poly.index(q)
                inner = ch[1:-1]
                polys[k] = poly[:s + 1] + inner[::-1]
                polys.append(poly[s:] + [p] + inner)
                break
        else:
            raise BooleanError("could not place a cut inside its triangle")
    loops.sort(key=lambda lp: -abs(_area2(lp, xy)))
    holes_of: Dict[int, list] = {}
    for lp in loops:
        if _area2(lp, xy) < 0:
            lp = lp[::-1]
        c = xy[lp[0]]
        host = None
        for k, poly in enumerate(polys):
            if _inside_poly(c, poly, xy):
                if host is None or abs(_area2(poly, xy)) < abs(_area2(polys[host], xy)):
                    host = k
        if host is None:
            raise BooleanError("a closed cut lies outside its triangle")
        holes_of.setdefault(host, []).append(list(lp))
        polys.append(list(lp))                              # the region inside the loop (may hold loops itself)
    out = []
    for k, poly in enumerate(polys):
        out.extend(holes_into(poly, holes_of[k], xy) if k in holes_of else [poly])
    return out


def inner_point(Q):
    """A point strictly inside the planar polygon with corners Q (k, 3): the centroid of one of its ears."""
    n = np.cross(Q - Q.mean(0), np.roll(Q, -1, 0) - Q.mean(0)).sum(0)
    n = n / max(np.linalg.norm(n), 1e-300)
    u = np.cross(n, [1.0, 0, 0] if abs(n[0]) < 0.9 else [0, 1.0, 0])
    u /= np.linalg.norm(u)
    xy = np.stack([Q @ u, Q @ np.cross(n, u)], 1)
    tri = earclip(list(range(len(Q))), xy)
    if not tri:
        return Q.mean(0)
    a = [abs(_cross2(xy[i], xy[j], xy[k])) for i, j, k in tri]
    i, j, k = tri[int(np.argmax(a))]
    return (Q[i] + Q[j] + Q[k]) / 3.0


# -- the whole operation ----------------------------------------------------------------------------------------
def boolean(PA, TA, PB, TB, op: str, weld: float = 0.0):
    """Boolean of two closed, consistently wound triangle meshes. Returns a dict: P (A's points, then B's, then the
    intersection points); polygons as `verts` (flat point ids) and `sizes`; per polygon `owner` (0/1: which mesh),
    `src` (source triangle: A's, then B's numbering) and `whole` (an uncut source triangle); `bary` (barycentric
    weights of every corner in its source triangle); `keys` (side, u, v, face) and `edge_t` (parameter along edge
    u-v) of the intersection points; `remap` (where each point was welded to, when ``weld`` merges points closer than
    that)."""
    PA, PB = np.asarray(PA, np.float64), np.asarray(PB, np.float64)
    TA, TB = np.asarray(TA, np.int64).reshape(-1, 3), np.asarray(TB, np.int64).reshape(-1, 3)
    nA, nB = len(PA), len(PB)
    ia, jb = candidate_pairs(PA, TA, PB, TB) if len(TA) and len(TB) else (np.zeros(0, np.int64),) * 2
    ia, jb, k1, k2 = intersect(PA, TA, PB, TB, ia, jb)
    keys, inv = (np.unique(np.concatenate([k1, k2]), axis=0, return_inverse=True) if len(k1)
                 else (np.zeros((0, 4), np.int64), np.zeros(0, np.int64)))
    inv = inv.reshape(-1)
    s1, s2 = inv[:len(k1)], inv[len(k1):]
    X, edge_t = point_coords(keys, PA, TA, PB, TB)
    P = np.concatenate([PA, PB, X])
    xid = nA + nB + np.arange(len(keys))                     # global point id of each intersection point
    polys, owner, src, whole, bary = [], [], [], [], []
    cut_edges = set()
    for side, (T, tri_of_seg, base) in enumerate(((TA, ia, 0), (TB, jb, nA))):
        if not len(T):
            continue
        cut = np.zeros(len(T), bool)
        cut[tri_of_seg] = True
        for t in np.flatnonzero(~cut):
            polys.append(T[t] + base)
            bary.append(np.eye(3))
        keep = np.flatnonzero(~cut)
        owner.extend([side] * len(keep))
        src.extend((keep + (0 if side == 0 else len(TA))).tolist())
        whole.extend([True] * len(keep))
        order = np.argsort(tri_of_seg, kind="stable")
        bounds = np.searchsorted(tri_of_seg[order], np.arange(len(T) + 1))
        for t in np.flatnonzero(cut):
            segs_k = order[bounds[t]:bounds[t + 1]]
            corners = T[t]
            c3 = P[corners + base]
            e1, e2 = c3[1] - c3[0], c3[2] - c3[0]
            G = np.array([[e1 @ e1, e1 @ e2], [e1 @ e2, e2 @ e2]])
            if np.linalg.det(G) <= 1e-24 * max(float(G.max()), 1e-300) ** 2:
                raise BooleanError("an input has a zero-area face where the surfaces cross; clean it first")
            ids = {}                                        # global point id -> local index
            xy = [np.array([0.0, 0.0]), np.array([1.0, 0.0]), np.array([0.0, 1.0])]
            for c in range(3):
                ids[int(corners[c] + base)] = c
            edge_pts = {0: [], 1: [], 2: []}
            segs = []
            for sg in segs_k:
                pair = []
                for kk in (s1[sg], s2[sg]):
                    g = int(xid[kk])
                    if g not in ids:
                        q = P[g] - c3[0]
                        ids[g] = len(xy)
                        xy.append(np.linalg.solve(G, np.array([q @ e1, q @ e2])))
                        key = keys[kk]
                        if key[0] == side:                  # on one of this triangle's edges
                            for e in range(3):
                                if {int(corners[e]), int(corners[(e + 1) % 3])} == {int(key[1]), int(key[2])}:
                                    edge_pts[e].append(ids[g])
                    pair.append(ids[g])
                segs.append((pair[0], pair[1]))
                cut_edges.add((min(s1[sg], s2[sg]), max(s1[sg], s2[sg])))
            xy = np.array(xy)
            boundary = []
            for e in range(3):
                boundary.append(e)
                a, d = xy[e], xy[(e + 1) % 3] - xy[e]
                boundary.extend(sorted(edge_pts[e], key=lambda i: float((xy[i] - a) @ d)))
            glob = np.empty(len(xy), np.int64)
            for g, li in ids.items():
                glob[li] = g
            w = np.stack([1.0 - xy[:, 0] - xy[:, 1], xy[:, 0], xy[:, 1]], 1)
            for poly in split_triangle(xy, boundary, segs):
                polys.append(glob[poly])
                bary.append(w[poly])
                owner.append(side)
                src.append(t + (0 if side == 0 else len(TA)))
                whole.append(False)
    sizes = np.array([len(q) for q in polys], np.int64)
    verts = np.concatenate(polys) if polys else np.zeros(0, np.int64)
    owner, src, whole = np.array(owner, np.int64), np.array(src, np.int64), np.array(whole, bool)
    bary = np.concatenate(bary) if bary else np.zeros((0, 3))
    # regions: polygons of one mesh joined across edges that are not cuts
    npoly = len(sizes)
    first = np.cumsum(sizes) - sizes
    poly_of = np.repeat(np.arange(npoly), sizes)
    local = np.arange(len(verts)) - first[poly_of]
    nxt = np.where(local == sizes[poly_of] - 1, first[poly_of], np.arange(len(verts)) + 1)
    lo, hi = np.minimum(verts, verts[nxt]), np.maximum(verts, verts[nxt])
    N1 = len(P) + 1
    is_cut = np.zeros(len(verts), bool)
    if cut_edges:
        cuts = np.array(sorted(cut_edges), np.int64).reshape(-1, 2)
        is_cut = np.isin(lo * N1 + hi, np.minimum(xid[cuts[:, 0]], xid[cuts[:, 1]]) * N1
                         + np.maximum(xid[cuts[:, 0]], xid[cuts[:, 1]]))
    ekey = (owner[poly_of] * N1 + lo) * N1 + hi
    order = np.argsort(ekey, kind="stable")
    same = np.flatnonzero((ekey[order][1:] == ekey[order][:-1]) & ~is_cut[order][1:])
    region = spatial.components(npoly, poly_of[order][same], poly_of[order][same + 1])
    # one inside test per region, at a point inside its largest polygon
    from .raycast import TriGrid, inside
    cr = np.cross(P[verts], P[verts[nxt]])
    area = np.linalg.norm(np.stack([np.bincount(poly_of, weights=cr[:, k], minlength=npoly) for k in range(3)], 1),
                          axis=1)
    nreg = int(region.max()) + 1 if npoly else 0
    rep = np.full(nreg, -1, np.int64)
    best = np.full(nreg, -1.0)
    for r_, pid in zip(region.tolist(), range(npoly)):
        if area[pid] > best[r_]:
            best[r_], rep[r_] = area[pid], pid
    is_in = np.zeros(nreg, bool)
    scale = float(np.ptp(P, axis=0).max()) if len(P) else 1.0
    for side, (Po, To) in enumerate(((PB, TB), (PA, TA))):
        regs = np.flatnonzero(owner[rep] == side) if nreg else np.zeros(0, np.int64)
        if not len(regs):
            continue
        samples = np.array([inner_point(P[verts[first[rep[r_]]:first[rep[r_]] + sizes[rep[r_]]]]) for r_ in regs])
        is_in[regs] = inside(TriGrid(Po[To]), samples, scale)
    pin = is_in[region]
    keep = np.where(owner == 0, {"union": ~pin, "intersect": pin, "subtract": ~pin}[op],
                    {"union": ~pin, "intersect": pin, "subtract": pin}[op])
    vkeep = keep[poly_of]
    flip = (owner == 1) & (op == "subtract")
    # reverse B's polygons for subtract (corner order and their weights)
    rev = np.arange(len(verts))
    fv = flip[poly_of]
    rev[fv] = (first[poly_of] + sizes[poly_of] - 1 - local)[fv]
    verts, bary = verts[rev], bary[rev]
    verts, bary = verts[vkeep], bary[vkeep]
    sizes, owner, src, whole = sizes[keep], owner[keep], src[keep], whole[keep]
    remap = np.arange(len(P))
    if weld > 0 and len(sizes):
        verts, sizes, owner, src, whole, bary, remap = _weld(P, verts, sizes, owner, src, whole, bary, weld,
                                                             nA + nB)
    return {"P": P, "verts": verts, "sizes": sizes, "owner": owner, "src": src, "whole": whole, "bary": bary,
            "flipped": (owner == 1) & (op == "subtract"), "keys": keys, "edge_t": edge_t, "remap": remap}


def _weld(P, verts, sizes, owner, src, whole, bary, tol, first_cut):
    """Merge used points closer than tol (to the lowest index: A's points, then B's, then cut points), then fold away
    zero-width spikes (an edge running back along the one before it, where a cut ran along an edge lying in the other
    surface), cleaning up after each pass."""
    used = np.unique(verts)
    i, j, _ = spatial.pairs_within(P[used], tol)
    lab = spatial.components(len(used), i, j)
    rep = np.full(int(lab.max()) + 1 if len(lab) else 0, len(P))
    np.minimum.at(rep, lab, used)
    remap = np.arange(len(P))
    remap[used] = rep[lab]
    for _ in range(16):
        verts, sizes, owner, src, whole, bary = _clean(P, remap[verts], sizes, owner, src, whole, bary)
        tips = _spikes(P, verts, sizes, first_cut)
        if not tips:
            break
        step = np.arange(len(P))
        for v, to in tips.items():
            step[v] = to
        for _ in range(8):                                  # follow chains of merges
            step = step[step]
        remap = step[remap]
    return verts, sizes, owner, src, whole, bary, remap


def _spikes(P, verts, sizes, first_cut):
    """Cut points (ids from first_cut on) where a polygon with real area turns back on itself along a line:
    {tip: the nearer neighbour to merge it into}. The inputs' own points never move."""
    first = np.cumsum(sizes) - sizes
    poly_of = np.repeat(np.arange(len(sizes)), sizes)
    local = np.arange(len(verts)) - first[poly_of]
    nxt = np.where(local == sizes[poly_of] - 1, first[poly_of], np.arange(len(verts)) + 1)
    prv = np.where(local == 0, first[poly_of] + sizes[poly_of] - 1, np.arange(len(verts)) - 1)
    a, v, b = P[verts[prv]], P[verts], P[verts[nxt]]
    e1, e2 = v - a, b - v
    l1, l2 = np.linalg.norm(e1, axis=1), np.linalg.norm(e2, axis=1)
    sin = np.linalg.norm(np.cross(e1, e2), axis=1) / np.maximum(l1 * l2, 1e-300)
    back = (np.einsum("ij,ij->i", e1, e2) < 0) & (sin < 1e-4) & (verts >= first_cut)
    cr = np.cross(P[verts], P[verts[nxt]])
    area = np.linalg.norm(np.stack([np.bincount(poly_of, weights=cr[:, k], minlength=len(sizes))
                                    for k in range(3)], 1), axis=1)
    longest = np.zeros(len(sizes))
    np.maximum.at(longest, poly_of, l2)
    back &= area[poly_of] > 1e-6 * longest[poly_of] ** 2    # not a zero-area filler between collinear points
    tips = {}
    for c in np.flatnonzero(back):
        tip = int(verts[c])
        if tip not in tips:
            tips[tip] = int(verts[prv[c]] if l1[c] <= l2[c] else verts[nxt[c]])
    return tips


def _clean(P, v, sizes, owner, src, whole, bary):
    """Drop repeated corners, split polygons that visit a point twice, drop those left with fewer than 3 corners and
    pairs of polygons on the same points (zero-thickness walls)."""
    first = np.cumsum(sizes) - sizes
    poly_of = np.repeat(np.arange(len(sizes)), sizes)
    local = np.arange(len(v)) - first[poly_of]
    nxt = np.where(local == sizes[poly_of] - 1, first[poly_of], np.arange(len(v)) + 1)
    keepv = v != v[nxt]
    ns = np.bincount(poly_of[keepv], minlength=len(sizes))
    whole = whole & (ns == sizes)
    v, bary, poly_of = v[keepv], bary[keepv], poly_of[keepv]
    # a polygon that now visits a point twice (it folded back on itself) splits there into simple loops
    pair = poly_of * (len(P) + 1) + v
    _, cnt = np.unique(pair, return_counts=True)
    pinched = np.unique(poly_of[np.isin(pair, np.unique(pair)[cnt > 1])]) if (cnt > 1).any() else np.zeros(0, int)
    first = np.cumsum(ns) - ns
    loops_v, loops_b, loops_of = [], [], []
    pinched_set = set(pinched.tolist())
    for q in range(len(sizes)):
        f, n = first[q], ns[q]
        if n < 3:
            continue
        if q not in pinched_set:
            loops_v.append(v[f:f + n])
            loops_b.append(bary[f:f + n])
            loops_of.append(q)
            continue
        stack = [list(range(f, f + n))]
        while stack:
            cur = stack.pop()
            seen_at = {}
            for k, ix in enumerate(cur):
                if v[ix] in seen_at:
                    i0 = seen_at[v[ix]]
                    stack.append(cur[i0:k])
                    stack.append(cur[:i0] + cur[k:])
                    break
                seen_at[v[ix]] = k
            else:
                if len(cur) >= 3:
                    loops_v.append(v[cur])
                    loops_b.append(bary[cur])
                    loops_of.append(q)
    loops_of = np.array(loops_of, np.int64)
    sizes = np.array([len(x) for x in loops_v], np.int64)
    v = np.concatenate(loops_v) if loops_v else np.zeros(0, np.int64)
    bary = np.concatenate(loops_b) if loops_b else np.zeros((0, 3))
    owner, src = owner[loops_of], src[loops_of]
    whole = whole[loops_of] & np.isin(loops_of, list(set(range(len(ns))) - pinched_set))
    # coincident polygons (same point set) come in opposite pairs: both go
    first = np.cumsum(sizes) - sizes
    sig = [tuple(sorted(v[f:f + n].tolist())) for f, n in zip(first, sizes)]
    count: Dict[tuple, int] = {}
    for sgn in sig:
        count[sgn] = count.get(sgn, 0) + 1
    single = np.array([count[sgn] == 1 for sgn in sig], bool)
    vk = np.repeat(single, sizes)
    return v[vk], sizes[single], owner[single], src[single], whole[single], bary[vk]
