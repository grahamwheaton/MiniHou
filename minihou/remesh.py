"""Field-aligned quad remeshing (an Instant-Meshes / QuadWild-style retopology, simplified, numpy only).

The reference tools for this job (QRemeshify) are thin wrappers around QuadWild's Bi-MDF cross-field solver, a large C++
codebase. This is a small, self-contained approximation of the same idea, built from the engine's own pieces:

1. **Orientation (cross) field** -- a 4-way rotationally-symmetric direction per point, aligned to the principal
   curvature direction and smoothed across the surface with parallel transport (:func:`cross_field`). This is what
   makes the output topology *follow* the shape rather than an axis-aligned voxel grid.
2. **Resampling** -- target-spaced samples over the surface by area-weighted Poisson-disk sampling plus a few
   Lloyd/CVT relaxation steps projected back onto the input (:func:`sample_surface`).
3. **Connectivity** -- the samples are triangulated by ball pivoting / advancing front, which grows a manifold surface
   over the front of accepted faces and closes the few remaining holes (:func:`triangulate_samples`).
4. **Quad extraction** -- neighbouring triangles are merged in pairs, greedily and scored by how well the resulting
   quad's edges line up with the cross field, its planarity and its regularity (:func:`merge_to_quads`).

The result is a **quad-dominant** mesh (mostly quads, triangles where the field has singularities -- QuadWild pro-
duces those too). It is not a perfect all-quad mesh and is coarser than a voxel remesh; the point is even,
flow-following topology.
"""
from __future__ import annotations

from typing import List, Tuple

import numpy as np

from .raycast import TriGrid
from . import spatial

MAX_SAMPLES = 200_000
MAX_CANDIDATES = 3_000_000
FIELD_ITERS = 24
POISSON_FACTOR = 30.0
POISSON_RADIUS = 0.78            # times L: hits roughly one sample per L^2 on the surface
_FLOAT = np.float32


# ---------------------------------------------------------------------------
# small mesh helpers (points P (n,3), triangles T (m,3))
# ---------------------------------------------------------------------------
def _normalize(v: np.ndarray, eps: float = 1e-20) -> np.ndarray:
    n = np.linalg.norm(v, axis=-1, keepdims=True)
    return v / np.maximum(n, eps)


def tri_edges(T: np.ndarray) -> np.ndarray:
    """Unique undirected edges (E, 2) of the triangles."""
    if not len(T):
        return np.zeros((0, 2), np.int64)
    a = np.concatenate([T[:, 0], T[:, 1], T[:, 2]])
    b = np.concatenate([T[:, 1], T[:, 2], T[:, 0]])
    return np.unique(np.stack([np.minimum(a, b), np.maximum(a, b)], 1), axis=0)


def _directed(E: np.ndarray):
    return np.concatenate([E[:, 0], E[:, 1]]), np.concatenate([E[:, 1], E[:, 0]])


def point_normals(P: np.ndarray, T: np.ndarray, iters: int = 8) -> np.ndarray:
    """Unnormalised (area-weighted) triangle normals accumulated per point, then smoothed over the point graph."""
    N = np.zeros_like(P, dtype=np.float64)
    if not len(T):
        return N
    fn = np.cross(P[T[:, 1]] - P[T[:, 0]], P[T[:, 2]] - P[T[:, 0]])
    for k in range(3):
        np.add.at(N, T[:, k], fn)
    N = _normalize(N)
    E = tri_edges(T)
    if len(E) and iters > 0:
        src, dst = _directed(E)
        for _ in range(iters):
            acc = N.copy()
            np.add.at(acc, src, N[dst])
            N = _normalize(0.5 * acc + 0.5 * N)
    return N


def _tangent_frames(N: np.ndarray) -> Tuple[np.ndarray, np.ndarray]:
    ref = np.tile(np.array([0.0, 0.0, 1.0]), (len(N), 1))
    ref[np.abs(N[:, 2]) > 0.9] = np.array([1.0, 0.0, 0.0])
    u = _normalize(np.cross(ref, N))
    v = np.cross(N, u)
    return u, v


