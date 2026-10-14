"""Helpers shared by built-in nodes (numpy only)."""
from __future__ import annotations

import numpy as np

from .. import accel
from ..geometry import Topology


def euler_xyz_matrix(rotate_deg) -> np.ndarray:
    """3x3 rotation matrix for XYZ Euler degrees (X applied first): R = Rz @ Ry @ Rx."""
    rx, ry, rz = np.radians(np.asarray(rotate_deg, dtype=np.float64))
    cx, sx, cy, sy, cz, sz = np.cos(rx), np.sin(rx), np.cos(ry), np.sin(ry), np.cos(rz), np.sin(rz)
    Rx = np.array([[1, 0, 0], [0, cx, -sx], [0, sx, cx]])
    Ry = np.array([[cy, 0, sy], [0, 1, 0], [-sy, 0, cy]])
    Rz = np.array([[cz, -sz, 0], [sz, cz, 0], [0, 0, 1]])
    return Rz @ Ry @ Rx


def normalize_rows(v: np.ndarray, eps: float = 1e-20) -> np.ndarray:
    n = np.sqrt(np.einsum("ij,ij->i", v, v))
    n = np.where(n < eps, 1.0, n)
    return (v / n[:, None]).astype(np.float32, copy=False)


def fan_triangulate(topo: Topology):
    """Fan-triangulate every primitive.

    Returns ``(tri_vertices, tri_prim)``: ``tri_vertices`` is ``(T, 3)`` of *vertex* indices
    (index through ``topo.vertex_point`` for points) and ``tri_prim`` the owning primitive.
    Primitives with fewer than 3 vertices produce no triangles.
    """
    offs = topo.prim_offsets.astype(np.int64)
    counts = np.maximum(np.diff(offs) - 2, 0)
    total = int(counts.sum())
    if total == 0:
        return np.zeros((0, 3), np.int64), np.zeros(0, np.int64)
    tri_prim = np.repeat(np.arange(counts.shape[0], dtype=np.int64), counts)
    first_tri = np.cumsum(counts) - counts
    local = np.arange(total, dtype=np.int64) - np.repeat(first_tri, counts)
    v0 = offs[tri_prim]
    return np.stack([v0, v0 + local + 1, v0 + local + 2], axis=1), tri_prim


def rotation_from_z_to(n: np.ndarray) -> np.ndarray:
    """Minimal rotations taking +Z onto each unit vector in ``n`` (shape (k, 3)) -> (k, 3, 3)."""
    n = n.astype(np.float64)
    k = n.shape[0]
    R = np.tile(np.eye(3), (k, 1, 1))
    c = n[:, 2]                                   # z . n
    # axis = z x n = (-ny, nx, 0)
    vx, vy = -n[:, 1], n[:, 0]
    anti = c < -1.0 + 1e-9                        # n ~ -Z: rotate 180 degrees about X
    ok = ~anti
    if ok.any():
        s = 1.0 / (1.0 + c[ok])
        x, y = vx[ok], vy[ok]
        R[ok, 0, 0] = 1.0 - s * y * y
        R[ok, 0, 1] = s * x * y
        R[ok, 0, 2] = y
        R[ok, 1, 0] = s * x * y
        R[ok, 1, 1] = 1.0 - s * x * x
        R[ok, 1, 2] = -x
        R[ok, 2, 0] = -y
        R[ok, 2, 1] = x
        R[ok, 2, 2] = 1.0 - s * (x * x + y * y)
    if anti.any():
        R[anti] = np.diag([1.0, -1.0, -1.0])
    return R


# -- gradient (Perlin) noise ------------------------------------------------------------------
_U32 = np.uint32


def _hash3(ix, iy, iz, seed: int) -> np.ndarray:
    """Deterministic uint32 lattice hash (wraps on overflow by design)."""
    h = ix.astype(_U32) * _U32(0x8DA6B343)
    h ^= iy.astype(_U32) * _U32(0xD8163841)
    h ^= iz.astype(_U32) * _U32(0xCB1AB31F)
    h ^= _U32((int(seed) * 0x9E3779B1) & 0xFFFFFFFF)
    h ^= h >> _U32(15)
    h *= _U32(0x2C1B3C6D)
    h ^= h >> _U32(12)
    h *= _U32(0x297A2D39)
    h ^= h >> _U32(15)
    return h


