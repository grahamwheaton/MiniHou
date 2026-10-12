"""Numba kernels (only imported when :mod:`minihou.accel` is available).

Each kernel implements *the same algorithm* as the numpy code next to it (same hash, same gradient set,
same float32 coordinate arithmetic where cell selection depends on it), so the two backends agree to float
rounding. ``tests/test_accel.py`` holds them to that.
"""
import numpy as np
from numba import njit, prange

_M = np.uint64(0xFFFFFFFF)
_C1, _C2, _C3 = np.uint64(0x8DA6B343), np.uint64(0xD8163841), np.uint64(0xCB1AB31F)
_C4, _C5 = np.uint64(0x2C1B3C6D), np.uint64(0x297A2D39)
_S12, _S15 = np.uint64(12), np.uint64(15)

_OPTS = dict(cache=True, nogil=True, fastmath=False)


@njit(**_OPTS)
def _hash3(ix, iy, iz, seed_mix):
    # all arithmetic in uint64 and masked to 32 bits == the uint32 wraparound of the numpy version
    h = ((np.uint64(ix) & _M) * _C1) & _M
    h ^= ((np.uint64(iy) & _M) * _C2) & _M
    h ^= ((np.uint64(iz) & _M) * _C3) & _M
    h ^= seed_mix
    h ^= h >> _S15
    h = (h * _C4) & _M
    h ^= h >> _S12
    h = (h * _C5) & _M
    h ^= h >> _S15
    return h


@njit(**_OPTS)
def _grad(h, x, y, z):
    h = h & np.uint64(15)
    u = x if h < 8 else y
    if h < 4:
        v = y
    elif h == 12 or h == 14:
        v = x
    else:
        v = z
    return (-u if (h & np.uint64(1)) else u) + (-v if (h & np.uint64(2)) else v)


@njit(**_OPTS)
def _perlin(px, py, pz, seed_mix):
    # px.. are float32 (cell selection must match numpy's float32 floor exactly)
    fx0, fy0, fz0 = np.floor(px), np.floor(py), np.floor(pz)
    fx, fy, fz = np.float64(px - fx0), np.float64(py - fy0), np.float64(pz - fz0)
    ix, iy, iz = np.int64(fx0), np.int64(fy0), np.int64(fz0)
    ux = fx * fx * fx * (fx * (fx * 6.0 - 15.0) + 10.0)
    uy = fy * fy * fy * (fy * (fy * 6.0 - 15.0) + 10.0)
    uz = fz * fz * fz * (fz * (fz * 6.0 - 15.0) + 10.0)
    g000 = _grad(_hash3(ix, iy, iz, seed_mix), fx, fy, fz)
    g100 = _grad(_hash3(ix + 1, iy, iz, seed_mix), fx - 1.0, fy, fz)
    g010 = _grad(_hash3(ix, iy + 1, iz, seed_mix), fx, fy - 1.0, fz)
    g110 = _grad(_hash3(ix + 1, iy + 1, iz, seed_mix), fx - 1.0, fy - 1.0, fz)
    g001 = _grad(_hash3(ix, iy, iz + 1, seed_mix), fx, fy, fz - 1.0)
    g101 = _grad(_hash3(ix + 1, iy, iz + 1, seed_mix), fx - 1.0, fy, fz - 1.0)
    g011 = _grad(_hash3(ix, iy + 1, iz + 1, seed_mix), fx, fy - 1.0, fz - 1.0)
    g111 = _grad(_hash3(ix + 1, iy + 1, iz + 1, seed_mix), fx - 1.0, fy - 1.0, fz - 1.0)
    x00 = g000 + ux * (g100 - g000)
    x10 = g010 + ux * (g110 - g010)
    x01 = g001 + ux * (g101 - g001)
    x11 = g011 + ux * (g111 - g011)
    y0 = x00 + uy * (x10 - x00)
    y1 = x01 + uy * (x11 - x01)
    return y0 + uz * (y1 - y0)


@njit(**_OPTS)
def _fbm_at(px, py, pz, seed, octaves, lacunarity, gain, norm):
    """Normalised fractal Perlin at one (float32) position. ``seed`` is already reduced to 32 bits."""
    total = 0.0
    amp = 1.0
    freq = 1.0
    for o in range(octaves):
        f32 = np.float32(freq)
        seed_mix = (np.uint64(seed + o * 7919) * np.uint64(0x9E3779B1)) & _M
        total += amp * _perlin(px * f32, py * f32, pz * f32, seed_mix)
        amp *= gain
        freq *= lacunarity
    return np.float32(total / norm) if norm > 0 else np.float32(total)