# ---------------------------------------------------------------------------
# orientation (cross) field
# ---------------------------------------------------------------------------
def _rotation_apply(a: np.ndarray, b: np.ndarray, w: np.ndarray) -> np.ndarray:
    """Apply, per row, the minimal rotation taking unit ``a`` onto unit ``b`` to ``w`` (Rodrigues vector form)."""
    a, b = _normalize(a), _normalize(b)
    c = np.einsum("ij,ij->i", a, b)
    v = np.cross(a, b)
    out = w + np.cross(v, w) + np.cross(v, np.cross(v, w)) / np.clip(1.0 + c, 1e-9, None)[:, None]
    anti = c < -1.0 + 1e-7
    if anti.any():
        ref = np.tile(np.array([0.0, 0.0, 1.0]), (int(anti.sum()), 1))
        ref[np.abs(a[anti, 2]) > 0.9] = np.array([1.0, 0.0, 0.0])
        p = _normalize(np.cross(a[anti], ref))
        out[anti] = 2.0 * np.einsum("ij,ij->i", w[anti], p)[:, None] * p - w[anti]
    return out


def cross_field(P: np.ndarray, T: np.ndarray, N: np.ndarray, align: float = 1.5,
                iters: int = FIELD_ITERS) -> Tuple[np.ndarray, np.ndarray]:
    """Per-point 4-RoSy direction (f, g): curvature-aligned and smoothed with parallel transport."""
    u, v = _tangent_frames(N)
    E = tri_edges(T)
    if not len(E):
        return u.copy(), v.copy()
    src, dst = _directed(E)
    dn = N[dst] - N[src]
    a = np.einsum("ij,ij->i", dn, u[src])
    b = np.einsum("ij,ij->i", dn, v[src])
    n = len(P)
    cxx = np.bincount(src, weights=a * a, minlength=n)
    cxy = np.bincount(src, weights=a * b, minlength=n)
    cyy = np.bincount(src, weights=b * b, minlength=n)
    disc = np.sqrt(np.maximum((cxx - cyy) ** 2 + 4.0 * cxy ** 2, 0.0))
    l1 = 0.5 * (cxx + cyy + disc)
    ex, ey = cxy, l1 - cxx
    flat = (np.abs(ex) < 1e-12) & (np.abs(ey) < 1e-12)
    ex, ey = np.where(flat, l1 - cyy, ex), np.where(flat, cxy, ey)
    theta_p = np.arctan2(ey, ex)
    z_principal = np.exp(1j * 4.0 * theta_p)
    # per directed edge: angle offset that transports dst's frame into src's frame
    ru = _rotation_apply(N[dst], N[src], u[dst])
    beta = np.arctan2(np.einsum("ij,ij->i", ru, v[src]), np.einsum("ij,ij->i", ru, u[src]))
    zc = np.exp(1j * 4.0 * beta)
    z = z_principal.copy()
    step = align / max(iters, 1)
    for _ in range(iters):
        contrib = z[dst] * zc
        s = (np.bincount(src, weights=contrib.real, minlength=n)
             + 1j * np.bincount(src, weights=contrib.imag, minlength=n))
        s = s + step * z_principal
        mag = np.abs(s)
        z = np.where(mag > 1e-12, s / np.maximum(mag, 1e-12), z)
    theta = np.arctan2(z.imag, z.real) / 4.0
    f = np.cos(theta)[:, None] * u + np.sin(theta)[:, None] * v
    g = np.cross(N, f)
    return _normalize(f), _normalize(g)


# ---------------------------------------------------------------------------
# resampling
# ---------------------------------------------------------------------------
def _poisson_disk(cands, radii, order) -> np.ndarray:
    lo = cands.min(0)
    h = max(float(radii.max()), 1e-9)
    dims = np.maximum(((cands.max(0) - lo) / h).astype(np.int64) + 1, 1)
    buckets: dict = {}
    accept: List[int] = []
    for i in order.tolist():
        r = radii[i]
        cell = np.clip(((cands[i] - lo) / h).astype(np.int64), 0, dims - 1).tolist()
        ok = True
        for dx in (-1, 0, 1):
            for dy in (-1, 0, 1):
                for dz in (-1, 0, 1):
                    for j in buckets.get((cell[0] + dx, cell[1] + dy, cell[2] + dz), ()):
                        if np.linalg.norm(cands[i] - cands[j]) < max(r, radii[j]):
                            ok = False
                            break
                    if not ok:
                        break
                if not ok:
                    break
            if not ok:
                break
        if ok:
            accept.append(i)
            buckets.setdefault(tuple(cell), []).append(i)
    return np.asarray(accept, np.int64)