def _grad(h, x, y, z):
    h = h & _U32(15)
    u = np.where(h < 8, x, y)
    v = np.where(h < 4, y, np.where((h == 12) | (h == 14), x, z))
    return np.where(h & _U32(1), -u, u) + np.where(h & _U32(2), -v, v)


def perlin3(p: np.ndarray, seed: int = 0) -> np.ndarray:
    """Improved-Perlin gradient noise at ``p`` (n, 3). Returns float32 (n,), roughly in [-1, 1]."""
    p = p.astype(np.float32, copy=False)
    fl = np.floor(p)
    f = p - fl
    i = fl.astype(np.int32)
    u = f * f * f * (f * (f * np.float32(6) - np.float32(15)) + np.float32(10))
    fx, fy, fz = f[:, 0], f[:, 1], f[:, 2]
    ux, uy, uz = u[:, 0], u[:, 1], u[:, 2]
    ix, iy, iz = i[:, 0], i[:, 1], i[:, 2]

    def corner(dx, dy, dz):
        return _grad(_hash3(ix + dx, iy + dy, iz + dz, seed), fx - dx, fy - dy, fz - dz)

    def lerp(a, b, t):
        return a + t * (b - a)

    x00 = lerp(corner(0, 0, 0), corner(1, 0, 0), ux)
    x10 = lerp(corner(0, 1, 0), corner(1, 1, 0), ux)
    x01 = lerp(corner(0, 0, 1), corner(1, 0, 1), ux)
    x11 = lerp(corner(0, 1, 1), corner(1, 1, 1), ux)
    return lerp(lerp(x00, x10, uy), lerp(x01, x11, uy), uz).astype(np.float32, copy=False)


def fbm3(p: np.ndarray, seed: int, octaves: int, lacunarity: float, gain: float) -> np.ndarray:
    """Fractal sum of Perlin octaves, normalised so the amplitude stays near [-1, 1].

    Uses the Numba kernel when available (same algorithm, parallel), else :func:`fbm3_numpy`. Only the low
    32 bits of ``seed`` matter to the hash, so it is reduced first (also keeps it in int64 range).
    """
    from .. import rust
    if rust.enabled():
        return rust.fbm3(p, int(seed) & 0xFFFFFFFF, octaves, lacunarity, gain)
    k = accel.kernels()
    if k is not None:
        pos = np.ascontiguousarray(p, dtype=np.float32)
        out = np.empty(pos.shape[0], np.float32)
        if accel.call(k.fbm3, pos, int(seed) & 0xFFFFFFFF, int(octaves), float(lacunarity), float(gain), out) \
                is not accel.FAILED:
            return out
    return fbm3_numpy(p, seed, octaves, lacunarity, gain)


def fbm3_numpy(p: np.ndarray, seed: int, octaves: int, lacunarity: float, gain: float) -> np.ndarray:
    """Pure-numpy implementation (the reference the kernel is tested against)."""
    total = np.zeros(p.shape[0], np.float32)
    amp, freq, norm = 1.0, 1.0, 0.0
    for o in range(octaves):
        total += np.float32(amp) * perlin3(p * np.float32(freq), seed + o * 7919)
        norm += amp
        amp *= gain
        freq *= lacunarity
    return total / np.float32(norm) if norm > 0 else total


MAX_GENERATED = 20_000_000


def guard_count(n, what: str, hint: str = "use a larger spacing") -> None:
    """Refuse to build absurdly many elements: a tiny spacing would exhaust memory long before it finished."""
    if not (n <= MAX_GENERATED):                                   # also catches inf and nan
        raise ValueError(f"{what} would make about {n:,.0f} elements; the limit is {MAX_GENERATED:,} ({hint})")
