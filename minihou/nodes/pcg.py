"""PCG nodes: point samplers, point booleans, density tools and spawning, like Unreal Engine's PCG graph.

Every node reads and writes PCG points (see :mod:`minihou.pcg`): positions with a rotation, scale, box bounds,
density and seed. Outputs are point clouds, so they chain freely and merge with ``merge`` or ``pcg_union``;
``pcg_debug`` shows the boxes, ``pcg_spawner`` hands the points to Blender as instances, and ``copy_to_points``
copies engine geometry onto them (it reads ``rot`` and ``scale``).
"""
import numpy as np

from .. import pcg, spatial
from ..geometry import Geometry
from ..sdk import Bool, Enum, Float, Input, Int, Str, Vec3, node
from . import _build as B
from ._util import fan_triangulate, fbm3, guard_count, normalize_rows, rotation_from_z_to
from .instance import matrix_to_euler_xyz


# -- helpers ----------------------------------------------------------------------------------------------------
def _cloud(geo, keep=None, attrs=None):
    """A point cloud of geo's points (all, or `keep`), carrying their point attributes and new PCG attrs."""
    idx = np.arange(geo.npoints) if keep is None else np.asarray(keep)
    if keep is not None and keep.dtype == bool:
        idx = np.flatnonzero(keep)
    out = Geometry.from_arrays(geo.P[idx])
    for n, a in geo.point.items():
        if n != "P":
            out.point[n] = a[idx]
    out.detail = dict(geo.detail)
    for n, a in (attrs or {}).items():
        a = np.asarray(a)
        out.point[n] = (a[idx] if len(a) == geo.npoints else a).astype(np.int32 if n == "seed" else np.float32)
    return out


def _new_points(P, rot=None, extents=(0.5, 0.5, 0.5), N=None, density=None):
    n = len(P)
    g = Geometry.from_arrays(np.asarray(P, np.float32).reshape(-1, 3))
    e = np.asarray(extents, np.float64)
    g.point["rot"] = np.zeros((n, 3), np.float32) if rot is None else np.asarray(rot, np.float32)
    g.point["scale"] = np.ones((n, 3), np.float32)
    g.point["bounds_min"] = np.tile(-e, (n, 1)).astype(np.float32)
    g.point["bounds_max"] = np.tile(e, (n, 1)).astype(np.float32)
    g.point["density"] = (np.ones(n) if density is None else density).astype(np.float32)
    g.point["seed"] = pcg.position_seeds(g.P)
    if N is not None:
        g.point["N"] = np.asarray(N, np.float32)
    return g


def _is_mesh(geo):
    return geo.nprims > 0 and not geo.topology.has_open


def _mesh_coverage(a, mesh, samples=3):
    """Fraction of each point's box inside a closed mesh, sampled on a samples^3 grid over the box (corners too)."""
    from ..raycast import TriGrid, inside
    if not len(a["P"]):
        return np.zeros(0)
    tris, _ = fan_triangulate(mesh.topology)
    Pm = mesh.P.astype(np.float64)
    grid_ = TriGrid(Pm[mesh.topology.vertex_point[tris]])
    C, A, H = pcg.boxes(a)
    g = np.linspace(-1.0, 1.0, samples) if samples > 1 else np.zeros(1)   # corners, faces and centre
    G = np.stack(np.meshgrid(g, g, g, indexing="ij"), -1).reshape(-1, 3)
    pts = C[:, None, :] + np.einsum("nij,nkj->nki", A, G[None] * H[:, None, :])
    ins = inside(grid_, pts.reshape(-1, 3), float(np.ptp(Pm, 0).max()) if len(Pm) else 1.0)
    return ins.reshape(len(C), -1).mean(1)


def _coverage(a, other):
    """(fraction of each point's box covered by `other`, whether it touches `other` at all): `other`'s points'
    boxes (exact overlap test; the fraction is sampled), or the inside of a closed mesh (sampled, corners
    included)."""
    if _is_mesh(other):
        frac = _mesh_coverage(a, other)
        return frac, frac > 0
    b = pcg.read(other)
    i, j = pcg.overlapping_pairs(a, b)
    touched = np.zeros(len(a["P"]), bool)
    touched[i] = True
    frac = pcg.covered_fraction(a, b, i, j)
    return np.where(touched, np.maximum(frac, 1e-3), 0.0), touched