def _area_sample(P, T, count, rng):
    e1, e2 = P[T[:, 1]] - P[T[:, 0]], P[T[:, 2]] - P[T[:, 0]]
    area = 0.5 * np.linalg.norm(np.cross(e1, e2), axis=1)
    tot = float(area.sum())
    if tot <= 0:
        return np.zeros((0, 3))
    tri = rng.choice(len(T), size=count, p=area / tot)
    r1 = np.sqrt(rng.random(count))
    r2 = rng.random(count)
    return P[T[tri, 0]] + (r1 * (1 - r2))[:, None] * e1[tri] + (r2 * r1)[:, None] * e2[tri]


def _point_curvature(P, T, N):
    """Per point, mean turn of the normal per unit length over its incident edges (0 on flat regions)."""
    E = tri_edges(T)
    n = len(P)
    if not len(E):
        return np.zeros(n)
    src, dst = _directed(E)
    turn = 1.0 - np.einsum("ij,ij->i", N[src], N[dst])
    length = np.linalg.norm(P[dst] - P[src], axis=1)
    val = turn / np.maximum(length, 1e-12)
    s = np.bincount(src, weights=val, minlength=n)
    c = np.maximum(np.bincount(src, minlength=n), 1)
    return s / c


def sample_surface(P, T, N, L, adaptivity, seed, relax_iters, grid: TriGrid):
    """Return (samples (m,3), sample normals, nearest input point per sample) spaced about ``L`` apart."""
    rng = np.random.default_rng(seed)
    e1 = P[T[:, 1]] - P[T[:, 0]]
    e2 = P[T[:, 2]] - P[T[:, 0]]
    area = 0.5 * np.linalg.norm(np.cross(e1, e2), axis=1)
    expect = max(int(float(area.sum()) / (L * L)), 1)
    if expect > MAX_SAMPLES:
        raise ValueError(f"edge_length {L} would make about {expect:,} samples; the limit is {MAX_SAMPLES:,}")
    cands = _area_sample(P, T, int(min(POISSON_FACTOR * expect, MAX_CANDIDATES)), rng)
    if not len(cands):
        return np.zeros((0, 3)), np.zeros((0, 3)), np.zeros(0, np.int64)
    radii = np.full(len(cands), POISSON_RADIUS * L)
    if adaptivity > 0.0:
        curv = _point_curvature(P, T, N)
        scale = float(np.percentile(curv[curv > 0], 75)) if np.any(curv > 0) else 1.0
        ci = _nearest_point(P, cands)
        k = np.clip(curv[ci] / max(scale, 1e-12), 0.0, 1.0)     # 0 on flat ground, ~1 in the tightest bends
        radii = POISSON_RADIUS * L * (1.0 + adaptivity * (1.0 - k))   # crowd the bends, thin out the flat parts
    keep = _poisson_disk(cands, radii, rng.permutation(len(cands)))
    S = cands[keep]
    S, _, _ = grid.closest(S)
    if relax_iters > 0 and len(S) > 3:
        radius = 1.9 * L
        for _ in range(relax_iters):
            i, j, _ = spatial.pairs_within(S, radius)
            if not len(i):
                break
            acc = np.zeros_like(S)
            cnt = np.zeros(len(S))
            np.add.at(acc, i, S[j])
            np.add.at(acc, j, S[i])
            np.add.at(cnt, i, 1.0)
            np.add.at(cnt, j, 1.0)
            moved = cnt > 0
            target = np.where(moved[:, None], acc / np.maximum(cnt, 1)[:, None], S)
            S = 0.5 * S + 0.5 * target
            S, _, _ = grid.closest(S)
    sidx = _nearest_point(P, S)
    sn = N[sidx]
    return S, sn, sidx


