"""UV unwrapping: islands, least-squares conformal maps (LSCM) and packing.

* **Islands**: faces joined across edges that are not seams. Seams are mesh borders, edges sharper than an angle and
  edges where an optional primitive attribute changes. LSCM needs disc-like islands (Euler characteristic 1), so closed
  or ring-shaped islands are cut by planes through their centre until they are discs (a sphere into two cups, a torus
  into quarters), trying the world and principal axes and keeping the cut that leaves the most discs.
* **LSCM** (Levy et al. 2002): per triangle, the map's deviation from a similarity in the triangle's own plane, summed
  as one sparse least-squares problem over all islands with two pinned points each. SciPy's sparse direct solver when
  installed, else conjugate gradients (Jacobi-preconditioned) on the normal equations in numpy, started from each
  island's planar projection (the CG loop compiled with Numba when available).
* **Packing**: islands are scaled to keep their 3-D area ratios, turned so their long axis lies along U, shelf-packed
  tallest first and the whole layout fitted into the 0-1 square with a margin.
"""
from __future__ import annotations

import logging
import os

import numpy as np

from . import accel, spatial, topo

log = logging.getLogger(__name__)
_hinted = False


def _scipy():
    if os.environ.get("MINIHOU_NO_SCIPY"):
        return None
    try:
        import scipy.sparse as sp
        import scipy.sparse.linalg as spla
        return sp, spla
    except Exception:  # noqa: BLE001 - optional
        return None


# -- islands ----------------------------------------------------------------------------------------------------
def face_islands(geo, twin, seam):
    """Labels 0..k-1 per primitive: faces joined across half-edges i -> twin[i] unless seam[i]."""
    pv = topo.prim_of_vertex(geo)
    j = np.flatnonzero((twin >= 0) & ~seam)
    return spatial.components(geo.nprims, pv[j], pv[twin[j]])