def _distance_to(points_P, target):
    """Distance from each point to the target: a mesh's surface, a curve's segments, or the nearest point."""
    if _is_mesh(target):
        from ..raycast import TriGrid
        tris, _ = fan_triangulate(target.topology)
        Pm = target.P.astype(np.float64)
        cp, _, _ = TriGrid(Pm[target.topology.vertex_point[tris]]).closest(points_P)
        return np.linalg.norm(cp - points_P, axis=1)
    if target.nprims:
        from .. import topo
        e = topo.edges(target)
        Pt = target.P.astype(np.float64)
        a, b = Pt[e.points[:, 0]], Pt[e.points[:, 1]]
        best = np.full(len(points_P), np.inf)
        ab = b - a
        L2 = np.maximum(np.einsum("ij,ij->i", ab, ab), 1e-300)
        for s in range(0, len(a), 256):                    # chunks of segments
            aa, dd, ll = a[s:s + 256], ab[s:s + 256], L2[s:s + 256]
            t = np.clip(np.einsum("pj,sj->ps", points_P, dd) - np.einsum("sj,sj->s", aa, dd)[None], 0, None) / ll
            t = np.minimum(t, 1.0)
            q = aa[None] + t[..., None] * dd[None]
            best = np.minimum(best, np.linalg.norm(points_P[:, None] - q, axis=2).min(1))
        return best
    dist, _ = spatial.nearest(target.P.astype(np.float64), points_P)
    return dist



# -- samplers ---------------------------------------------------------------------------------------------------
@node("pcg_points", inputs=["geo"], category="pcg", params=[
    Vec3("extents", (0.5, 0.5, 0.5), min=0.0, doc="Half size of each point's box where it has no bounds yet"),
    Float("density", 1.0, min=0.0, max=1.0, doc="Density where the points have none yet"),
])
def pcg_points(geo, extents, density):
    """Turn any points (a mesh's, a scatter's...) into PCG points: fills rot, scale, bounds, density and seed where
    missing and drops the primitives."""
    a = pcg.read(geo)
    e = np.asarray(extents, np.float64)
    attrs = {k: v for k, v in a.items() if k not in geo.point or k == "P"}
    if "bounds_min" not in geo.point:
        attrs["bounds_min"] = np.tile(-e, (geo.npoints, 1))
    if "bounds_max" not in geo.point:
        attrs["bounds_max"] = np.tile(e, (geo.npoints, 1))
    if "density" not in geo.point:
        attrs["density"] = np.full(geo.npoints, float(density))
    attrs.pop("P", None)
    return _cloud(geo, attrs=attrs)


@node("pcg_surface_sampler", inputs=["surface"], category="pcg", params=[
    Float("points_per_m2", 1.0, min=0.0, soft_max=100.0, doc="How many points per square metre of surface", proxy="count"),
    Vec3("extents", (0.5, 0.5, 0.5), min=0.0, doc="Half size of each point's box"),
    Float("looseness", 0.3, min=0.0, max=1.0, doc="0: evenly spaced (blue noise); 1: plain random"),
    Bool("align", False, doc="Rotate points so their +Z follows the surface normal (else upright)"),
    Int("seed", 0),
])
def pcg_surface_sampler(surface, points_per_m2, extents, looseness, align, seed):
    """Points on a surface (PCG's Surface Sampler): random positions thinned to an even spacing, with N, bounds,
    density 1 and a per-point seed."""
    from .scatter import scatter
    tris, _ = fan_triangulate(surface.topology)
    P = surface.P.astype(np.float64)
    T = P[surface.topology.vertex_point[tris]]
    area = float(np.linalg.norm(np.cross(T[:, 1] - T[:, 0], T[:, 2] - T[:, 0]), axis=1).sum() / 2.0) if len(T) else 0
    target = int(round(area * points_per_m2))
    if target <= 0:
        return _new_points(np.zeros((0, 3)), N=np.zeros((0, 3)))
    over = 1 if looseness >= 1.0 else 6
    pts = scatter(surface, target * over, seed)
    X, N = pts.P.astype(np.float64), pts.point["N"].astype(np.float64)
    if over > 1:
        # blue noise: keep a random-priority independent set at a radius that leaves about `target` points
        r = (1.0 - looseness) * 0.75 / np.sqrt(points_per_m2)
        i, j, _ = spatial.pairs_within(X, r) if r > 0 else (np.zeros(0, int),) * 3
        pri = pcg.random01(np.arange(len(X)), seed, 1)
        keep = pcg.greedy_independent(len(X), i, j, pri)
        idx = np.flatnonzero(keep)
        idx = idx[np.argsort(-pri[idx], kind="stable")[:target]]
        X, N = X[idx], N[idx]
    rot = matrix_to_euler_xyz(rotation_from_z_to(normalize_rows(N))) if align else None
    return _new_points(X, rot, extents, N)