def _nearest_point(P, pts):
    if not len(P):
        return np.zeros(len(pts), np.int64)
    idx, _ = spatial.nearest(P, pts)
    return idx


# ---------------------------------------------------------------------------
# connectivity of the samples (ball pivoting / advancing front)
# ---------------------------------------------------------------------------
def _adjacency(S: np.ndarray, r: float) -> List[set]:
    i, j, _ = spatial.pairs_within(S, r)
    adj: List[set] = [set() for _ in range(len(S))]
    for a, b in zip(i.tolist(), j.tolist()):
        adj[a].add(b)
        adj[b].add(a)
    return adj


def _circum(p0: np.ndarray, p1: np.ndarray, p2: np.ndarray):
    """Circumcentre and radius of a triangle in 3D."""
    ab, ac = p1 - p0, p2 - p0
    n = np.cross(ab, ac)
    nn = float(n @ n)
    if nn < 1e-30:
        return p0, 0.0
    centre = p0 + (float(ab @ ab) * np.cross(ac, n) + float(ac @ ac) * np.cross(n, ab)) / (2.0 * nn)
    return centre, float(np.linalg.norm(centre - p0))


def triangulate_samples(S: np.ndarray, N: np.ndarray, L: float):
    """Triangulate the samples with **ball pivoting** (advancing front): grow triangles over the front, always taking
    the neighbour that makes the best-shaped, outward-facing face, then cap leftover holes with a centre fan.

    Returns ``(F, added)``: the triangles (indexing ``S`` followed by ``added``) and any extra cap-centre points."""
    m = len(S)
    if m < 4:
        return np.zeros((0, 3), np.int64), np.zeros((0, 3))
    adj = _adjacency(S, 2.2 * L)
    umax = 2.5 * L

    def key(u, v):
        return (u, v) if u < v else (v, u)

    edge_cnt: dict = {}
    front: set = set()                                # directed boundary edges, triangle on the inside of (u -> v)
    tris: List[Tuple[int, int, int]] = []

    def push(a, b, c):
        tris.append((a, b, c))
        for u, v in ((a, b), (b, c), (c, a)):
            edge_cnt[key(u, v)] = edge_cnt.get(key(u, v), 0) + 1
            if (v, u) in front:
                front.discard((v, u))
            else:
                front.add((u, v))

    def best_third(a, b, u=umax):
        """The free point making the smallest, empty-circumsphere, outward triangle on the far side of a -> b."""
        if edge_cnt.get(key(a, b), 0) >= 2:                 # stale front edge: the slot is already full
            return -1
        cand = adj[a] & adj[b]
        if not cand:
            return -1
        near = adj[a] | adj[b]
        for r in cand:
            near |= adj[r]
        best, best_key = -1, None
        fallback, fallback_key = -1, None
        for r in cand:
            if r == a or r == b or edge_cnt.get(key(b, r), 0) >= 2 or edge_cnt.get(key(r, a), 0) >= 2:
                continue
            if _cnt_bad(S[b], S[a], S[r], u):
                continue
            nrm = np.cross(S[a] - S[b], S[r] - S[b])
            if np.dot(nrm, N[a] + N[b] + N[r]) <= 0:            # faces the wrong way (or is degenerate)
                continue
            centre, radius = _circum(S[b], S[a], S[r])
            if radius <= 1e-12:
                continue
            empty = True
            for o in near:
                if o == a or o == b or o == r:
                    continue
                if np.linalg.norm(S[o] - centre) < radius - 1e-6 * radius:
                    empty = False
                    break
            # On a sphere every candidate's circumsphere is the surface itself, so radius cannot tell them apart;
            # scoring by the longest side instead keeps the front made of small, well-shaped triangles.
            longest = max(np.linalg.norm(S[a] - S[b]), np.linalg.norm(S[b] - S[r]), np.linalg.norm(S[r] - S[a]))
            sc = (1 if empty else 0, -longest, -radius)
            if best_key is None or sc > best_key:
                best, best_key = r, sc
            if fallback_key is None or -longest > fallback_key:
                fallback, fallback_key = r, -longest
        return best if best != -1 else fallback

    start = int(np.argmax([len(s) for s in adj]))
    p = -1
    for q in sorted(adj[start]):
        for r in sorted(adj[start]):
            if q < r and r in adj[q] and key(start, q) not in edge_cnt and \
                    not _cnt_bad(S[start], S[q], S[r], umax):
                nrm = np.cross(S[q] - S[start], S[r] - S[start])
                if np.dot(nrm, N[start] + N[q] + N[r]) > 0:
                    p = (q, r)
                    break
        if p != -1:
            break
    if p == -1:
        return np.zeros((0, 3), np.int64)
    push(start, p[0], p[1])
    stalled: List[Tuple[int, int]] = []
    while front:
        a, b = front.pop()
        if edge_cnt.get(key(a, b), 0) >= 2:                 # a shared edge pushed alongside a real one
            continue
        r = best_third(a, b)
        if r < 0:
            stalled.append((a, b))
        else:
            push(b, a, r)
    for it in range(16):                                    # filling nearby edges frees edges that stalled before
        again = []
        u = umax * (1.0 + 0.5 * it)                         # let later passes reach across a slightly wider gap
        for a, b in stalled:
            if edge_cnt.get(key(a, b), 0) >= 2:
                continue
            r = best_third(a, b, u)
            if r < 0:
                again.append((a, b))
            else:
                push(b, a, r)
        if len(again) == len(stalled):
            break
        stalled = again
    if not tris:
        return np.zeros((0, 3), np.int64), np.zeros((0, 3))
    added: List[np.ndarray] = []
    _fill_holes(S, tris, edge_cnt, added)
    return np.array(tris, np.int64), (np.asarray(added, np.float64) if added else np.zeros((0, 3)))