@njit(**_OPTS)
def _norm(octaves, gain):
    norm = 0.0
    amp = 1.0
    for _o in range(octaves):
        norm += amp
        amp *= gain
    return norm


@njit(parallel=True, **_OPTS)
def fbm3(pos, seed, octaves, lacunarity, gain, out):
    """Fractal Perlin sum into ``out`` (float32, n). ``pos`` is float32 (n, 3). Mirrors ``_util.fbm3_numpy``."""
    norm = _norm(octaves, gain)
    for i in prange(pos.shape[0]):
        out[i] = _fbm_at(pos[i, 0], pos[i, 1], pos[i, 2], seed, octaves, lacunarity, gain, norm)


@njit(parallel=True, **_OPTS)
def noise_vec(P, freq, offset, amp, seed, octaves, lacunarity, gain, vector_mode, add_to_P, out):
    """Fused noise node body: sample at ``P * freq + offset``, scale by ``amp`` and (optionally) add to P,
    in one pass with no temporaries. Same float32 operation order as the numpy node, so results match
    to rounding. ``vector_mode`` uses three decorrelated channels (seed + 1013 * c)."""
    norm = _norm(octaves, gain)
    f32 = np.float32(freq)
    for i in prange(P.shape[0]):
        x = P[i, 0] * f32 + offset[0]
        y = P[i, 1] * f32 + offset[1]
        z = P[i, 2] * f32 + offset[2]
        n0 = _fbm_at(x, y, z, seed, octaves, lacunarity, gain, norm)
        if vector_mode:
            n1 = _fbm_at(x, y, z, seed + 1013, octaves, lacunarity, gain, norm)
            n2 = _fbm_at(x, y, z, seed + 2026, octaves, lacunarity, gain, norm)
        else:
            n1 = n0
            n2 = n0
        if add_to_P:
            out[i, 0] = P[i, 0] + n0 * amp[0]
            out[i, 1] = P[i, 1] + n1 * amp[1]
            out[i, 2] = P[i, 2] + n2 * amp[2]
        else:
            out[i, 0] = n0 * amp[0]
            out[i, 1] = n1 * amp[1]
            out[i, 2] = n2 * amp[2]


@njit(parallel=True, **_OPTS)
def tri_areas(P, vp, offs, counts, first, area, tri_prim):
    """Fan-triangulate every primitive on the fly; write each triangle's area and owning primitive."""
    for p in prange(counts.shape[0]):
        v0 = offs[p]
        a = P[vp[v0]]
        for j in range(counts[p]):
            b = P[vp[v0 + j + 1]]
            c = P[vp[v0 + j + 2]]
            e1x, e1y, e1z = np.float64(b[0]) - a[0], np.float64(b[1]) - a[1], np.float64(b[2]) - a[2]
            e2x, e2y, e2z = np.float64(c[0]) - a[0], np.float64(c[1]) - a[1], np.float64(c[2]) - a[2]
            cx = e1y * e2z - e1z * e2y
            cy = e1z * e2x - e1x * e2z
            cz = e1x * e2y - e1y * e2x
            t = first[p] + j
            area[t] = 0.5 * np.sqrt(cx * cx + cy * cy + cz * cz)
            tri_prim[t] = p


@njit(parallel=True, **_OPTS)
def sample_surface(P, vp, offs, first, tri_prim, cdf, r, r1, r2, out_pos, out_n, out_prim):
    """Area-weighted barycentric sampling. ``r`` etc. are the same uniform streams the numpy path draws."""
    total = cdf[cdf.shape[0] - 1]
    last = cdf.shape[0] - 1
    for s in prange(r.shape[0]):
        t = np.searchsorted(cdf, r[s] * total, side="right")
        if t > last:
            t = last
        p = tri_prim[t]
        j = t - first[p]
        v0 = offs[p]
        a = P[vp[v0]]
        b = P[vp[v0 + j + 1]]
        c = P[vp[v0 + j + 2]]
        u, v = r1[s], r2[s]
        if u + v > 1.0:
            u, v = 1.0 - u, 1.0 - v
        e1x, e1y, e1z = np.float64(b[0]) - a[0], np.float64(b[1]) - a[1], np.float64(b[2]) - a[2]
        e2x, e2y, e2z = np.float64(c[0]) - a[0], np.float64(c[1]) - a[1], np.float64(c[2]) - a[2]
        out_pos[s, 0] = np.float32(a[0] + e1x * u + e2x * v)
        out_pos[s, 1] = np.float32(a[1] + e1y * u + e2y * v)
        out_pos[s, 2] = np.float32(a[2] + e1z * u + e2z * v)
        cx = e1y * e2z - e1z * e2y
        cy = e1z * e2x - e1x * e2z
        cz = e1x * e2y - e1y * e2x
        ln = np.sqrt(cx * cx + cy * cy + cz * cz)
        if ln < 1e-20:
            ln = 1.0
        out_n[s, 0] = np.float32(cx / ln)
        out_n[s, 1] = np.float32(cy / ln)
        out_n[s, 2] = np.float32(cz / ln)
        out_prim[s] = p