@node("pcg_volume_sampler", inputs=["volume"], category="pcg", params=[
    Vec3("voxel_size", (1.0, 1.0, 1.0), min=0.001, doc="Spacing of the grid of points"),
    Float("jitter", 0.0, min=0.0, max=1.0, doc="Random offset, as a fraction of the voxel"),
    Int("seed", 0),
])
def pcg_volume_sampler(volume, voxel_size, jitter, seed):
    """Points on a grid inside a closed mesh (PCG's Volume Sampler); each point's box is its voxel."""
    from ..raycast import TriGrid, inside
    B.require_polygons(volume, "pcg_volume_sampler")
    if volume.nprims == 0:
        return _new_points(np.zeros((0, 3)))
    v = np.asarray(voxel_size, np.float64)
    P = volume.P.astype(np.float64)
    lo, hi = P.min(0), P.max(0)
    dims = np.maximum(np.floor((hi - lo) / v).astype(np.int64), 1)
    if int(np.prod(dims)) > 5_000_000:
        raise ValueError(f"{dims[0]} x {dims[1]} x {dims[2]} voxels is too many; use a larger voxel size")
    start = lo + ((hi - lo) - dims * v) / 2.0 + v / 2.0
    g = np.stack(np.meshgrid(*[np.arange(d) for d in dims], indexing="ij"), -1).reshape(-1, 3)
    X = start + g * v
    tris, _ = fan_triangulate(volume.topology)
    X = X[inside(TriGrid(P[volume.topology.vertex_point[tris]]), X, float(np.ptp(P, 0).max()))]
    if jitter > 0 and len(X):
        s = pcg.position_seeds(X)
        X = X + (np.stack([pcg.random01(s, seed, k) for k in range(3)], 1) - 0.5) * v * jitter
    return _new_points(X, extents=v / 2.0)


