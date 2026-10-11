"""Point-based procedural generation, in the spirit of Unreal Engine's PCG framework.

A *PCG point* is an ordinary point with these attributes (any that are missing take the defaults):

* ``P``                            -- position
* ``rot``                          -- Euler XYZ rotation in radians (as the instance node writes it); default 0
* ``scale``                        -- per-axis scale; default 1
* ``bounds_min`` / ``bounds_max``  -- the point's box in its own frame (before scale); default -0.5 .. 0.5
* ``density``                      -- 0..1, how strongly the point exists; filters and booleans read and write it
* ``seed``                         -- int per point, so random choices follow the point, not its index

So each point is an oriented box. The point booleans (difference, intersection, self-pruning) test exact box
overlaps (the separating-axis test, vectorised over candidate pairs from a grid of bounding boxes).
"""
from __future__ import annotations

from typing import Dict, Tuple

import numpy as np

DEFAULT_EXTENT = 0.5


# -- attributes -------------------------------------------------------------------------------------------------
def hash32(x) -> np.ndarray:
    """A well-mixed 32-bit hash of ints (lowbias32), as uint64 values below 2**32."""
    x = np.asarray(x).astype(np.uint64) & np.uint64(0xFFFFFFFF)
    x ^= x >> np.uint64(16)
    x = (x * np.uint64(0x7FEB352D)) & np.uint64(0xFFFFFFFF)
    x ^= x >> np.uint64(15)
    x = (x * np.uint64(0x846CA68B)) & np.uint64(0xFFFFFFFF)
    x ^= x >> np.uint64(16)
    return x


def random01(seeds, salt: int, k: int = 0) -> np.ndarray:
    """Deterministic uniform numbers in [0, 1) from per-point seeds, a node's seed (salt) and a stream index k."""
    s = np.asarray(seeds).astype(np.int64) & 0xFFFFFFFF
    h = hash32(s ^ hash32(np.int64(salt) & 0xFFFFFFFF ^ (k * 0x9E3779B9 & 0xFFFFFFFF)).astype(np.int64))
    return h.astype(np.float64) / 4294967296.0


def position_seeds(P) -> np.ndarray:
    """Seeds from positions (millimetre grid), so a point keeps its seed when others are added or removed."""
    q = np.floor(np.asarray(P, np.float64) * 1000.0).astype(np.int64)
    h = hash32(q[:, 0]) ^ hash32(q[:, 1] * 3 + 0x51ED27) ^ hash32(q[:, 2] * 7 + 0x1B873593)
    return (hash32(h) & np.uint64(0x7FFFFFFF)).astype(np.int32)


def read(geo) -> Dict[str, np.ndarray]:
    """The PCG attributes of every point, with defaults filled in (float64 / int32 arrays)."""
    n = geo.npoints
    pt = geo.point

    def vec(name, default):
        a = pt.get(name)
        if a is not None and a.ndim == 2 and a.shape[1] == 3:
            return a.astype(np.float64)
        return np.tile(np.asarray(default, np.float64), (n, 1))

    d = pt.get("density")
    s = pt.get("seed")
    return {
        "P": geo.P.astype(np.float64),
        "rot": vec("rot", (0.0, 0.0, 0.0)),
        "scale": vec("scale", (1.0, 1.0, 1.0)),
        "bounds_min": vec("bounds_min", (-DEFAULT_EXTENT,) * 3),
        "bounds_max": vec("bounds_max", (DEFAULT_EXTENT,) * 3),
        "density": (d.astype(np.float64).reshape(n, -1)[:, 0] if d is not None else np.ones(n)),
        "seed": (s.astype(np.int64).reshape(n, -1)[:, 0].astype(np.int32) if s is not None
                 else position_seeds(geo.P)),
    }


def write(geo, attrs: Dict[str, np.ndarray]):
    """Store PCG attributes on (a copy of) geo."""
    out = geo.copy()
    for k, v in attrs.items():
        if k == "P":
            out.point["P"] = np.asarray(v, np.float32)
        elif k == "seed":
            out.point[k] = np.asarray(v, np.int32)
        else:
            out.point[k] = np.asarray(v, np.float32)
    return out


