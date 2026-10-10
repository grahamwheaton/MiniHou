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