@node("pcg_spline_sampler", inputs=["curve"], category="pcg", params=[
    Enum("mode", ["on_curve", "interior"], doc="on_curve: along the curves; interior: filling closed curves (in "
         "their plane, e.g. a lot outline drawn in XY)"),
    Float("spacing", 1.0, min=0.001, soft_max=20.0, doc="Distance between points", proxy="step1"),
    Vec3("extents", (0.5, 0.5, 0.5), min=0.0),
    Float("jitter", 0.0, min=0.0, max=1.0, doc="interior: random offset, as a fraction of the spacing"),
    Int("seed", 0),
])
def pcg_spline_sampler(curve, mode, spacing, extents, jitter, seed):
    """Points along curves, their X axis following the curve (PCG's Spline Sampler), or on a grid inside closed
    curves."""
    from .curves import _sample, curve_arrays
    if curve.nprims == 0:
        return _new_points(np.zeros((0, 3)))
    if mode == "on_curve":
        prims, starts, sizes, closed, vert, seg = curve_arrays(curve)
        lengths = np.bincount(np.repeat(np.arange(len(prims)), sizes), weights=seg, minlength=len(prims))
        guard_count(float(lengths.sum() / spacing), "pcg_spline_sampler")
        s_lists = [np.arange(0.0, L + 1e-9, spacing) if L > 0 else np.zeros(1) for L in lengths]
        s_lists = [s[:-1] if c and len(s) > 1 and abs(s[-1] - L) < 1e-9 else s
                   for s, c, L in zip(s_lists, closed, lengths)]
        res, _ = _sample(curve, prims, s_lists)
        X = res.P.astype(np.float64)
        # tangents from neighbouring samples on the same curve
        offs = np.concatenate([[0], np.cumsum([len(s) for s in s_lists])])
        Tg = np.zeros_like(X)
        for k in range(len(s_lists)):
            a, b = offs[k], offs[k + 1]
            if b - a >= 2:
                Tg[a:b] = np.gradient(X[a:b], axis=0)
        Tg = normalize_rows(np.where(np.linalg.norm(Tg, axis=1, keepdims=True) > 0, Tg, [[1.0, 0, 0]]))
        yaw = np.arctan2(Tg[:, 1], Tg[:, 0])
        pitch = -np.arcsin(np.clip(Tg[:, 2], -1, 1))
        rot = np.stack([np.zeros(len(X)), pitch, yaw], 1)
        return _new_points(X, rot, extents)
    # interior: a grid over each closed curve, kept where inside it (in the curve's best-fit plane)
    out = []
    prims, starts, sizes, closed, vert, seg = curve_arrays(curve)
    Pc = curve.P.astype(np.float64)[curve.topology.vertex_point[vert]]
    first = np.concatenate([[0], np.cumsum(sizes)])
    produced = 0.0
    for k in range(len(prims)):
        Q = Pc[first[k]:first[k + 1]]
        if len(Q) < 3:
            continue
        c = Q.mean(0)
        n = np.cross(Q - c, np.roll(Q, -1, 0) - c).sum(0)
        if np.linalg.norm(n) < 1e-12:
            continue
        n /= np.linalg.norm(n)
        u = np.cross(n, [1.0, 0, 0] if abs(n[0]) < 0.9 else [0, 1.0, 0])
        u /= np.linalg.norm(u)
        w = np.cross(n, u)
        if abs(n[2]) > 0.9:                                  # a curve drawn on the ground: keep the world axes
            u, w = np.array([1.0, 0, 0]), np.array([0, 1.0, 0]) * np.sign(n[2])
        xy = np.stack([(Q - c) @ u, (Q - c) @ w], 1)
        lo, hi = xy.min(0), xy.max(0)
        produced += float(np.prod(np.maximum((hi - lo) / spacing, 1.0)))
        guard_count(produced, "pcg_spline_sampler")                 # all the curves together, not each one
        gx = np.arange(lo[0] + spacing / 2, hi[0], spacing)
        gy = np.arange(lo[1] + spacing / 2, hi[1], spacing)
        G = np.stack(np.meshgrid(gx, gy, indexing="ij"), -1).reshape(-1, 2)
        if jitter > 0 and len(G):
            s = pcg.position_seeds(np.c_[G, np.full(len(G), k)])
            G = G + (np.stack([pcg.random01(s, seed, 0), pcg.random01(s, seed, 1)], 1) - 0.5) * spacing * jitter
        inside_ = np.zeros(len(G), bool)
        for e in range(len(xy)):                             # crossing test, vectorised over the grid
            a, b = xy[e - 1], xy[e]
            cross = (a[1] > G[:, 1]) != (b[1] > G[:, 1])
            x = a[0] + (G[:, 1] - a[1]) * (b[0] - a[0]) / np.where(b[1] != a[1], b[1] - a[1], 1.0)
            inside_ ^= cross & (G[:, 0] < x)
        G = G[inside_]
        out.append(c + G[:, :1] * u + G[:, 1:] * w)
    X = np.concatenate(out) if out else np.zeros((0, 3))
    return _new_points(X, extents=extents)


