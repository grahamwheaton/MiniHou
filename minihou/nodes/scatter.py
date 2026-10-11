import numpy as np

from .. import accel
from ..geometry import Geometry
from ..sdk import Enum, Int, node
from ._util import fan_triangulate, normalize_rows


def _empty():
    g = Geometry.from_arrays(np.zeros((0, 3), np.float32))
    g.point["N"] = np.zeros((0, 3), np.float32)
    g.point["primnum"] = np.zeros(0, np.int32)
    return g


def _finish(pos, normals, prim):
    out = Geometry.from_arrays(pos)
    out.point["N"] = normals
    out.point["primnum"] = prim
    return out


def _scatter_numba(geo, count, seed):
    """Fused kernels; returns None if acceleration is unavailable or failed (caller falls back)."""
    k = accel.kernels()
    if k is None:
        return None
    offs = geo.topology.prim_offsets
    counts = np.maximum(np.diff(offs.astype(np.int64)) - 2, 0)
    ntri = int(counts.sum())
    if ntri == 0:
        return _empty()
    first = np.cumsum(counts) - counts
    P = np.ascontiguousarray(geo.P)
    vp = np.ascontiguousarray(geo.topology.vertex_point)
    area = np.empty(ntri, np.float64)
    tri_prim = np.empty(ntri, np.int32)
    if accel.call(k.tri_areas, P, vp, offs, counts, first, area, tri_prim) is accel.FAILED:
        return None
    cdf = np.cumsum(area)
    if cdf[-1] <= 0.0:
        return _empty()
    rng = np.random.default_rng(int(seed) & 0xFFFFFFFF)   # same draws, same order as the numpy path
    r, r1, r2 = rng.random(count), rng.random(count), rng.random(count)
    pos = np.empty((count, 3), np.float32)
    nrm = np.empty((count, 3), np.float32)
    prim = np.empty(count, np.int32)
    if accel.call(k.sample_surface, P, vp, offs, first, tri_prim, cdf, r, r1, r2, pos, nrm, prim) is accel.FAILED:
        return None
    return _finish(pos, nrm, prim)


def _scatter_numpy(geo, count, seed):
    tris, tri_prim = fan_triangulate(geo.topology)
    if tris.shape[0] == 0:
        return _empty()
    pts = geo.topology.vertex_point[tris]                       # (T, 3) point indices
    P = geo.P.astype(np.float64, copy=False)
    a, b, c = P[pts[:, 0]], P[pts[:, 1]], P[pts[:, 2]]
    cross = np.cross(b - a, c - a)
    area = 0.5 * np.sqrt(np.einsum("ij,ij->i", cross, cross))
    cdf = np.cumsum(area)
    if cdf[-1] <= 0.0:
        return _empty()
    rng = np.random.default_rng(int(seed) & 0xFFFFFFFF)
    t = np.minimum(np.searchsorted(cdf, rng.random(count) * cdf[-1], side="right"), area.shape[0] - 1)
    r1, r2 = rng.random(count), rng.random(count)
    flip = r1 + r2 > 1.0
    r1 = np.where(flip, 1.0 - r1, r1)
    r2 = np.where(flip, 1.0 - r2, r2)
    pos = a[t] + (b[t] - a[t]) * r1[:, None] + (c[t] - a[t]) * r2[:, None]
    return _finish(pos.astype(np.float32), normalize_rows(cross[t].astype(np.float32)),
                   tri_prim[t].astype(np.int32))


def _scatter_volume(geo, count, seed):
    """Uniform points inside a closed mesh: rejection sampling of its bounding box with a ray-parity inside test."""
    from ..raycast import TriGrid, inside
    tris, _ = fan_triangulate(geo.topology)
    if len(tris) == 0:
        return _empty()
    P = geo.P.astype(np.float64)
    grid = TriGrid(P[geo.topology.vertex_point[tris]])
    lo, hi = P.min(0), P.max(0)
    ext = hi - lo
    rng = np.random.default_rng([int(seed) & 0xFFFFFFFF, 11])
    found, have, frac = [], 0, 0.5
    for _ in range(64):
        if have >= count:
            break
        n = int(min(max(1024, (count - have) / max(frac, 0.02) * 1.25), 2_000_000))
        cand = lo + rng.random((n, 3)) * ext
        m = inside(grid, cand, float(ext.max()))
        frac = max(float(m.mean()), 1e-3)
        found.append(cand[m])
        have += int(m.sum())
    pos = np.concatenate(found)[:count] if found else np.zeros((0, 3))
    if len(pos) == 0:
        return _empty()
    return _finish(pos.astype(np.float32), np.zeros((len(pos), 3), np.float32), np.full(len(pos), -1, np.int32))


@node("scatter", inputs=["geo"], category="generate", params=[
    Int("count", 1000, min=0, soft_max=1000000),
    Int("seed", 0),
    Enum("mode", ["surface", "volume"], doc="surface: on the faces (area weighted); volume: inside a closed mesh"),
])
def scatter(geo, count, seed, mode="surface"):
    """Scatter points uniformly over the surface (area weighted), or inside a closed mesh.

    Output is a point cloud with ``P``, ``N`` (face normal; zero in volume mode) and ``primnum`` (source primitive;
    -1 in volume mode). Primitives are fan-triangulated for sampling. An input with no area yields an empty cloud.
    Uses a Numba kernel when available; the numpy path draws the same random numbers in the same order.
    """
    if count == 0 or geo.nprims == 0:
        return _empty()
    if mode == "volume":
        from ._build import require_polygons
        require_polygons(geo, "scatter in volume mode")
        return _scatter_volume(geo, count, seed)
    out = _scatter_numba(geo, count, seed)
    return out if out is not None else _scatter_numpy(geo, count, seed)