def euler(geo, label, k):
    """V - E + F of each island (1 for a disc)."""
    t = geo.topology
    pv = topo.prim_of_vertex(geo)
    vp = t.vertex_point.astype(np.int64)
    nx = topo.next_vertex(geo)
    lab = label[pv].astype(np.int64)
    n = max(geo.npoints, 1)
    V = np.unique(lab * n + vp)
    a, b = np.minimum(vp, vp[nx]), np.maximum(vp, vp[nx])
    E = np.unique((lab * n + a) * n + b)
    return (np.bincount(V // n, minlength=k) - np.bincount(E // (n * n), minlength=k)
            + np.bincount(label, minlength=k))


def make_discs(geo, twin, seam, label, rounds: int = 6):
    """Cut non-disc islands (see the module docstring); returns new labels. Islands still not discs after ``rounds``
    are left as they are (LSCM then folds them, but never fails)."""
    from .nodes.normal import prim_normals
    area = np.linalg.norm(prim_normals(geo), axis=1) / 2.0 + 1e-30
    cen = topo.prim_centroids(geo).astype(np.float64)
    pv = topo.prim_of_vertex(geo)

    def relabel(lab):                                      # split islands into connected pieces of equal label
        j = np.flatnonzero((twin >= 0) & ~seam)
        j = j[lab[pv[j]] == lab[pv[twin[j]]]]
        return spatial.components(geo.nprims, pv[j], pv[twin[j]])

    for _ in range(rounds):
        k = int(label.max()) + 1 if len(label) else 0
        chi = euler(geo, label, k)
        bad = np.flatnonzero(chi != 1)
        if not len(bad):
            break
        for isl in bad:
            f = np.flatnonzero(label == isl)
            if len(f) < 2:
                continue
            w = area[f]
            c = cen[f] - np.average(cen[f], axis=0, weights=w)
            _, _, axes = np.linalg.svd(c * np.sqrt(w)[:, None], full_matrices=False)
            best, best_score = None, None
            # world axes first, principal axes only when strictly better: symmetric shapes (a cube's principal axes
            # are arbitrary) then cut the same way on every platform
            for ax in np.concatenate([np.eye(3), axes]):
                side = c @ ax > 0
                if side.all() or not side.any():
                    continue
                trial = label.copy()
                trial[f[side]] = k
                trial = relabel(trial)
                kk = int(trial.max()) + 1
                pieces = np.unique(trial[f])
                chi_t = euler(geo, trial, kk)[pieces]
                score = (int((chi_t == 1).sum()) - len(pieces) * 0.01, -abs(int(chi_t.min()) - 1))
                if best_score is None or score > best_score:
                    best, best_score = side, score
            if best is not None:
                label = label.copy()
                label[f[best]] = k
                k += 1
        label = relabel(label)
    return label


# -- LSCM -------------------------------------------------------------------------------------------------------
def lscm(P, tris, pins, pin_uv, island_of=None, tol=1e-7, max_iter=50000):
    """Solve LSCM for the points ``P`` (the unknowns) of triangles ``tris`` (T, 3), with ``pins`` fixed at ``pin_uv``.
    ``island_of`` (per point) lets the numpy fallback start from a planar projection. Returns (len(P), 2) uvs."""
    P = np.asarray(P, np.float64)
    p0, p1, p2 = P[tris[:, 0]], P[tris[:, 1]], P[tris[:, 2]]
    e1, e2 = p1 - p0, p2 - p0
    nrm = np.cross(e1, e2)
    dT = np.linalg.norm(nrm, axis=1)                        # twice the area
    ok = dT > 1e-20 * max(1.0, float(np.abs(P).max(initial=0.0)) ** 2)
    e1, e2, nrm, dT, tris_u = e1[ok], e2[ok], nrm[ok], dT[ok], tris[ok]
    x = e1 / np.maximum(np.linalg.norm(e1, axis=1, keepdims=True), 1e-300)
    y = np.cross(nrm / dT[:, None], x)
    xs = np.stack([np.zeros(len(x)), np.einsum("ij,ij->i", e1, x), np.einsum("ij,ij->i", e2, x)], 1)
    ys = np.stack([np.zeros(len(x)), np.zeros(len(x)), np.einsum("ij,ij->i", e2, y)], 1)
    s = 1.0 / np.sqrt(dT)
    Wr = np.stack([xs[:, 2] - xs[:, 1], xs[:, 0] - xs[:, 2], xs[:, 1] - xs[:, 0]], 1) * s[:, None]
    Wi = np.stack([ys[:, 2] - ys[:, 1], ys[:, 0] - ys[:, 2], ys[:, 1] - ys[:, 0]], 1) * s[:, None]
    T = len(Wr)
    # rows 2t (real part) and 2t+1 (imaginary part); columns u_j = j, v_j = n + j
    n = len(P)
    rows = np.concatenate([np.repeat(2 * np.arange(T), 3)] * 2 + [np.repeat(2 * np.arange(T) + 1, 3)] * 2)
    cols = np.concatenate([tris_u.ravel(), n + tris_u.ravel(), tris_u.ravel(), n + tris_u.ravel()])
    vals = np.concatenate([Wr.ravel(), -Wi.ravel(), Wi.ravel(), Wr.ravel()])
    fixed = np.zeros(2 * n, bool)
    fixed[pins] = fixed[n + pins] = True
    xfix = np.zeros(2 * n)
    xfix[pins], xfix[n + pins] = pin_uv[:, 0], pin_uv[:, 1]
    rhs_full = -np.bincount(rows, weights=vals * xfix[cols], minlength=2 * T)
    free = np.flatnonzero(~fixed)
    col_of = np.full(2 * n, -1, np.int64)
    col_of[free] = np.arange(len(free))
    keep = ~fixed[cols]
    r, c, v = rows[keep], col_of[cols[keep]], vals[keep]
    sol = xfix.copy()
    if len(free):
        sp = _scipy()
        if sp is not None:
            A = sp[0].csr_matrix((v, (r, c)), shape=(2 * T, len(free)))
            AtA = (A.T @ A).tocsc()
            AtA = AtA + sp[0].identity(len(free), format="csc") * 1e-12 * float(abs(AtA.diagonal()).max(initial=1.0))
            sol[free] = sp[1].spsolve(AtA, A.T @ rhs_full)
        else:
            global _hinted
            if not _hinted and len(free) > 20000:
                _hinted = True
                log.info("uv_flatten: install SciPy (pip install scipy) for much faster unwrapping of big meshes")
            x0 = np.zeros(len(free))
            if island_of is not None:
                g = planar_guess(P, tris_u, nrm, island_of, pins, pin_uv)
                x0 = np.concatenate([g[:, 0], g[:, 1]])[free]
            sol[free] = _cg_normal(r, c, v, len(free), rhs_full, x0, tol, max_iter)
    return np.stack([sol[:n], sol[n:]], 1)


def planar_guess(P, tris, nrm, island_of, pins, pin_uv):
    """Each island projected onto its best-fit plane (facing its mean normal), moved onto its pins by a similarity."""
    k = int(island_of.max()) + 1
    out = np.zeros((len(P), 2))
    tri_isl = island_of[tris[:, 0]]
    mean_n = np.stack([np.bincount(tri_isl, weights=nrm[:, j], minlength=k) for j in range(3)], 1)
    pin_of = np.full(k, -1, np.int64)
    pin2_of = np.full(k, -1, np.int64)
    pin_of[island_of[pins][::-1]] = np.arange(len(pins))[::-1]
    pin2_of[island_of[pins]] = np.arange(len(pins))
    order = np.argsort(island_of, kind="stable")
    starts = np.searchsorted(island_of[order], np.arange(k + 1))
    for i in range(k):
        idx = order[starts[i]:starts[i + 1]]
        if len(idx) < 3 or pin_of[i] < 0 or pin_of[i] == pin2_of[i]:
            continue
        q = P[idx] - P[idx].mean(0)
        _, _, vt = np.linalg.svd(q, full_matrices=False)
        a, b = vt[0], vt[1]
        if np.cross(a, b) @ mean_n[i] < 0:
            b = -b
        z = (q @ a) + 1j * (q @ b)
        pa, pb = pin_of[i], pin2_of[i]
        za = (P[pins[pa]] - P[idx].mean(0)) @ a + 1j * ((P[pins[pa]] - P[idx].mean(0)) @ b)
        zb = (P[pins[pb]] - P[idx].mean(0)) @ a + 1j * ((P[pins[pb]] - P[idx].mean(0)) @ b)
        wa, wb = complex(*pin_uv[pa]), complex(*pin_uv[pb])
        sim = (wb - wa) / (zb - za) if abs(zb - za) > 1e-30 else 1.0
        w = wa + sim * (z - za)
        out[idx] = np.stack([w.real, w.imag], 1)
    return out


def _cg_normal(r, c, v, n, b, x0, tol, max_iter):
    """Least squares |A x - b| (A as COO rows r, columns c, values v): Jacobi-preconditioned conjugate gradients on
    A^T A x = A^T b, with A^T A assembled once (pairs of entries sharing a row)."""
    order = np.argsort(r, kind="stable")
    r, c, v = r[order], c[order], v[order]
    cnt = np.bincount(r)
    cnt = cnt[r]
    first = np.searchsorted(r, r)
    rep = np.repeat(np.arange(len(r)), cnt)                 # each entry paired with every entry of its row
    other = np.repeat(first, cnt) + (np.arange(len(rep)) - np.repeat(np.cumsum(cnt) - cnt, cnt))
    key = c[rep] * n + c[other]
    uk, inv = np.unique(key, return_inverse=True)
    M_val = np.bincount(inv.reshape(-1), weights=v[rep] * v[other])
    M_row, M_col = uk // n, uk % n
    Atb = np.bincount(c, weights=v * b[r], minlength=n)

    def M(x):
        return np.bincount(M_row, weights=M_val * x[M_col], minlength=n)

    d = np.bincount(M_row[M_row == M_col], weights=M_val[M_row == M_col], minlength=n)
    Minv = 1.0 / np.where(d > 0, d, 1.0)
    x = x0.astype(np.float64).copy()
    stop = tol * tol * max(Atb @ (Minv * Atb), 1e-300)
    k = accel.kernels()
    if k is not None:                                       # the same loop, compiled: ~10x faster
        indptr = np.searchsorted(M_row, np.arange(n + 1)).astype(np.int64)
        if accel.call(k.pcg_csr, indptr, M_col.astype(np.int64), M_val, Atb, x, Minv, stop, max_iter) \
                is not accel.FAILED:
            return x
        x = x0.astype(np.float64).copy()
    g = Atb - M(x)
    z = Minv * g
    p = z.copy()
    gz = g @ z
    for _ in range(max_iter):
        if gz <= stop:
            break
        Mp = M(p)
        alpha = gz / max(p @ Mp, 1e-300)
        x += alpha * p
        g -= alpha * Mp
        z = Minv * g
        gz_new = g @ z
        p = z + (gz_new / gz) * p
        gz = gz_new
    return x


# -- packing ----------------------------------------------------------------------------------------------------
def _align(q):
    """Rotate points so their bounding box is (near) smallest, long side along U: 1 degree steps, then refined."""
    if len(q) < 2:
        return q

    def areas(ang):
        c, s = np.cos(ang), np.sin(ang)
        x = q[:, 0][None] * c[:, None] - q[:, 1][None] * s[:, None]
        y = q[:, 0][None] * s[:, None] + q[:, 1][None] * c[:, None]
        return np.ptp(x, 1) * np.ptp(y, 1)

    coarse = np.radians(np.arange(0.0, 90.0, 1.0))
    best = coarse[int(np.argmin(areas(coarse)))]
    fine = best + np.radians(np.arange(-1.0, 1.0001, 0.02))
    a = fine[int(np.argmin(areas(fine)))]
    c, s = np.cos(a), np.sin(a)
    q = q @ np.array([[c, s], [-s, c]])
    if np.ptp(q[:, 1]) > np.ptp(q[:, 0]):                   # long side along U (a quarter turn keeps orientation)
        q = np.stack([q[:, 1], -q[:, 0]], 1)
    return q


def pack(uv, island_of, scale, k, padding):
    """Scale islands (``scale`` per island: keeps their 3-D area ratios), align, shelf-pack and fit in 0-1. ``uv`` is
    per unknown and ``island_of`` its island."""
    uv = uv.copy()
    boxes = []
    order = np.argsort(island_of, kind="stable")
    starts = np.searchsorted(island_of[order], np.arange(k + 1))
    for i in range(k):
        idx = order[starts[i]:starts[i + 1]]
        if not len(idx):
            boxes.append((0.0, 0.0))
            continue
        q = _align(uv[idx] - uv[idx].mean(0)) * scale[i]
        q -= q.min(0)
        uv[idx] = q
        boxes.append(tuple(q.max(0)))
    boxes = np.asarray(boxes, np.float64).reshape(-1, 2)
    total = float((boxes[:, 0] * boxes[:, 1]).sum())
    gap = padding * np.sqrt(max(total, 1e-30))
    width = max(np.sqrt(total) * 1.15, float(boxes[:, 0].max(initial=0.0)))
    offs = np.zeros((k, 2))
    x = y = shelf = 0.0
    tall = np.round(boxes[:, 1] / max(float(boxes[:, 1].max(initial=0.0)), 1e-30), 3)   # equal-ish heights keep
    for i in np.argsort(-tall, kind="stable"):                                            # island order: stable
        if x > 0 and x + boxes[i, 0] > width:
            x, y, shelf = 0.0, y + shelf + gap, 0.0
        offs[i] = (x, y)
        x += boxes[i, 0] + gap
        shelf = max(shelf, boxes[i, 1])
    uv += offs[island_of]
    span = max(float(uv.max(initial=0.0)), 1e-30)
    margin = padding / 2.0
    return margin + uv / span * (1.0 - 2.0 * margin)
