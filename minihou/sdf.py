"""Signed distance fields on dense voxel grids (the engine's take on Houdini's VDB workflow), numpy only.

* :func:`from_mesh`    -- exact distances in a narrow band around a *closed* mesh (closest points via
  :mod:`minihou.raycast`), signed by flood-filling the outside from the grid border with the surface as a wall
* :func:`from_points`  -- a metaball field around points (smooth blending), as signed-distance-like values
* :func:`blur`, booleans with ``np.minimum`` / ``np.maximum`` (or the smooth variants), offsets by adding a constant
* :func:`surface_nets` -- the zero level as a quad mesh (naive surface nets: one point per surface cell at the mean of
  its edge crossings, one quad per crossed grid edge), with consistent outward winding

Negative is inside. Values far from the surface are clamped to +-``band``.
"""
from __future__ import annotations

from dataclasses import dataclass

import numpy as np

MAX_VOXELS = 24_000_000
MAX_POINT_WORK = 150_000_000          # points x kernel voxels for metaballs
MAX_MESH_WORK = 15_000_000            # (triangle, voxel) pairs for a mesh SDF: about half a minute of closest-point queries


@dataclass
class Volume:
    origin: np.ndarray        # world position of voxel (0, 0, 0)
    h: float                  # voxel size
    values: np.ndarray        # (nx, ny, nz) float32, negative inside

    def positions(self, idx: np.ndarray) -> np.ndarray:
        return self.origin + idx * self.h


def grid_for(lo, hi, h, pad):
    lo = np.asarray(lo, np.float64) - pad
    hi = np.asarray(hi, np.float64) + pad
    dims = np.floor((hi - lo) / h).astype(np.int64) + 2
    total = int(np.prod(dims))
    if total > MAX_VOXELS:
        raise ValueError(f"{dims[0]} x {dims[1]} x {dims[2]} = {total:,} voxels is too many; use a larger voxel size")
    return lo, dims


# -- from a mesh ------------------------------------------------------------------------------------------------
def mesh_work(tris: np.ndarray, h: float, band_voxels: float = 3.0) -> int:
    """How many (triangle, voxel) pairs from_mesh would have to examine: the bounding boxes of the triangles, grown by the
    band, in voxels. Cheap, so callers can refuse hopeless voxel sizes before spending time."""
    tris = np.asarray(tris, np.float64)
    if not len(tris):
        return 0
    span = np.ceil((tris.max(1) - tris.min(1) + 2 * band_voxels * h) / h) + 1
    return int(span.prod(1).sum())