# -- modifiers --------------------------------------------------------------------------------------------------
@node("pcg_transform", inputs=["points"], category="pcg", params=[
    Vec3("offset_min", (0.0, 0.0, 0.0)), Vec3("offset_max", (0.0, 0.0, 0.0)),
    Vec3("rotation_min", (0.0, 0.0, 0.0), doc="Degrees, Euler XYZ"), Vec3("rotation_max", (0.0, 0.0, 360.0)),
    Vec3("scale_min", (1.0, 1.0, 1.0)), Vec3("scale_max", (1.0, 1.0, 1.0)),
    Bool("uniform_scale", True, doc="One random factor for all axes (from the X range)"),
    Bool("absolute_rotation", False, doc="Replace the rotation instead of adding to it"),
    Int("seed", 0),
])
def pcg_transform(points, offset_min, offset_max, rotation_min, rotation_max, scale_min, scale_max, uniform_scale,
                  absolute_rotation, seed):
    """Random offsets, rotations and scales per point (PCG's Transform Points), driven by each point's seed."""
    a = pcg.read(points)
    s = a["seed"]
    r = [pcg.random01(s, seed, k) for k in range(9)]

    def lerp(lo, hi, k0):
        lo, hi = np.asarray(lo, np.float64), np.asarray(hi, np.float64)
        return np.stack([lo[c] + (hi[c] - lo[c]) * r[k0 + c] for c in range(3)], 1)

    off = lerp(offset_min, offset_max, 0)
    rot = np.radians(lerp(rotation_min, rotation_max, 3))
    sc = lerp(scale_min, scale_max, 6)
    if uniform_scale:
        sc = np.repeat(sc[:, :1], 3, axis=1)
    P = a["P"] + off
    if absolute_rotation:
        new_rot = rot
    else:
        R = pcg.rotations(a["rot"]) @ pcg.rotations(rot)       # random turn in the point's own frame
        new_rot = matrix_to_euler_xyz(R)
    return pcg.write(points, {"P": P, "rot": new_rot, "scale": a["scale"] * sc})


@node("pcg_bounds", inputs=["points"], category="pcg", params=[
    Enum("mode", ["set", "scale"]),
    Vec3("bounds_min", (-0.5, -0.5, -0.5)), Vec3("bounds_max", (0.5, 0.5, 0.5)),
    Float("factor", 1.0, min=0.0, soft_max=4.0, doc="scale: multiply the bounds"),
])
def pcg_bounds(points, mode, bounds_min, bounds_max, factor):
    """Set or scale the points' boxes (PCG's Bounds Modifier): what the booleans and self-pruning test."""
    a = pcg.read(points)
    n = points.npoints
    if mode == "set":
        lo, hi = np.tile(np.asarray(bounds_min, np.float64), (n, 1)), np.tile(np.asarray(bounds_max, np.float64), (n, 1))
    else:
        c = (a["bounds_min"] + a["bounds_max"]) / 2.0
        lo, hi = c + (a["bounds_min"] - c) * factor, c + (a["bounds_max"] - c) * factor
    return pcg.write(points, {"bounds_min": lo, "bounds_max": hi})


@node("pcg_density_noise", inputs=["points"], category="pcg", params=[
    Enum("noise", ["random", "perlin"], doc="random: per point (from its seed); perlin: smooth patches in space"),
    Enum("mode", ["set", "multiply", "add", "minimum", "maximum"], doc="How the noise combines with the density"),
    Float("frequency", 0.2, min=0.0, soft_max=5.0, doc="perlin: patches per metre"),
    Float("low", 0.0, min=0.0, max=1.0), Float("high", 1.0, min=0.0, max=1.0, doc="Range of the noise values"),
    Bool("invert", False),
    Int("seed", 0),
])
def pcg_density_noise(points, noise, mode, frequency, low, high, invert, seed):
    """Vary density with noise (PCG's Density Noise / Spatial Noise): feed a density filter to thin points in
    patches."""
    a = pcg.read(points)
    if noise == "random":
        v = pcg.random01(a["seed"], seed, 0)
    else:
        v = np.clip(fbm3(a["P"].astype(np.float32) * float(frequency), int(seed) & 0xFFFFFFFF, 3, 2.0, 0.5) * 0.5 + 0.5,
                    0.0, 1.0)
    if invert:
        v = 1.0 - v
    v = low + (high - low) * v
    d = a["density"]
    d = {"set": v, "multiply": d * v, "add": d + v, "minimum": np.minimum(d, v), "maximum": np.maximum(d, v)}[mode]
    return pcg.write(points, {"density": np.clip(d, 0.0, 1.0)})


