import numpy as np

from .. import accel
from ..geometry import Geometry
from ..sdk import Bool, Enum, Int, Str, node
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


def _tri_weights(geo, tris, tri_prim, name):
    """Per-triangle density from a primitive attribute, or the mean of a point attribute over its corners."""
    a = geo.prim.get(name)
    if a is not None and a.ndim == 1:
        return np.maximum(a.astype(np.float64)[tri_prim], 0.0)
    a = geo.point.get(name)
    if a is not None and a.ndim == 1:
        return np.maximum(a.astype(np.float64)[geo.topology.vertex_point[tris]].mean(1), 0.0)
    raise ValueError(f"no scalar primitive or point attribute {name!r} to scale the density by")


def _scatter_numpy(geo, count, seed, density_attrib="", group="", density=""):
    tris, tri_prim = fan_triangulate(geo.topology)
    if tris.shape[0] == 0:
        return _empty()
    pts = geo.topology.vertex_point[tris]                       # (T, 3) point indices
    P = geo.P.astype(np.float64, copy=False)
    a, b, c = P[pts[:, 0]], P[pts[:, 1]], P[pts[:, 2]]
    cross = np.cross(b - a, c - a)
    area = 0.5 * np.sqrt(np.einsum("ij,ij->i", cross, cross))
    if density_attrib:
        area = area * _tri_weights(geo, tris, tri_prim, density_attrib)
    from .. import groups
    selected = groups.mask(geo, "prim", group)
    if selected is not None:
        area *= selected[tri_prim]
    weights = None
    if density:
        if density in geo.point:
            values = np.asarray(geo.point[density], dtype=np.float64)
            if values.shape != (geo.npoints,):
                raise ValueError("scatter density must be a scalar attribute")
            weights = values[pts]
        elif density in geo.prim:
            values = np.asarray(geo.prim[density], dtype=np.float64)
            if values.shape != (geo.nprims,):
                raise ValueError("scatter density must be a scalar attribute")
            weights = np.repeat(values[tri_prim, None], 3, axis=1)
        else:
            raise ValueError(f"no point or primitive density attribute {density!r}")
        if not np.isfinite(weights).all() or (weights < 0).any():
            raise ValueError("scatter density must be finite and non-negative")
        area *= weights.mean(axis=1)
    cdf = np.cumsum(area)
    if cdf[-1] <= 0.0:
        return _empty()
    rng = np.random.default_rng(int(seed) & 0xFFFFFFFF)
    t = np.minimum(np.searchsorted(cdf, rng.random(count) * cdf[-1], side="right"), area.shape[0] - 1)
    r1, r2 = rng.random(count), rng.random(count)
    flip = r1 + r2 > 1.0
    r1 = np.where(flip, 1.0 - r1, r1)
    r2 = np.where(flip, 1.0 - r2, r2)
    if weights is not None:
        # A linear density over a triangle is a mixture of three Dirichlet(2,1,1)
        # distributions. This samples it exactly without rejection or a point-count shortfall.
        w = weights[t]
        corner = (rng.random(count)[:, None] * w.sum(axis=1)[:, None] > np.cumsum(w, axis=1)).sum(axis=1)
        alpha = np.ones((count, 3))
        alpha[np.arange(count), np.minimum(corner, 2)] = 2
        bary = rng.gamma(alpha)
        bary /= bary.sum(axis=1)[:, None]
        r1, r2 = bary[:, 1], bary[:, 2]
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


@node("scatter", inputs=["geo"], category="generate", version=2, params=[
    Int("count", 1000, min=0, soft_max=1000000, proxy="count"),
    Int("seed", 0),
    Enum("mode", ["surface", "volume"], doc="surface: on the faces (area weighted); volume: inside a closed mesh"),
    Str("density_attrib", "", doc="surface: scalar primitive (or point) attribute that scales the density: faces with 2 get twice the points, "
                                 "0 none"),
    Int("relax_iterations", 0, min=0, soft_max=50, doc="Push the points apart this many times and back onto the surface (an even, blue-noise-like "
                                                    "spread; see point_relax)"),
    Bool("output_id", False, doc="Write the point attribute id (0, 1, 2...)"),
    Str("group", "", doc="Surface primitives to scatter on"),
    Str("density", "", doc="Non-negative scalar point or primitive attribute controlling surface density; point values interpolate linearly"),
])
def scatter(geo, count, seed, mode="surface", density_attrib="", relax_iterations=0, output_id=False, group="", density=""):
    """Scatter points uniformly over the surface (area weighted), or inside a closed mesh.

    Output is a point cloud with ``P``, ``N`` (face normal; zero in volume mode) and ``primnum`` (source primitive;
    -1 in volume mode). Primitives are fan-triangulated for sampling. An input with no area yields an empty cloud.
    Uses a Numba kernel when available; the numpy path draws the same random numbers in the same order.
    """
    if density_attrib and density:
        raise ValueError("choose either density or density_attrib, not both")
    if count == 0 or geo.nprims == 0:
        return _empty()
    if mode == "volume":
        if group or density:
            raise ValueError("scatter group and density apply to surface mode only")
        from ._build import require_polygons
        require_polygons(geo, "scatter in volume mode")
        return _scatter_volume(geo, count, seed)
    out = None if density_attrib or group or density else _scatter_numba(geo, count, seed)
    if out is None:
        out = _scatter_numpy(geo, count, seed, density_attrib, group, density)
    if relax_iterations and out.npoints > 1:
        out = _relax(out, geo, relax_iterations)
    if output_id:
        out = out.copy()
        out.point["id"] = np.arange(out.npoints, dtype=np.int32)
    return out


def _relax(points, surface, iterations):
    from ..sdk import REGISTRY
    nt = REGISTRY.get("point_relax")
    return nt.fn(points, surface, **nt.coerce_params({"iterations": iterations}))