def _fill_holes(S: np.ndarray, tris, edge_cnt: dict, added: List[np.ndarray]) -> None:
    """Close the boundary loops the front left behind, so the surface comes out watertight.

    The undirected boundary edges form an Eulerian graph (every boundary vertex has an even number of them), so they
    split into closed loops; each loop (a figure-eight is split at its pinch first) gets one centre vertex at its
    average, fanned to the whole loop. The centre's spokes are all new edges, so the cap is manifold and never folds
    over the surrounding surface."""
    def kk(u, v):
        return (u, v) if u < v else (v, u)

    remaining: set = set()
    adj: dict = {}
    for a, b, c in tris:
        for u, v in ((a, b), (b, c), (c, a)):
            k = kk(u, v)
            if edge_cnt.get(k, 0) == 1:
                remaining.add(k)
    if not remaining:
        return
    for u_, v_ in remaining:
        adj.setdefault(u_, []).append(v_)
        adj.setdefault(v_, []).append(u_)
    while remaining:
        u0, v0 = next(iter(remaining))
        remaining.discard(kk(u0, v0))
        loop, cur = [u0], v0
        while cur != u0:
            loop.append(cur)
            nbrs = [w for w in adj.get(cur, ()) if kk(cur, w) in remaining]
            if not nbrs:
                loop = []
                break
            remaining.discard(kk(cur, nbrs[0]))
            cur = nbrs[0]
        if len(loop) < 3:
            continue
        for simple in _split_loop(loop):
            centre = len(S) + len(added)
            added.append(S[simple].mean(0))
            for j in range(len(simple)):
                a, b = simple[j], simple[(j + 1) % len(simple)]
                tris.append((centre, a, b))
                edge_cnt[kk(a, b)] = edge_cnt.get(kk(a, b), 0) + 1
                edge_cnt[kk(centre, a)] = edge_cnt.get(kk(centre, a), 0) + 1


def _split_loop(loop: List[int]) -> List[List[int]]:
    """Break a figure-eight walk (a vertex visited twice, e.g. two holes meeting at a pinch) into simple loops."""
    pos: dict = {}
    for i, x in enumerate(loop):
        if x in pos:
            j = pos[x]
            inner, rest = loop[j:i], loop[:j + 1] + loop[i + 1:]
            out: List[List[int]] = []
            if len(inner) >= 3:
                out += _split_loop(inner)
            if len(rest) >= 3:
                out += _split_loop(rest)
            return out
        pos[x] = i
    return [loop]