@node("pcg_distance", inputs=["points", "target"], category="pcg", params=[
    Float("max_distance", 5.0, min=1e-6, soft_max=100.0, doc="Density goes from 0 at the target to 1 at this distance"),
    Enum("mode", ["set", "multiply"]),
    Bool("invert", False, doc="1 at the target, 0 far away"),
])
def pcg_distance(points, target, max_distance, mode, invert):
    """Density from the distance to another input: a mesh's surface, a curve, or its points (PCG's Distance)."""
    a = pcg.read(points)
    dist = np.full(points.npoints, np.inf) if target.npoints == 0 else _distance_to(a["P"], target)
    v = np.clip(dist / max_distance, 0.0, 1.0)
    if invert:
        v = 1.0 - v
    d = v if mode == "set" else a["density"] * v
    return pcg.write(points, {"density": d})


@node("pcg_density_filter", inputs=["points"], category="pcg", params=[
    Float("lower", 0.5, min=0.0, max=1.0), Float("upper", 1.0, min=0.0, max=1.0),
    Bool("invert", False, doc="Keep the points outside the range instead"),
    Bool("probabilistic", False, doc="Keep each point with probability = its density (ignores the range)"),
    Int("seed", 0),
])
def pcg_density_filter(points, lower, upper, invert, probabilistic, seed):
    """Keep points by density (PCG's Density Filter)."""
    a = pcg.read(points)
    d = a["density"]
    keep = pcg.random01(a["seed"], seed, 0) < d if probabilistic else (d >= lower) & (d <= upper)
    if invert:
        keep = ~keep
    return _cloud(points, keep)


# -- point booleans ---------------------------------------------------------------------------------------------
@node("pcg_difference", inputs=["points", "subtract"], category="pcg", params=[
    Enum("density_function", ["binary", "subtract"], doc="binary: drop points that touch; subtract: lower their "
         "density by how much of their box is covered (dropping those left at 0)"),
])
def pcg_difference(points, subtract, density_function):
    """Remove points whose boxes overlap the other input (PCG's Difference): its points' boxes, or the inside of a
    closed mesh (a house footprint, a road made with sweep...)."""
    a = pcg.read(points)
    cov, touched = _coverage(a, subtract)
    if density_function == "binary":
        return _cloud(points, ~touched)
    d = a["density"] * (1.0 - cov)
    out = _cloud(points, d > 1e-6, {"density": d})
    return out


@node("pcg_intersection", inputs=["points", "other"], category="pcg", params=[
    Enum("density_function", ["binary", "multiply"], doc="binary: keep points that touch; multiply: scale their "
         "density by how much of their box is covered"),
])
def pcg_intersection(points, other, density_function):
    """Keep points whose boxes overlap the other input (PCG's Intersection): its points' boxes, or the inside of
    a closed mesh."""
    a = pcg.read(points)
    cov, touched = _coverage(a, other)
    if density_function == "binary":
        return _cloud(points, touched)
    d = a["density"] * cov
    return _cloud(points, d > 1e-6, {"density": d})


@node("pcg_union", inputs=[Input("inputs", multi=True)], category="pcg", params=[
    Enum("density_function", ["keep", "maximum", "add"], doc="maximum / add: where boxes from different inputs "
         "overlap, raise each point's density to the highest one, or add them (clamped to 1)"),
])
def pcg_union(inputs, density_function):
    """Merge point sets (PCG's Union), optionally combining the density where they overlap."""
    from .merge import merge
    if not inputs:
        return _new_points(np.zeros((0, 3)))
    parts = [_cloud(g, attrs={k: v for k, v in pcg.read(g).items() if k != "P"}) for g in inputs]
    which = np.concatenate([np.full(p.npoints, k) for k, p in enumerate(parts)])
    out = merge(parts) if len(parts) > 1 else parts[0]
    if density_function == "keep" or len(parts) < 2:
        return out
    a = pcg.read(out)
    i, j = pcg.overlapping_pairs(a)
    cross = which[i] != which[j]
    i, j = i[cross], j[cross]
    d = a["density"].copy()
    if density_function == "maximum":
        np.maximum.at(d, i, a["density"][j])
        np.maximum.at(d, j, a["density"][i])
    else:
        np.add.at(d, i, a["density"][j])
        np.add.at(d, j, a["density"][i])
    return pcg.write(out, {"density": np.clip(d, 0.0, 1.0)})