@njit(parallel=True, **_OPTS)
def copy_points(src, tP, tN, orient, pscale, use_pscale, out):
    """P of every copy: (src * pscale) rotated (+Z -> N) and moved to the target point. Mirrors the numpy path
    (``rotation_from_z_to`` + batched matmul) without materialising per-copy matrices."""
    T = tP.shape[0]
    S = src.shape[0]
    for i in prange(T):
        r00, r01, r02, r10, r11, r12, r20, r21, r22 = 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0
        if orient:
            nx, ny, nz = np.float64(tN[i, 0]), np.float64(tN[i, 1]), np.float64(tN[i, 2])
            ln = np.sqrt(nx * nx + ny * ny + nz * nz)
            if ln < 1e-20:
                ln = 1.0
            nx, ny, nz = nx / ln, ny / ln, nz / ln
            if nz < -1.0 + 1e-9:                     # +Z -> -Z: 180 degrees about X
                r11, r22 = -1.0, -1.0
            else:
                s = 1.0 / (1.0 + nz)
                x, y = -ny, nx
                r00, r01, r02 = 1.0 - s * y * y, s * x * y, y
                r10, r11, r12 = s * x * y, 1.0 - s * x * x, -x
                r20, r21, r22 = -y, x, 1.0 - s * (x * x + y * y)
        sc = np.float64(pscale[i]) if use_pscale else 1.0
        tx, ty, tz = np.float64(tP[i, 0]), np.float64(tP[i, 1]), np.float64(tP[i, 2])
        for k in range(S):
            x = np.float64(src[k, 0]) * sc
            y = np.float64(src[k, 1]) * sc
            z = np.float64(src[k, 2]) * sc
            o = i * S + k
            out[o, 0] = np.float32(r00 * x + r01 * y + r02 * z + tx)
            out[o, 1] = np.float32(r10 * x + r11 * y + r12 * z + ty)
            out[o, 2] = np.float32(r20 * x + r21 * y + r22 * z + tz)


@njit(parallel=True, **_OPTS)
def prim_normals(P, vp, offs, out):
    """Newell normal per primitive (length = 2 x area), float64. Mirrors ``normal.prim_normals``."""
    for p in prange(offs.shape[0] - 1):
        s, e = offs[p], offs[p + 1]
        nx = ny = nz = 0.0
        for v in range(s, e):
            w = v + 1 if v + 1 < e else s
            ax, ay, az = np.float64(P[vp[v], 0]), np.float64(P[vp[v], 1]), np.float64(P[vp[v], 2])
            bx, by, bz = np.float64(P[vp[w], 0]), np.float64(P[vp[w], 1]), np.float64(P[vp[w], 2])
            nx += (ay - by) * (az + bz)
            ny += (az - bz) * (ax + bx)
            nz += (ax - bx) * (ay + by)
        out[p, 0] = nx
        out[p, 1] = ny
        out[p, 2] = nz


@njit(**_OPTS)
def accumulate_point_normals(vp, offs, prim_n, out):
    """Sum each primitive's normal onto its points (serial: no write races), then normalise to float32 ``out``."""
    npts = out.shape[0]
    acc = np.zeros((npts, 3), np.float64)
    for p in range(offs.shape[0] - 1):
        for v in range(offs[p], offs[p + 1]):
            q = vp[v]
            acc[q, 0] += prim_n[p, 0]
            acc[q, 1] += prim_n[p, 1]
            acc[q, 2] += prim_n[p, 2]
    for q in range(npts):
        ln = np.sqrt(acc[q, 0] ** 2 + acc[q, 1] ** 2 + acc[q, 2] ** 2)
        if ln < 1e-20:
            ln = 1.0
        out[q, 0] = np.float32(acc[q, 0] / ln)
        out[q, 1] = np.float32(acc[q, 1] / ln)
        out[q, 2] = np.float32(acc[q, 2] / ln)