def _cnt_bad(p0, p1, p2, umax) -> bool:
    return (np.linalg.norm(p1 - p0) > umax or np.linalg.norm(p2 - p1) > umax
            or np.linalg.norm(p2 - p0) > umax)


def _poly_normal(P, f):
    nrm = np.zeros(3)
    for q in range(len(f)):
        nrm += np.cross(P[f[q]], P[f[(q + 1) % len(f)]])
    return nrm


def _orient_faces(P, N, F: np.ndarray) -> np.ndarray:
    """Flip triangle rows so their normal agrees with the samples' normals."""
    if not len(F):
        return F
    n = _normalize(np.cross(P[F[:, 1]] - P[F[:, 0]], P[F[:, 2]] - P[F[:, 0]]))
    ref = _normalize(N[F].sum(1))
    flip = np.einsum("ij,ij->i", n, ref) < 0
    out = F.copy()
    out[flip] = out[flip][:, ::-1]
    return out


def _orient_polys(P, N, polys) -> List[np.ndarray]:
    """Return a list of index arrays, each wound so its face normal agrees with the local surface normal."""
    out = []
    for f in polys:
        f = np.asarray(f, np.int64)
        if np.dot(_poly_normal(P, f), N[f].sum(0)) < 0:
            f = f[::-1]
        out.append(f)
    return out


# ---------------------------------------------------------------------------
# quad extraction
# ---------------------------------------------------------------------------
def _edge_pairs(T: np.ndarray):
    """For each interior edge: the two triangles and their opposite vertices. Returns (t1, t2, a, b, c, d)."""
    nv = len(T) * 3
    vp = T.reshape(-1).astype(np.int64)
    prim = np.repeat(np.arange(len(T)), 3)
    local = np.arange(nv) - 3 * prim
    opposite = T[prim, (local + 2) % 3]
    lo = np.minimum(vp, T[prim, (local + 1) % 3])
    hi = np.maximum(vp, T[prim, (local + 1) % 3])
    n = int(vp.max()) + 1
    key = lo * n + hi
    uniq, inv, cnt = np.unique(key, return_inverse=True, return_counts=True)
    two = uniq[cnt == 2]
    he = np.flatnonzero(np.isin(uniq[inv], two))
    order = np.lexsort((he, inv[he]))
    he = he[order]
    h1, h2 = he[0::2], he[1::2]
    return prim[h1], prim[h2], vp[h1], vp[h2], opposite[h1], opposite[h2]


def merge_to_quads(S, N, F, fdir, gdir, L) -> List[np.ndarray]:
    """Greedily merge triangle pairs of ``F`` into quads, scored by field alignment, planarity and regularity.
    Returns a quad-dominant list of polygons (the merged quads plus the leftover triangles)."""
    polys: List[np.ndarray] = []
    if not len(F):
        return polys
    tri_used = np.zeros(len(F), bool)
    if F.shape[1] == 3:
        t1, t2, a, b, c, d = _edge_pairs(F)
        if len(t1):
            quad = np.stack([a, d, b, c], 1)
            e = [S[quad[:, 1]] - S[quad[:, 0]], S[quad[:, 2]] - S[quad[:, 1]],
                 S[quad[:, 3]] - S[quad[:, 2]], S[quad[:, 0]] - S[quad[:, 3]]]
            elen = np.stack([np.linalg.norm(x, axis=1) for x in e], 1)
            reg = elen.min(1) / np.maximum(elen.max(1), 1e-20)
            n1 = _normalize(np.cross(S[quad[:, 1]] - S[quad[:, 0]], S[quad[:, 2]] - S[quad[:, 0]]))
            n2 = _normalize(np.cross(S[quad[:, 2]] - S[quad[:, 0]], S[quad[:, 3]] - S[quad[:, 0]]))
            planar = np.abs(np.einsum("ij,ij->i", n1, n2))
            centre = 0.25 * (S[quad[:, 0]] + S[quad[:, 1]] + S[quad[:, 2]] + S[quad[:, 3]])
            fc = _field_at(S, fdir, gdir, centre)
            align = np.zeros(len(quad))
            for x in e:
                xh = x / np.maximum(np.linalg.norm(x, axis=1, keepdims=True), 1e-20)
                align += np.maximum(np.abs(np.einsum("ij,ij->i", xh, fc[0])),
                                    np.abs(np.einsum("ij,ij->i", xh, fc[1])))
            align /= 4.0
            dh = _normalize(S[c] - S[a])
            diag_align = np.maximum(np.abs(np.einsum("ij,ij->i", dh, fc[0])),
                                    np.abs(np.einsum("ij,ij->i", dh, fc[1])))
            score = align + 0.7 * planar + 0.5 * reg - 0.4 * diag_align
            for k in np.argsort(-score):
                if planar[k] < 0.05 or reg[k] < 0.12:
                    continue
                if tri_used[t1[k]] or tri_used[t2[k]]:
                    continue
                tri_used[t1[k]] = tri_used[t2[k]] = True
                polys.append(quad[k].copy())
    for i in np.flatnonzero(~tri_used):
        polys.append(F[i].copy())
    return polys