@node("pcg_self_prune", inputs=["points"], category="pcg", params=[
    Enum("priority", ["large_first", "small_first", "density", "random"], doc="Which of two overlapping points "
         "survives: the larger box, the smaller, the denser, or by chance"),
    Int("seed", 0),
])
def pcg_self_prune(points, priority, seed):
    """Remove points whose boxes overlap others (PCG's Self Pruning): no two remaining boxes overlap."""
    a = pcg.read(points)
    i, j = pcg.overlapping_pairs(a)
    vol = np.prod(np.abs(a["bounds_max"] - a["bounds_min"]) * np.abs(a["scale"]), axis=1)
    tie = pcg.random01(a["seed"], seed, 0) * 1e-6
    pri = {"large_first": vol + tie * vol.max(initial=1.0), "small_first": -vol + tie * vol.max(initial=1.0),
           "density": a["density"] + tie, "random": tie}[priority]
    return _cloud(points, pcg.greedy_independent(points.npoints, i, j, pri))


# -- projection, copying, spawning ------------------------------------------------------------------------------
@node("pcg_projection", inputs=["points", "surface"], category="pcg", params=[
    Vec3("direction", (0.0, 0.0, -1.0), doc="Cast along this direction (and back) to find the surface"),
    Bool("align", True, doc="Turn each point's +Z to the surface normal"),
    Bool("keep_misses", False, doc="Keep points that miss the surface (unchanged)"),
])
def pcg_projection(points, surface, direction, align, keep_misses):
    """Drop points onto a surface (PCG's Projection): terrain, a roof, a wall..."""
    from ..raycast import TriGrid
    a = pcg.read(points)
    tris, _ = fan_triangulate(surface.topology)
    Pm = surface.P.astype(np.float64)
    tri_pts = Pm[surface.topology.vertex_point[tris]]
    grid_ = TriGrid(tri_pts)
    d = np.asarray(direction, np.float64)
    d = d / max(np.linalg.norm(d), 1e-12)
    far = float(np.ptp(np.concatenate([Pm, a["P"]]), 0).max()) * 2 + 1.0 if len(Pm) else 1.0
    t1, f1, _ = grid_.intersect(a["P"], np.tile(d, (len(a["P"]), 1)), far)
    t2, f2, _ = grid_.intersect(a["P"], np.tile(-d, (len(a["P"]), 1)), far)
    use_back = (~np.isfinite(t1)) | (np.isfinite(t2) & (t2 < t1))
    t = np.where(use_back, -t2, t1)
    f = np.where(use_back, f2, f1)
    hit = np.isfinite(t)
    P = np.where(hit[:, None], a["P"] + d * np.where(hit, t, 0.0)[:, None], a["P"])
    attrs = {"P": P}
    if align and hit.any():
        T = tri_pts[np.maximum(f, 0)]
        n = normalize_rows(np.cross(T[:, 1] - T[:, 0], T[:, 2] - T[:, 0])).astype(np.float64)
        n = np.where((n @ -d < 0)[:, None], -n, n)          # face the side the points came from
        R = rotation_from_z_to(n) @ pcg.rotations(np.c_[np.zeros(len(n)), np.zeros(len(n)), a["rot"][:, 2]])
        attrs["rot"] = np.where(hit[:, None], matrix_to_euler_xyz(R), a["rot"])
        attrs["N"] = np.where(hit[:, None], n, points.point["N"] if "N" in points.point else n)
    out = pcg.write(points, attrs)
    return out if keep_misses else _cloud(out, hit)