# -- oriented boxes ---------------------------------------------------------------------------------------------
def rotations(rot) -> np.ndarray:
    from .nodes.instance import euler_xyz_to_matrix
    return euler_xyz_to_matrix(np.asarray(rot, np.float64).reshape(-1, 3))


def boxes(a: Dict[str, np.ndarray]) -> Tuple[np.ndarray, np.ndarray, np.ndarray]:
    """World-space oriented boxes: centres (n, 3), unit axes as matrix columns (n, 3, 3), half extents (n, 3)."""
    R = rotations(a["rot"])
    s = a["scale"]
    lo, hi = a["bounds_min"] * s, a["bounds_max"] * s
    half = np.abs(hi - lo) / 2.0
    centre = a["P"] + np.einsum("nij,nj->ni", R, (lo + hi) / 2.0)
    flip = np.sign(np.where(s == 0, 1.0, s))                # negative scale mirrors an axis: keep axes right-handed
    return centre, R * flip[:, None, :], half


def aabbs(C, A, H):
    ext = np.einsum("nij,nj->ni", np.abs(A), H)
    return C - ext, C + ext


def aabb_pairs(loA, hiA, loB, hiB, same: bool = False):
    """Pairs (i, j) of overlapping axis-aligned boxes between two sets (i < j and no self pairs if same)."""
    if not len(loA) or not len(loB):
        return np.zeros(0, np.int64), np.zeros(0, np.int64)
    lo = np.minimum(loA.min(0), loB.min(0))
    ext = np.maximum(np.maximum(hiA.max(0), hiB.max(0)) - lo, 1e-12)
    size = max(float(np.concatenate([hiA - loA, hiB - loB]).mean(0).max()) * 1.5, float(ext.max()) / 512.0, 1e-9)
    dims = np.maximum(np.ceil(ext / size).astype(np.int64), 1)

    def cells(l, h):
        a = np.clip(((l - lo) / size).astype(np.int64), 0, dims - 1)
        b = np.clip(((h - lo) / size).astype(np.int64), 0, dims - 1)
        span = b - a + 1
        cnt = span.prod(1)
        owner = np.repeat(np.arange(len(a)), cnt)
        k = np.arange(cnt.sum()) - np.repeat(np.cumsum(cnt) - cnt, cnt)
        sx, sy = span[owner, 0], span[owner, 1]
        c = ((a[owner, 0] + k % sx) * dims[1] + a[owner, 1] + (k // sx) % sy) * dims[2] + a[owner, 2] + k // (sx * sy)
        return owner, c

    oa, ca = cells(loA, hiA)
    ob, cb = cells(loB, hiB)
    order = np.argsort(cb, kind="stable")
    cb, ob = cb[order], ob[order]
    left, right = np.searchsorted(cb, ca, "left"), np.searchsorted(cb, ca, "right")
    cnt = right - left
    i = np.repeat(oa, cnt)
    j = ob[np.repeat(left, cnt) + (np.arange(cnt.sum()) - np.repeat(np.cumsum(cnt) - cnt, cnt))]
    if same:
        keep = i < j
        i, j = i[keep], j[keep]
    if not len(i):
        return i, j
    pair = np.unique(np.stack([i, j], 1), axis=0)
    i, j = pair[:, 0], pair[:, 1]
    ok = np.all((loA[i] <= hiB[j]) & (loB[j] <= hiA[i]), axis=1)
    return i[ok], j[ok]


def obb_overlap(Ca, Aa, Ha, Cb, Ab, Hb, tol: float = 1e-9) -> np.ndarray:
    """Separating-axis test for pairs of oriented boxes (centres, axis matrices, half extents): True where they
    overlap with positive volume (boxes that only touch along a face axis do not)."""
    m = len(Ca)
    if not m:
        return np.zeros(0, bool)
    R = np.einsum("mki,mkj->mij", Aa, Ab)                   # R[i, j] = a_i . b_j
    AR = np.abs(R) + 1e-12
    t = np.einsum("mki,mk->mi", Aa, Cb - Ca)                # centre offset in A's frame
    scale = np.maximum(np.maximum(Ha.max(1), Hb.max(1)), 1e-12) * tol
    sep = np.zeros(m, bool)
    for i in range(3):
        sep |= np.abs(t[:, i]) >= Ha[:, i] + np.einsum("mj,mj->m", Hb, AR[:, i, :]) - scale
    for j in range(3):
        sep |= np.abs(np.einsum("mi,mi->m", t, R[:, :, j])) >= \
            np.einsum("mi,mi->m", Ha, AR[:, :, j]) + Hb[:, j] - scale
    for i in range(3):
        i1, i2 = (i + 1) % 3, (i + 2) % 3
        for j in range(3):
            j1, j2 = (j + 1) % 3, (j + 2) % 3
            ra = Ha[:, i1] * AR[:, i2, j] + Ha[:, i2] * AR[:, i1, j]
            rb = Hb[:, j1] * AR[:, i, j2] + Hb[:, j2] * AR[:, i, j1]
            d = np.abs(t[:, i2] * R[:, i1, j] - t[:, i1] * R[:, i2, j])
            sep |= d > ra + rb                            # strict: parallel axes make this 0 > 0 (no verdict)
    return ~sep


def overlapping_pairs(a, b=None):
    """Pairs (i, j) of points whose boxes overlap: between point sets a and b, or within a (i < j) if b is None."""
    Ca, Aa, Ha = boxes(a)
    la, ha = aabbs(Ca, Aa, Ha)
    if b is None:
        i, j = aabb_pairs(la, ha, la, ha, same=True)
        Cb, Ab, Hb = Ca, Aa, Ha
    else:
        Cb, Ab, Hb = boxes(b)
        lb, hb = aabbs(Cb, Ab, Hb)
        i, j = aabb_pairs(la, ha, lb, hb)
    ok = obb_overlap(Ca[i], Aa[i], Ha[i], Cb[j], Ab[j], Hb[j])
    return i[ok], j[ok]


def covered_fraction(a, b, i, j, samples: int = 3):
    """For each point of a: the fraction of its box covered by the boxes of b it overlaps (pairs i, j), estimated on
    a samples^3 grid of cell centres inside the box."""
    n = len(a["P"])
    if not len(i):
        return np.zeros(n)
    Ca, Aa, Ha = boxes(a)
    Cb, Ab, Hb = boxes(b)
    g = (np.arange(samples) + 0.5) / samples * 2.0 - 1.0
    grid = np.stack(np.meshgrid(g, g, g, indexing="ij"), -1).reshape(-1, 3)      # (k, 3) in [-1, 1]
    covered = np.zeros((n, len(grid)), bool)
    for s in range(0, len(i), 65536):
        ii, jj = i[s:s + 65536], j[s:s + 65536]
        pts = Ca[ii][:, None, :] + np.einsum("mij,mkj->mki", Aa[ii], grid[None] * Ha[ii][:, None, :])
        local = np.einsum("mji,mkj->mki", Ab[jj], pts - Cb[jj][:, None, :])
        inside = np.all(np.abs(local) <= Hb[jj][:, None, :] + 1e-12, axis=2)
        np.logical_or.at(covered, ii, inside)
    return covered.mean(1)


def greedy_independent(n, i, j, priority) -> np.ndarray:
    """Keep points so that no two kept points are joined by a pair (i, j), preferring higher priority: the same
    result as visiting points from highest priority and keeping each one not touching a kept one, computed in
    rounds (every point that beats all its undecided neighbours is kept, their neighbours dropped)."""
    order = np.lexsort((np.arange(n), -np.asarray(priority, np.float64)))
    rank = np.empty(n, np.int64)
    rank[order] = np.arange(n)                              # 0 = best
    state = np.zeros(n, np.int8)                            # 0 undecided, 1 kept, -1 dropped
    i, j = np.asarray(i, np.int64), np.asarray(j, np.int64)
    while True:
        und = state == 0
        if not und.any():
            break
        live = und[i] & und[j]
        best = rank.copy()
        np.minimum.at(best, i[live], rank[j[live]])
        np.minimum.at(best, j[live], rank[i[live]])
        win = und & (best == rank)
        state[win] = 1
        lost = np.zeros(n, bool)
        lost[j[win[i] & und[j]]] = True
        lost[i[win[j] & und[i]]] = True
        state[lost & (state == 0)] = -1
    return state == 1