def _field_at(S, fdir, gdir, pts):
    idx = np.clip(_nearest_point(S, pts), 0, max(len(S) - 1, 0))
    return fdir[idx], gdir[idx]


# ---------------------------------------------------------------------------
# relaxation and top level
# ---------------------------------------------------------------------------
def _poly_edges(polys):
    ea, eb = [], []
    for f in polys:
        ea.append(f)
        eb.append(np.roll(f, -1))
    if not ea:
        return np.zeros((0, 2), np.int64)
    a, b = np.concatenate(ea), np.concatenate(eb)
    return np.unique(np.stack([np.minimum(a, b), np.maximum(a, b)], 1), axis=0)


def remesh(P, T, L, adaptivity=0.0, relax_iters=4, seed=0, field_align=1.5, smooth=3):
    """Field-aligned remesh of a triangle mesh. Returns (points, polygons, source_index)."""
    P = np.asarray(P, np.float64)
    T = np.asarray(T, np.int64).reshape(-1, 3)
    if L <= 0:
        ext = float(np.linalg.norm(P.max(0) - P.min(0))) if len(P) else 1.0
        L = max(ext * 0.05, 1e-6)
    if not len(T):
        return P.astype(_FLOAT), [], np.arange(len(P), np.int64)
    grid = TriGrid(P[T])
    N = point_normals(P, T)
    fdir, gdir = cross_field(P, T, N, align=field_align)
    S, sn, _ = sample_surface(P, T, N, L, adaptivity, seed, relax_iters, grid)
    F, added = triangulate_samples(S, sn, L)
    if len(added):
        S = np.vstack([S, added])
        sn = N[_nearest_point(P, S)]
    F = _orient_faces(S, sn, F)
    si = _nearest_point(P, S)
    fS, gS = fdir[si], gdir[si]
    polys = merge_to_quads(S, sn, F, fS, gS, L)
    S, polys = _relax(S, polys, grid, iters=smooth)
    sn2 = N[_nearest_point(P, S)]
    polys = _orient_polys(S, sn2, polys)
    sidx = _nearest_point(P, S)
    return S.astype(_FLOAT), polys, sidx


def _relax(S, polys, grid, iters=3):
    if not polys or iters <= 0:
        return S, polys
    for _ in range(iters):
        E = _poly_edges(polys)
        acc = np.zeros_like(S)
        cnt = np.zeros(len(S))
        np.add.at(acc, E[:, 0], S[E[:, 1]])
        np.add.at(acc, E[:, 1], S[E[:, 0]])
        np.add.at(cnt, E[:, 0], 1.0)
        np.add.at(cnt, E[:, 1], 1.0)
        moved = cnt > 0
        target = np.where(moved[:, None], acc / np.maximum(cnt, 1)[:, None], S)
        S = 0.5 * S + 0.5 * target
        S, _, _ = grid.closest(S)
    return S, polys