@node("pcg_copy_points", inputs=["source", "targets"], category="pcg", params=[
    Bool("inherit_rotation", True), Bool("inherit_scale", True),
])
def pcg_copy_points(source, targets, inherit_rotation, inherit_scale):
    """Copy a set of points onto every target point, composing their transforms (PCG's Copy Points): clusters,
    rings of rocks around each tree... Seeds mix both, densities multiply."""
    s, t = pcg.read(source), pcg.read(targets)
    ns, nt = source.npoints, targets.npoints
    Rt = pcg.rotations(t["rot"]) if inherit_rotation else np.tile(np.eye(3), (nt, 1, 1))
    St = t["scale"] if inherit_scale else np.ones((nt, 3))
    ti, si = np.repeat(np.arange(nt), ns), np.tile(np.arange(ns), nt)
    P = t["P"][ti] + np.einsum("nij,nj->ni", Rt[ti], s["P"][si] * St[ti])
    R = Rt[ti] @ pcg.rotations(s["rot"][si])
    out = Geometry.from_arrays(P.astype(np.float32))
    for n, arr in source.point.items():
        if n != "P":
            out.point[n] = arr[si]
    out.point["rot"] = matrix_to_euler_xyz(R).astype(np.float32)
    out.point["scale"] = (s["scale"][si] * St[ti]).astype(np.float32)
    out.point["bounds_min"] = s["bounds_min"][si].astype(np.float32)
    out.point["bounds_max"] = s["bounds_max"][si].astype(np.float32)
    out.point["density"] = (s["density"][si] * t["density"][ti]).astype(np.float32)
    out.point["seed"] = (pcg.hash32(s["seed"][si].astype(np.int64) * 0x9E3779B1 ^ t["seed"][ti].astype(np.int64))
                         & np.uint64(0x7FFFFFFF)).astype(np.int32)
    return out


@node("pcg_spawner", inputs=["points"], category="pcg", params=[
    Str("objects", "", doc="Weighted Blender objects, e.g. 'Tree:3 Bush:2 Rock:1' (weight 1 if omitted)"),
    Int("seed", 0),
])
def pcg_spawner(points, objects, seed):
    """Choose a Blender object for every point by weight (PCG's Static Mesh Spawner); Blender instances them with
    each point's rot and scale. Also writes ``variant`` (0-based choice) for copy_to_points or wrangles."""
    entries = []
    for tok in objects.replace(",", " ").split():
        name, _, w = tok.partition(":")
        try:
            weight = float(w) if w else 1.0
        except ValueError:
            raise ValueError(f"weight of {name!r} must be a number, e.g. {name}:2") from None
        if name and weight > 0:
            entries.append((name, weight))
    if not entries:
        raise ValueError("list the Blender objects to spawn, e.g. 'Tree:3 Rock:1'")
    a = pcg.read(points)
    w = np.array([e[1] for e in entries])
    cdf = np.cumsum(w) / w.sum()
    pick = np.minimum(np.searchsorted(cdf, pcg.random01(a["seed"], seed, 0), side="right"), len(entries) - 1)
    out = pcg.write(points, {"rot": a["rot"], "scale": a["scale"]})
    out.point["instance"] = (pick + 1).astype(np.int32)
    out.point["variant"] = pick.astype(np.int32)
    out.detail["instance_objects"] = [e[0] for e in entries]
    out.detail.pop("instance_bounds", None)
    return out


@node("pcg_debug", inputs=["points"], category="pcg", params=[
    Bool("color_by_density", True, doc="Cd from density: dark red (0) to white (1)"),
])
def pcg_debug(points, color_by_density):
    """Show each point as its box (PCG's Debug view), to see what the booleans and pruning see."""
    from .primitives import box
    a = pcg.read(points)
    n = points.npoints
    unit = box((2.0, 2.0, 2.0), (0.0, 0.0, 0.0), 1)          # -1..1 cube: 8 points, 6 quads
    C, A, H = pcg.boxes(a)
    corners = unit.P.astype(np.float64)
    P = C[:, None, :] + np.einsum("nij,nkj->nki", A, corners[None] * H[:, None, :])
    vp = (unit.topology.vertex_point[None, :] + 8 * np.arange(n)[:, None]).ravel()
    out = B.make(P.reshape(-1, 3), vp, np.arange(6 * n + 1) * 4)
    if color_by_density:
        d = np.repeat(a["density"], 8)[:, None]
        out.point["Cd"] = (np.array([0.4, 0.02, 0.02]) * (1 - d) + d).astype(np.float32)
    out.point["density"] = np.repeat(a["density"], 8).astype(np.float32)
    return out