@njit(**_OPTS)
def pcg_csr(indptr, cols, vals, b, x, minv, stop, max_iter):
    """Jacobi-preconditioned conjugate gradients for the SPD matrix in CSR form (the same iteration as
    ``lscm._cg_normal``'s numpy loop); solves in place into x and returns the iteration count."""
    n = b.shape[0]
    g = np.empty(n)
    for i in range(n):
        s = 0.0
        for j in range(indptr[i], indptr[i + 1]):
            s += vals[j] * x[cols[j]]
        g[i] = b[i] - s
    z = minv * g
    p = z.copy()
    gz = 0.0
    for i in range(n):
        gz += g[i] * z[i]
    mp = np.empty(n)
    it = 0
    while it < max_iter and gz > stop:
        pmp = 0.0
        for i in range(n):
            s = 0.0
            for j in range(indptr[i], indptr[i + 1]):
                s += vals[j] * p[cols[j]]
            mp[i] = s
            pmp += p[i] * s
        alpha = gz / max(pmp, 1e-300)
        gz_new = 0.0
        for i in range(n):
            x[i] += alpha * p[i]
            g[i] -= alpha * mp[i]
            z[i] = minv[i] * g[i]
            gz_new += g[i] * z[i]
        beta = gz_new / gz
        for i in range(n):
            p[i] = z[i] + beta * p[i]
        gz = gz_new
        it += 1
    return it


# -- triangle grid queries (minihou.raycast.TriGrid) ---------------------------------------------------------------
@njit(**_OPTS)
def _closest_on_tri(px, py, pz, t):
    """Closest point on triangle t (3, 3) to p (Ericson 5.1.5, the same case analysis as raycast._closest_on_tri);
    returns the squared distance and the point."""
    ax, ay, az = t[0, 0], t[0, 1], t[0, 2]
    abx, aby, abz = t[1, 0] - ax, t[1, 1] - ay, t[1, 2] - az
    acx, acy, acz = t[2, 0] - ax, t[2, 1] - ay, t[2, 2] - az
    apx, apy, apz = px - ax, py - ay, pz - az
    d1 = abx * apx + aby * apy + abz * apz
    d2 = acx * apx + acy * apy + acz * apz
    rx, ry, rz = ax, ay, az
    if d1 <= 0.0 and d2 <= 0.0:
        pass
    else:
        bpx, bpy, bpz = px - t[1, 0], py - t[1, 1], pz - t[1, 2]
        d3 = abx * bpx + aby * bpy + abz * bpz
        d4 = acx * bpx + acy * bpy + acz * bpz
        if d3 >= 0.0 and d4 <= d3:
            rx, ry, rz = t[1, 0], t[1, 1], t[1, 2]
        else:
            vc = d1 * d4 - d3 * d2
            if vc <= 0.0 and d1 >= 0.0 and d3 <= 0.0:
                v = d1 / (d1 - d3)
                rx, ry, rz = ax + abx * v, ay + aby * v, az + abz * v
            else:
                cpx, cpy, cpz = px - t[2, 0], py - t[2, 1], pz - t[2, 2]
                d5 = abx * cpx + aby * cpy + abz * cpz
                d6 = acx * cpx + acy * cpy + acz * cpz
                if d6 >= 0.0 and d5 <= d6:
                    rx, ry, rz = t[2, 0], t[2, 1], t[2, 2]
                else:
                    vb = d5 * d2 - d1 * d6
                    if vb <= 0.0 and d2 >= 0.0 and d6 <= 0.0:
                        w = d2 / (d2 - d6)
                        rx, ry, rz = ax + acx * w, ay + acy * w, az + acz * w
                    else:
                        va = d3 * d6 - d5 * d4
                        if va <= 0.0 and (d4 - d3) >= 0.0 and (d5 - d6) >= 0.0:
                            w = (d4 - d3) / ((d4 - d3) + (d5 - d6))
                            rx = t[1, 0] + (t[2, 0] - t[1, 0]) * w
                            ry = t[1, 1] + (t[2, 1] - t[1, 1]) * w
                            rz = t[1, 2] + (t[2, 2] - t[1, 2]) * w
                        else:
                            den = 1.0 / (va + vb + vc)
                            v = vb * den
                            w = vc * den
                            rx = ax + abx * v + acx * w
                            ry = ay + aby * v + acy * w
                            rz = az + abz * v + acz * w
    dx, dy, dz = px - rx, py - ry, pz - rz
    return dx * dx + dy * dy + dz * dz, rx, ry, rz


