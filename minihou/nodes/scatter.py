import numpy as np

from ..geometry import Geometry
from ..sdk import Int, node
from ._util import fan_triangulate, normalize_rows


@node("scatter", inputs=["geo"], category="generate", params=[
    Int("count", 1000, min=0, soft_max=1000000),
    Int("seed", 0),
])
def scatter(geo, count, seed):
    """Scatter points uniformly over the surface (area weighted).

    Output is a point cloud with ``P``, ``N`` (face normal) and ``primnum`` (source primitive).
    Primitives are fan-triangulated for sampling. An input with no area yields an empty cloud.
    """
    tris, tri_prim = fan_triangulate(geo.topology)
    empty = Geometry.from_arrays(np.zeros((0, 3), np.float32))
    empty.point["N"] = np.zeros((0, 3), np.float32)
    empty.point["primnum"] = np.zeros(0, np.int32)
    if tris.shape[0] == 0 or count == 0:
        return empty
    pts = geo.topology.vertex_point[tris]                       # (T, 3) point indices
    P = geo.P.astype(np.float64, copy=False)
    a, b, c = P[pts[:, 0]], P[pts[:, 1]], P[pts[:, 2]]
    cross = np.cross(b - a, c - a)
    area = 0.5 * np.sqrt(np.einsum("ij,ij->i", cross, cross))
    cdf = np.cumsum(area)
    if cdf[-1] <= 0.0:
        return empty
    rng = np.random.default_rng(seed)
    t = np.minimum(np.searchsorted(cdf, rng.random(count) * cdf[-1], side="right"), area.shape[0] - 1)
    r1, r2 = rng.random(count), rng.random(count)
    flip = r1 + r2 > 1.0
    r1 = np.where(flip, 1.0 - r1, r1)
    r2 = np.where(flip, 1.0 - r2, r2)
    pos = a[t] + (b[t] - a[t]) * r1[:, None] + (c[t] - a[t]) * r2[:, None]
    out = Geometry.from_arrays(pos.astype(np.float32))
    out.point["N"] = normalize_rows(cross[t].astype(np.float32))
    out.point["primnum"] = tri_prim[t].astype(np.int32)
    return out
