import numpy as np

from .. import accel
from ..geometry import Geometry
from ..sdk import Int, node
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
    rng = np.random.default_rng(seed)                     # same draws, same order as the numpy path
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
    rng = np.random.default_rng(seed)
    t = np.minimum(np.searchsorted(cdf, rng.random(count) * cdf[-1], side="right"), area.shape[0] - 1)
    r1, r2 = rng.random(count), rng.random(count)
    flip = r1 + r2 > 1.0
    r1 = np.where(flip, 1.0 - r1, r1)
    r2 = np.where(flip, 1.0 - r2, r2)
    pos = a[t] + (b[t] - a[t]) * r1[:, None] + (c[t] - a[t]) * r2[:, None]
    return _finish(pos.astype(np.float32), normalize_rows(cross[t].astype(np.float32)),
                   tri_prim[t].astype(np.int32))


@node("scatter", inputs=["geo"], category="generate", params=[
    Int("count", 1000, min=0, soft_max=1000000),
    Int("seed", 0),
])
def scatter(geo, count, seed):
    """Scatter points uniformly over the surface (area weighted).

    Output is a point cloud with ``P``, ``N`` (face normal) and ``primnum`` (source primitive).
    Primitives are fan-triangulated for sampling. An input with no area yields an empty cloud.
    Uses a Numba kernel when available; the numpy path draws the same random numbers in the same order.
    """
    if count == 0 or geo.nprims == 0:
        return _empty()
    out = _scatter_numba(geo, count, seed)
    return out if out is not None else _scatter_numpy(geo, count, seed)