@njit(parallel=True, **_OPTS)
def grid_closest(q, tris, cstart, ccount, cell_tris, lo, hv, dims, best_d, best_tri, best_p):
    """For each query the closest point on the mesh, searching rings of grid cells outward until nothing unsearched
    can be nearer. best_* hold an upper bound on entry (the nearest triangle corner) and the answer on exit."""
    m = q.shape[0]
    n1, n2 = dims[1], dims[2]
    for i in prange(m):
        px, py, pz = q[i, 0], q[i, 1], q[i, 2]
        p = (px, py, pz)
        qc = np.empty(3, np.int64)
        o2 = np.empty(3)
        for a in range(3):
            c = np.int64(np.floor((p[a] - lo[a]) / hv[a]))
            qc[a] = min(max(c, 0), dims[a] - 1)
            gap = max(max(lo[a] - p[a], p[a] - (lo[a] + hv[a] * dims[a])), 0.0)
            o2[a] = gap * gap
        o2_total = o2[0] + o2[1] + o2[2]
        bd2 = best_d[i] * best_d[i]
        bt = best_tri[i]
        bx, by, bz = best_p[i, 0], best_p[i, 1], best_p[i, 2]
        rmax = max(dims[0], max(dims[1], dims[2])) + 1
        r = 0
        while True:
            rx0 = r if dims[0] > 1 else 0
            ry0 = r if dims[1] > 1 else 0
            rz0 = r if dims[2] > 1 else 0
            for dx in range(-rx0, rx0 + 1):
                cx = qc[0] + dx
                if cx < 0 or cx >= dims[0]:
                    continue
                for dy in range(-ry0, ry0 + 1):
                    cy = qc[1] + dy
                    if cy < 0 or cy >= dims[1]:
                        continue
                    face = r == 0 or (rx0 > 0 and abs(dx) == r) or (ry0 > 0 and abs(dy) == r)
                    if not face and rz0 == 0:
                        continue
                    zstep = 1 if face else 2 * rz0              # off the x/y faces only the two z faces are on the ring
                    dz = -rz0
                    while dz <= rz0:
                        cz = qc[2] + dz
                        if cz >= 0 and cz < dims[2]:
                            # cell box farther than the best so far: skip it
                            gx = max(max(lo[0] + cx * hv[0] - px, px - (lo[0] + (cx + 1) * hv[0])), 0.0)
                            gy = max(max(lo[1] + cy * hv[1] - py, py - (lo[1] + (cy + 1) * hv[1])), 0.0)
                            gz = max(max(lo[2] + cz * hv[2] - pz, pz - (lo[2] + (cz + 1) * hv[2])), 0.0)
                            if gx * gx + gy * gy + gz * gz < bd2:
                                k = (cx * n1 + cy) * n2 + cz
                                s = cstart[k]
                                for j in range(s, s + ccount[k]):
                                    ti = cell_tris[j]
                                    d2, rx, ry, rz = _closest_on_tri(px, py, pz, tris[ti])
                                    if d2 < bd2:
                                        bd2, bt, bx, by, bz = d2, ti, rx, ry, rz
                        dz += zstep
            # squared lower bound on the distance to anything outside the searched (2r+1)-cube of cells
            bound = np.inf
            for a in range(3):
                if dims[a] <= 1:
                    continue
                box_lo = lo[a] + (qc[a] - r) * hv[a]
                box_hi = lo[a] + (qc[a] + r + 1) * hv[a]
                below = p[a] - box_lo if qc[a] - r > 0 else np.inf
                above = box_hi - p[a] if qc[a] + r < dims[a] - 1 else np.inf
                gap = max(min(below, above), 0.0)
                cand = gap * gap + (o2_total - o2[a])
                if cand < bound:
                    bound = cand
            if bd2 <= bound or r > rmax:
                break
            r += 1
        best_d[i] = np.sqrt(bd2)
        best_tri[i] = bt
        best_p[i, 0], best_p[i, 1], best_p[i, 2] = bx, by, bz