def from_mesh(tris: np.ndarray, h: float, band_voxels: float = 3.0, pad_voxels: int = 2, grid=None) -> Volume:
    """SDF of a closed triangle mesh (T, 3, 3), on its own padded grid or on ``grid`` = (origin, dims)."""
    from .raycast import TriGrid
    tris = np.asarray(tris, np.float64)
    band = band_voxels * h
    if grid is not None:
        lo, dims = np.asarray(grid[0], np.float64), np.asarray(grid[1], np.int64)
    elif len(tris):
        lo, dims = grid_for(tris.reshape(-1, 3).min(0), tris.reshape(-1, 3).max(0), h, band + pad_voxels * h)
    else:                                               # nothing to measure: a tiny all-outside grid
        lo, dims = np.zeros(3), np.array([2, 2, 2], np.int64)
    if len(tris) == 0:
        return Volume(lo, float(h), np.full(tuple(dims), band, np.float32))
    nx, ny, nz = dims
    # voxels within the band of some triangle's bounding box: candidates for exact distances
    tlo = np.floor((tris.min(1) - band - lo) / h).astype(np.int64)
    thi = np.ceil((tris.max(1) + band - lo) / h).astype(np.int64)
    tlo, thi = np.clip(tlo, 0, dims - 1), np.clip(thi, 0, dims - 1)
    span = thi - tlo + 1
    cnt = span.prod(1)
    if int(cnt.sum()) > 4 * MAX_VOXELS:
        raise ValueError("the mesh has triangles much larger than the voxel size; use a larger voxel size")
    tri_id = np.repeat(np.arange(len(tris)), cnt)
    local = np.arange(cnt.sum()) - np.repeat(np.cumsum(cnt) - cnt, cnt)
    sp = span[tri_id]
    ijk = tlo[tri_id] + np.stack([local // (sp[:, 1] * sp[:, 2]), (local // sp[:, 2]) % sp[:, 1], local % sp[:, 2]], 1)
    flat = np.unique((ijk[:, 0] * ny + ijk[:, 1]) * nz + ijk[:, 2])
    idx = np.stack([flat // (ny * nz), (flat // nz) % ny, flat % nz], 1)
    pts = lo + idx * h
    grid = TriGrid(tris)
    cp, dist, tri = grid.closest(pts)
    unsigned = np.full(int(np.prod(dims)), band, np.float32)
    unsigned[flat] = np.minimum(dist, band)
    unsigned = unsigned.reshape(dims)
    # inside/outside: flood the outside from the border through voxels the surface does not pass through
    wall = unsigned < h * 0.8660254 + 1e-12                                 # the surface can cross these voxels
    outside = _flood(~wall)
    sign = np.where(outside, 1.0, -1.0)
    # wall voxels: the side of the nearest triangle (its normal) decides
    wi = np.flatnonzero(wall.reshape(-1))
    if len(wi):
        where = np.searchsorted(flat, wi)
        t = tris[tri[where]]
        n = np.cross(t[:, 1] - t[:, 0], t[:, 2] - t[:, 0])
        side = np.einsum("ij,ij->i", pts[where] - cp[where], n)
        sign.reshape(-1)[wi] = np.where(side >= 0, 1.0, -1.0)
    return Volume(lo, float(h), (sign * unsigned).astype(np.float32))


def _flood(free: np.ndarray) -> np.ndarray:
    """Voxels of ``free`` connected (6-neighbourhood) to the grid border."""
    try:
        from scipy import ndimage
        seed = np.zeros_like(free)
        seed[[0, -1], :, :] = seed[:, [0, -1], :] = seed[:, :, [0, -1]] = True
        return ndimage.binary_propagation(seed & free, mask=free)
    except ImportError:
        pass
    reach = np.zeros_like(free)
    reach[[0, -1], :, :] = reach[:, [0, -1], :] = reach[:, :, [0, -1]] = True
    reach &= free
    while True:                                   # dilate within 'free' until nothing changes
        grown = reach.copy()
        grown[1:] |= reach[:-1]
        grown[:-1] |= reach[1:]
        grown[:, 1:] |= reach[:, :-1]
        grown[:, :-1] |= reach[:, 1:]
        grown[:, :, 1:] |= reach[:, :, :-1]
        grown[:, :, :-1] |= reach[:, :, 1:]
        grown &= free
        if np.array_equal(grown, reach):
            return reach
        reach = grown


# -- from points ------------------------------------------------------------------------------------------------
def from_points(P: np.ndarray, radius: np.ndarray, h: float, blend: float = 1.0) -> Volume:
    """Metaballs: each point adds a smooth kernel reaching radius x (1 + blend); the threshold is chosen so a lone
    point gives a sphere of exactly its radius, and nearby points melt together (more with a larger blend)."""
    P = np.asarray(P, np.float64).reshape(-1, 3)
    r = np.broadcast_to(np.asarray(radius, np.float64), (len(P),)).astype(np.float64)
    R = r * (1.0 + max(blend, 0.0))                     # kernel support
    lo, dims = grid_for(P.min(0) - R.max(), P.max(0) + R.max(), h, 2 * h)
    reach = int(np.ceil(R.max() / h)) + 1
    if len(P) * (2 * reach + 2) ** 3 > MAX_POINT_WORK:
        raise ValueError(f"{len(P):,} points with kernels {2 * reach} voxels wide is too much work; use a larger voxel_size "
                         f"or a smaller radius")
    field = np.zeros(int(np.prod(dims)), np.float64)
    # kernel k(q) = (1 - q^2)^3 for q = d / R < 1. A lone point's surface at d = r: threshold = k(r / R)
    thr = (1.0 - (1.0 / (1.0 + max(blend, 0.0))) ** 2) ** 3
    m = int(np.ceil(R.max() / h)) + 1
    base = np.floor((P - lo) / h).astype(np.int64)
    rng = np.arange(-m, m + 2)
    offs = np.stack(np.meshgrid(rng, rng, rng, indexing="ij"), -1).reshape(-1, 3)
    for chunk in np.array_split(offs, max(1, len(offs) // 64)):
        ijk = base[:, None, :] + chunk[None, :, :]
        ok = np.all((ijk >= 0) & (ijk < dims), axis=2)
        pos = lo + ijk * h
        q2 = ((pos - P[:, None, :]) ** 2).sum(2) / (R[:, None] ** 2)
        ok &= q2 < 1.0
        w = (1.0 - q2[ok]) ** 3
        flat = ((ijk[..., 0] * dims[1] + ijk[..., 1]) * dims[2] + ijk[..., 2])[ok]
        np.add.at(field, flat, w)
    # an SDF-like value: negative inside, scaled to roughly world units near the surface
    vals = (thr - field.reshape(dims)) * (r.mean() / max(thr, 1e-6))
    return Volume(lo, float(h), vals.astype(np.float32))


# -- operations -------------------------------------------------------------------------------------------------
def blur(vol: Volume, iterations: int) -> Volume:
    """Mean-curvature-like smoothing: repeated 3-tap box blur along each axis."""
    v = vol.values.astype(np.float32)
    for _ in range(iterations):
        for ax in range(3):
            pad = np.pad(v, [(1, 1) if a == ax else (0, 0) for a in range(3)], mode="edge")
            sl = [slice(None)] * 3
            a, b, c = sl.copy(), sl.copy(), sl.copy()
            a[ax], b[ax], c[ax] = slice(0, -2), slice(1, -1), slice(2, None)
            v = (pad[tuple(a)] + pad[tuple(b)] + pad[tuple(c)]) / 3.0
    return Volume(vol.origin, vol.h, v.astype(np.float32))


def resample_to(vol: Volume, origin: np.ndarray, dims, h: float, fill: float) -> np.ndarray:
    """Values of ``vol`` on another grid (same voxel size, integer-aligned): pads with ``fill``."""
    off = np.round((np.asarray(origin) - vol.origin) / h).astype(np.int64)
    out = np.full(tuple(dims), fill, np.float32)
    src = vol.values
    a0 = np.maximum(off, 0)
    a1 = np.minimum(off + np.asarray(dims), src.shape)
    if np.any(a1 <= a0):
        return out
    b0 = a0 - off
    b1 = a1 - off
    out[b0[0]:b1[0], b0[1]:b1[1], b0[2]:b1[2]] = src[a0[0]:a1[0], a0[1]:a1[1], a0[2]:a1[2]]
    return out


def smooth_min(a, b, k):
    if k <= 0:
        return np.minimum(a, b)
    h = np.clip(0.5 + 0.5 * (b - a) / k, 0.0, 1.0)
    return b * (1 - h) + a * h - k * h * (1 - h)


# -- meshing ----------------------------------------------------------------------------------------------------
def _resolve_face_saddles(v: np.ndarray, max_rounds: int = 16) -> np.ndarray:
    """Remove face saddles (a cell face whose corners alternate inside/outside diagonally): there all four face edges
    are crossed and four quads would share one mesh edge (non-manifold). The corner nearest to zero is nudged across;
    it changes the field by less than its own tiny value. Repeated in case a nudge creates a saddle next door."""
    v = v.copy()
    for _ in range(max_rounds):
        found = False
        s = v < 0
        for ax in range(3):
            o0, o1 = [o for o in range(3) if o != ax]

            def c(d0, d1):
                sl = [slice(None)] * 3
                sl[o0] = slice(d0, v.shape[o0] - 1 + d0)
                sl[o1] = slice(d1, v.shape[o1] - 1 + d1)
                return tuple(sl)

            a, b, cc, d = s[c(0, 0)], s[c(1, 0)], s[c(1, 1)], s[c(0, 1)]
            saddle = (a == cc) & (b == d) & (a != b)
            if not saddle.any():
                continue
            found = True
            idx = np.argwhere(saddle)
            corners = []
            for d0, d1 in ((0, 0), (1, 0), (1, 1), (0, 1)):
                k = idx.copy()
                k[:, o0] += d0
                k[:, o1] += d1
                corners.append(k)
            vals = np.stack([np.abs(v[tuple(k.T)]) for k in corners], 1)
            pick = np.argmin(vals, axis=1)
            target = np.stack(corners, 1)[np.arange(len(idx)), pick]
            t = tuple(target.T)
            v[t] = np.where(v[t] < 0, 1e-7, -1e-7) * max(1.0, float(np.abs(v).max()) * 1e-6)
        if not found:
            break
    return v


def surface_nets(vol: Volume):
    """(points (n, 3), quads (m, 4) point indices) of the zero level, quads wound counter-clockwise seen from outside.
    Edge-manifold: face saddles are resolved first."""
    v = _resolve_face_saddles(vol.values.astype(np.float64))
    nx, ny, nz = v.shape
    inside = v < 0
    cells = (nx - 1, ny - 1, nz - 1)
    if min(cells) < 1:
        return np.zeros((0, 3)), np.zeros((0, 4), np.int64)
    acc = np.zeros(cells + (3,))
    cnt = np.zeros(cells)
    edges = []                                    # (axis, crossing mask over grid edges)
    for ax in range(3):
        sl0 = [slice(None)] * 3
        sl1 = [slice(None)] * 3
        sl0[ax], sl1[ax] = slice(0, -1), slice(1, None)
        a, b = v[tuple(sl0)], v[tuple(sl1)]
        cross = (a < 0) != (b < 0)
        edges.append((ax, cross, a < 0))
        t = np.where(cross, a / np.where(cross, a - b, 1.0), 0.0)
        idx = np.argwhere(cross)
        if not len(idx):
            continue
        p = idx.astype(np.float64)
        p[:, ax] += t[cross]
        # each crossed edge touches up to 4 cells: those with the edge on their boundary
        others = [o for o in range(3) if o != ax]
        for d0 in (0, -1):
            for d1 in (0, -1):
                c = idx.copy()
                c[:, others[0]] += d0
                c[:, others[1]] += d1
                ok = np.all((c >= 0) & (c < np.array(cells)), axis=1)
                np.add.at(acc, (c[ok, 0], c[ok, 1], c[ok, 2]), p[ok])
                np.add.at(cnt, (c[ok, 0], c[ok, 1], c[ok, 2]), 1)
    active = cnt > 0
    vid = np.full(cells, -1, np.int64)
    vid[active] = np.arange(int(active.sum()))
    pts = vol.origin + (acc[active] / cnt[active][:, None]) * vol.h
    quads = []
    for ax, cross, a_in in edges:
        idx = np.argwhere(cross)
        others = [o for o in range(3) if o != ax]
        # interior edges only: all four surrounding cells exist
        ok = np.all([(idx[:, o] >= 1) & (idx[:, o] < cells[o]) for o in others], axis=0)
        ok &= idx[:, ax] < cells[ax]
        idx = idx[ok]
        if not len(idx):
            continue
        inside_first = a_in[tuple(idx.T)]
        ring = []
        for d0, d1 in ((-1, -1), (0, -1), (0, 0), (-1, 0)):
            c = idx.copy()
            c[:, others[0]] += d0
            c[:, others[1]] += d1
            ring.append(vid[c[:, 0], c[:, 1], c[:, 2]])
        q = np.stack(ring, 1)
        # orientation: around +axis the ring (o0, o1) runs counter-clockwise when (o0, o1, ax) is right-handed
        right_handed = (others[0] + 1) % 3 == others[1]
        keep = inside_first ^ (not right_handed)            # inside -> outside along +axis: ring order is outward
        q = np.where(keep[:, None], q, q[:, ::-1])
        quads.append(q)
    quads = np.concatenate(quads) if quads else np.zeros((0, 4), np.int64)
    return pts, quads