@njit(parallel=True, **_OPTS)
def grid_intersect(o, d, tris, cstart, ccount, cell_tris, lo, hi, hv, dims, max_dist, out_t, out_tri, out_uv):
    """Nearest hit along each ray: 3-D DDA through the grid with Moller-Trumbore on the triangles of each cell (both
    faces count). d must be unit length. The same traversal and tolerances as ``TriGrid._intersect_chunk``."""
    m = o.shape[0]
    n1, n2 = dims[1], dims[2]
    hmax = max(hv[0], max(hv[1], hv[2]))
    eps = 1e-9 * hmax
    for i in prange(m):
        oo = (o[i, 0], o[i, 1], o[i, 2])
        dd = (d[i, 0], d[i, 1], d[i, 2])
        tmin, tmax = -np.inf, np.inf
        miss = False
        for a in range(3):
            if dd[a] != 0.0:
                t1, t2 = (lo[a] - oo[a]) / dd[a], (hi[a] - oo[a]) / dd[a]
                tmin = max(tmin, min(t1, t2))
                tmax = min(tmax, max(t1, t2))
            elif oo[a] < lo[a] or oo[a] > hi[a]:
                miss = True
        t_enter = max(tmin, 0.0)
        out_t[i], out_tri[i], out_uv[i, 0], out_uv[i, 1] = np.inf, -1, 0.0, 0.0
        if miss or t_enter > tmax or t_enter > max_dist:
            continue
        cell = np.empty(3, np.int64)
        step = np.empty(3, np.int64)
        t_next = np.empty(3)
        t_delta = np.empty(3)
        for a in range(3):
            c = np.int64(np.floor((oo[a] + dd[a] * t_enter - lo[a]) / hv[a]))
            cell[a] = min(max(c, 0), dims[a] - 1)
            if dd[a] >= 0:
                step[a] = 1
            else:
                step[a] = -1
            if dd[a] != 0.0:
                bound = lo[a] + (cell[a] + (1 if step[a] > 0 else 0)) * hv[a]
                t_next[a] = (bound - oo[a]) / dd[a]
                t_delta[a] = hv[a] / abs(dd[a])
            else:
                t_next[a] = np.inf
                t_delta[a] = np.inf
        bt, btri, bu, bv = np.inf, -1, 0.0, 0.0
        for _ in range(dims[0] + dims[1] + dims[2] + 3):
            k = (cell[0] * n1 + cell[1]) * n2 + cell[2]
            s = cstart[k]
            for j in range(s, s + ccount[k]):
                ti = cell_tris[j]
                v0x, v0y, v0z = tris[ti, 0, 0], tris[ti, 0, 1], tris[ti, 0, 2]
                e1x, e1y, e1z = tris[ti, 1, 0] - v0x, tris[ti, 1, 1] - v0y, tris[ti, 1, 2] - v0z
                e2x, e2y, e2z = tris[ti, 2, 0] - v0x, tris[ti, 2, 1] - v0y, tris[ti, 2, 2] - v0z
                pvx = dd[1] * e2z - dd[2] * e2y
                pvy = dd[2] * e2x - dd[0] * e2z
                pvz = dd[0] * e2y - dd[1] * e2x
                det = e1x * pvx + e1y * pvy + e1z * pvz
                n1l = np.sqrt(e1x * e1x + e1y * e1y + e1z * e1z)
                n2l = np.sqrt(e2x * e2x + e2y * e2y + e2z * e2z)
                if abs(det) <= 1e-14 * max(n1l * n2l, 1e-300):
                    continue
                inv = 1.0 / det
                tvx, tvy, tvz = oo[0] - v0x, oo[1] - v0y, oo[2] - v0z
                u = (tvx * pvx + tvy * pvy + tvz * pvz) * inv
                if u < -1e-12 or u > 1.0 + 1e-12:
                    continue
                qx = tvy * e1z - tvz * e1y
                qy = tvz * e1x - tvx * e1z
                qz = tvx * e1y - tvy * e1x
                v = (dd[0] * qx + dd[1] * qy + dd[2] * qz) * inv
                if v < -1e-12 or u + v > 1.0 + 1e-12:
                    continue
                t = (e2x * qx + e2y * qy + e2z * qz) * inv
                if t >= 0.0 and t <= max_dist and t < bt:
                    bt, btri, bu, bv = t, ti, u, v
            t_out = min(t_next[0], min(t_next[1], t_next[2]))
            if bt <= t_out + eps or t_out > max_dist:
                break
            axis = 0
            if t_next[1] < t_next[axis]:
                axis = 1
            if t_next[2] < t_next[axis]:
                axis = 2
            cell[axis] += step[axis]
            t_next[axis] += t_delta[axis]
            if cell[axis] < 0 or cell[axis] >= dims[axis]:
                break
        out_t[i], out_tri[i], out_uv[i, 0], out_uv[i, 1] = bt, btri, bu, bv
