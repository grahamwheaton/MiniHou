"""Point nodes: the rest of Houdini's Points family (Point Generate, Replicate, Velocity, Cluster Points, Point Cloud Measure / Normal /
Reduce / Surface, Points from Volume, Scatter in Texture Mask, Particle Trail).

All of them take and give plain point clouds (a polygon input is read for its points), so they chain with ``scatter``, ``point_relax``,
``point_jitter`` and the ``pcg_*`` nodes. Neighbour searches use SciPy's KD-tree when it is installed and a chunked brute force
(capped) when it is not.
"""
import numpy as np

from .. import groups, images, spatial, timectx
from ..geometry import Geometry
from ..sdk import Bool, Enum, Float, Input, Int, Str, Vec3, node
from ._util import fan_triangulate, guard_count, normalize_rows

MAX_POINTS = 20_000_000


def _cloud(P, **attrs) -> Geometry:
    g = Geometry.from_arrays(np.ascontiguousarray(P, np.float32).reshape(-1, 3))
    for k, v in attrs.items():
        g.point[k] = v
    return g


def take_points(geo: Geometry, idx) -> Geometry:
    """A point cloud of the points ``idx`` (repeats allowed): every point attribute follows, the detail attributes are kept;
    primitives are dropped."""
    idx = np.asarray(idx, np.int64)
    out = Geometry.from_arrays(np.asarray(geo.P)[idx])
    for name in geo.point.names():
        if name != "P":
            out.point[name] = np.asarray(geo.point[name])[idx]
    for k, v in geo.detail.items():
        out.detail[k] = v
    return out


def _spacing(P) -> float:
    """A typical distance between neighbouring points: the median nearest-neighbour distance."""
    if len(P) < 2:
        return 1.0
    sample = P if len(P) <= 5000 else P[np.random.default_rng(1).choice(len(P), 5000, replace=False)]
    _, d = spatial.knn(P, sample, 2)
    d = d[:, 1]
    d = d[np.isfinite(d) & (d > 0)]
    return float(np.median(d)) if len(d) else 1.0


# -- Point Generate ------------------------------------------------------------------------------------------------------
@node("point_generate", category="generate", params=[
    Int("count", 100, min=0, soft_max=100000, doc="How many points"),
    Enum("shape", ["box", "sphere", "ball", "line", "circle", "origin"],
         doc="box: random inside a box; sphere: random on a sphere; ball: random inside a sphere; line: evenly along X; "
             "circle: evenly on a circle in XY; origin: all at the centre"),
    Vec3("center", (0.0, 0.0, 0.0)),
    Vec3("size", (1.0, 1.0, 1.0), doc="Full size along each axis (sphere / ball / circle use the radius = half of it)"),
    Int("seed", 0),
    Float("pscale", 0.0, min=0.0, soft_max=1.0, doc="Write this as the point attribute pscale (0: do not write it)"),
])
def point_generate(count, shape, center, size, seed, pscale):
    """Points from nothing (Houdini's Point Generate): random or regular points in a simple shape, each with an ``id``."""
    guard_count(count, "points", "use a smaller count")
    n = int(count)
    rng = np.random.default_rng([int(seed) & 0xFFFFFFFF, 3])
    half = np.asarray(size, np.float64) / 2.0
    c = np.asarray(center, np.float64)
    if shape == "box":
        P = c + (rng.random((n, 3)) * 2 - 1) * half
    elif shape in ("sphere", "ball"):
        v = rng.normal(size=(n, 3))
        v /= np.maximum(np.linalg.norm(v, axis=1, keepdims=True), 1e-12)
        if shape == "ball":
            v *= rng.random((n, 1)) ** (1.0 / 3.0)
        P = c + v * half
    elif shape == "line":
        t = (np.arange(n) + 0.5) / max(n, 1) * 2 - 1
        P = c + np.stack([t * half[0], np.zeros(n), np.zeros(n)], 1)
    elif shape == "circle":
        a = np.arange(n) / max(n, 1) * 2 * np.pi
        P = c + np.stack([np.cos(a) * half[0], np.sin(a) * half[1], np.zeros(n)], 1)
    else:
        P = np.tile(c, (n, 1))
    g = _cloud(P, id=np.arange(n, dtype=np.int32))
    if pscale > 0:
        g.point["pscale"] = np.full(n, pscale, np.float32)
    return g


# -- Point Replicate -----------------------------------------------------------------------------------------------------
@node("point_replicate", inputs=["geo"], category="generate", params=[
    Int("copies", 2, min=1, soft_max=100, doc="How many points each point becomes (with the original)"),
    Str("copies_attrib", "", doc="Integer point attribute that sets the number of copies per point instead (0 removes the point)"),
    Vec3("offset", (0.0, 0.0, 0.0), doc="Random offset of every copy (not of the original), up to this much along each axis"),
    Bool("keep_original", True, doc="The first copy sits exactly on the original point"),
    Int("seed", 0),
    Str("group", "", doc="Only these points are replicated (others stay as they are)"),
])
def point_replicate(geo, copies, copies_attrib, offset, keep_original, seed, group):
    """Make several points out of each point (Houdini's Point Replicate). Attributes are copied; ``copy`` (0 for the original) and
    ``source`` (the point it came from) say which is which. Primitives are dropped."""
    n = geo.npoints
    if copies_attrib:
        a = geo.point.get(copies_attrib)
        if a is None or a.ndim != 1:
            raise ValueError(f"no scalar point attribute {copies_attrib!r} to count copies by")
        per = np.maximum(a.astype(np.int64), 0)
    else:
        per = np.full(n, int(copies), np.int64)
    sel = groups.mask(geo, "point", group)
    if sel is not None:
        per = np.where(sel, per, 1)
    total = int(per.sum())
    guard_count(total, "points", "use fewer copies")
    src = np.repeat(np.arange(n), per)
    copy = np.arange(total) - np.repeat(np.cumsum(per) - per, per)
    out = take_points(geo, src)
    rng = np.random.default_rng([int(seed) & 0xFFFFFFFF, 5])
    jitter = (rng.random((total, 3)) * 2 - 1) * np.asarray(offset, np.float64)
    if keep_original:
        jitter[copy == 0] = 0
    if sel is not None:
        jitter[~sel[src]] = 0
    out.point["P"] = (out.P.astype(np.float64) + jitter).astype(np.float32)
    out.point["copy"] = copy.astype(np.int32)
    out.point["source"] = src.astype(np.int32)
    return out


# -- Point Velocity ------------------------------------------------------------------------------------------------------
def _curl(p, freq, seed, eps=1e-2):
    from ._util import perlin3
    q = p * freq
    pot = [lambda x, s=s: perlin3(x, seed + s).astype(np.float64) for s in (0, 101, 202)]
    e = np.eye(3) * eps
    d = lambda f, k: (f(q + e[k]) - f(q - e[k])) / (2 * eps)
    return np.stack([d(pot[2], 1) - d(pot[1], 2), d(pot[0], 2) - d(pot[2], 0), d(pot[1], 0) - d(pot[0], 1)], 1) * freq


@node("point_velocity", inputs=["geo", Input("previous", optional=True, doc="The same points one step earlier (for method 'difference')")],
      category="modify", params=[
    Enum("method", ["difference", "noise", "direction"], doc="difference: (P - previous P) / time step, points matched by 'id' when both have "
                                                           "it, else by number; noise: a divergence-free (curl) noise field; direction: the "
                                                           "same velocity for every point (plus the random spread)"),
    Str("attrib", "v", doc="Point attribute to write"),
    Float("scale", 1.0, soft_max=10.0, doc="difference: multiply the velocity by this"),
    Bool("add", False, doc="Add to the attribute when it exists instead of replacing it"),
    Float("time_step", 0.0, min=0.0, soft_max=1.0, doc="Seconds between 'previous' and the points (0: one frame at the current frame rate)"),
    Vec3("direction", (0.0, 0.0, 1.0)),
    Float("speed", 1.0, soft_max=10.0, doc="noise / direction: the speed (the length of the velocity)"),
    Float("spread", 0.0, min=0.0, soft_max=1.0, doc="direction: random variation of the speed (fraction)"),
    Float("frequency", 1.0, min=0.0, soft_max=10.0, doc="noise: how fast the field changes in space"),
    Int("seed", 0),
    Str("group", ""),
])
def point_velocity(geo, previous=None, *, method, attrib, scale, add, time_step, direction, speed, spread, frequency, seed, group):
    """Give points a velocity (Houdini's Point Velocity): from where they were one step earlier, or from a noise field."""
    n = geo.npoints
    P = geo.P.astype(np.float64)
    if method == "difference":
        if previous is None:
            raise ValueError("method 'difference' needs the earlier points on the 'previous' input")
        dt = float(time_step) or 1.0 / max(timectx.fps(), 1e-6)
        pid, cid = previous.point.get("id"), geo.point.get("id")
        if pid is not None and cid is not None and pid.ndim == 1 and cid.ndim == 1:
            order = {int(v): i for i, v in enumerate(pid)}
            idx = np.array([order.get(int(v), -1) for v in cid], np.int64)
        else:
            if previous.npoints != n:
                raise ValueError(f"previous has {previous.npoints} points but the input has {n}: give both an integer 'id' attribute to match them")
            idx = np.arange(n)
        ok = idx >= 0
        v = np.zeros((n, 3))
        v[ok] = (P[ok] - previous.P.astype(np.float64)[idx[ok]]) / dt * float(scale)
    elif method == "noise":
        v = _curl(P, float(frequency), int(seed))
        norm = np.linalg.norm(v, axis=1, keepdims=True)
        v = v / np.maximum(np.percentile(norm, 95) if n else 1.0, 1e-9) * speed      # roughly unit speed, then scaled
    else:
        d = np.asarray(direction, np.float64)
        d = d / max(np.linalg.norm(d), 1e-12)
        rng = np.random.default_rng([int(seed) & 0xFFFFFFFF, 9])
        v = d * (speed * (1.0 + (rng.random((n, 1)) * 2 - 1) * spread))
    old = geo.point.get(attrib)
    if add and old is not None and old.shape == (n, 3):
        v = old.astype(np.float64) + v
    out = geo.copy()
    out.point[attrib] = v.astype(np.float32)
    return groups.restrict(geo, out, "point", group)


# -- Cluster Points ------------------------------------------------------------------------------------------------------
def _kmeans(P, k, iterations, seed):
    n = len(P)
    rng = np.random.default_rng([int(seed) & 0xFFFFFFFF, 13])
    centres = np.empty((k, 3))
    centres[0] = P[rng.integers(n)]
    d2 = ((P - centres[0]) ** 2).sum(1)
    for i in range(1, k):                                               # k-means++: far points are likelier centres
        tot = d2.sum()
        pick = rng.integers(n) if tot <= 0 else np.searchsorted(np.cumsum(d2), rng.random() * tot)
        centres[i] = P[min(pick, n - 1)]
        d2 = np.minimum(d2, ((P - centres[i]) ** 2).sum(1))
    label = np.zeros(n, np.int64)
    for _ in range(max(int(iterations), 1)):
        new = np.empty(n, np.int64)
        step = max(1, 8_000_000 // k)
        for a in range(0, n, step):
            new[a:a + step] = ((P[a:a + step, None, :] - centres[None]) ** 2).sum(-1).argmin(1)
        if np.array_equal(new, label) and _ > 0:
            break
        label = new
        cnt = np.bincount(label, minlength=k)
        for c in range(3):
            s = np.bincount(label, weights=P[:, c], minlength=k)
            centres[:, c] = np.where(cnt > 0, s / np.maximum(cnt, 1), centres[:, c])
    return label


@node("cluster_points", inputs=["geo", Input("rest", optional=True, doc="Rest positions: cluster by where the points were here (same point count) "
                                                                    "instead of where they are")], category="modify", params=[
    Enum("method", ["kmeans", "grid", "distance"], doc="kmeans: this many groups of nearby points; grid: one cluster per cell of the "
                                                      "given size; distance: points closer than the size are chained into one cluster"),
    Int("count", 8, min=1, soft_max=100, doc="kmeans: number of clusters"),
    Float("size", 1.0, min=1e-6, soft_max=10.0, doc="grid: cell size; distance: link distance"),
    Int("iterations", 20, min=1, soft_max=100),
    Int("seed", 0),
    Str("attrib", "cluster", doc="Integer point attribute that gets the cluster number"),
    Enum("output", ["attribute", "centroids"], doc="attribute: the input points with the cluster number; centroids: one point per cluster "
                                                  "(its centre) with the number of points in it as 'count'"),
    Str("group", ""),
])
def cluster_points(geo, rest=None, *, method, count, size, iterations, seed, attrib, output, group):
    """Sort points into clusters (Houdini's Cluster Points)."""
    n = geo.npoints
    P = geo.P.astype(np.float64)
    if rest is not None:
        if rest.npoints != n:
            raise ValueError(f"the rest input has {rest.npoints} points but the points to cluster have {n}")
        P = rest.P.astype(np.float64)
    sel = groups.mask(geo, "point", group)
    use = np.flatnonzero(sel) if sel is not None else np.arange(n)
    label = np.full(n, -1, np.int64)
    if len(use):
        Q = P[use]
        if method == "kmeans":
            lab = _kmeans(Q, min(int(count), len(Q)), iterations, seed)
        elif method == "grid":
            cell = np.floor((Q - Q.min(0)) / float(size)).astype(np.int64)
            _, lab = np.unique(cell, axis=0, return_inverse=True)
        else:
            i, j, _d = spatial.pairs_within(Q, float(size))
            lab = spatial.components(len(Q), i, j)
        _, lab = np.unique(np.asarray(lab).reshape(-1), return_inverse=True)           # consecutive numbers from 0
        label[use] = lab.reshape(-1)
    if output == "centroids":
        k = int(label.max()) + 1 if len(use) else 0
        cnt = np.bincount(label[use], minlength=k)
        cen = np.stack([np.bincount(label[use], weights=P[use, c], minlength=k) for c in range(3)], 1) / np.maximum(cnt, 1)[:, None]
        return _cloud(cen, **{attrib: np.arange(k, dtype=np.int32), "count": cnt.astype(np.int32)})
    out = geo.copy()
    out.point[attrib] = label.astype(np.int32)
    return out


# -- Point Cloud: local planes -------------------------------------------------------------------------------------------
def _local_planes(cloud, query, k, weighted=True):
    """For each query point the weighted centre and plane normal of its ``k`` nearest cloud points: (centre, normal, spacing)."""
    idx, dist = spatial.knn(cloud, query, k)
    ok = idx >= 0
    nb = cloud[np.where(ok, idx, 0)]                                    # (m, k, 3)
    h = np.maximum(np.where(ok, dist, 0).max(1, keepdims=True), 1e-12)
    w = np.where(ok, np.exp(-(np.where(ok, dist, 0) / h) ** 2 * 2.0) if weighted else 1.0, 0.0)
    ws = np.maximum(w.sum(1, keepdims=True), 1e-12)
    centre = (nb * w[..., None]).sum(1) / ws
    d = nb - centre[:, None, :]
    cov = np.einsum("mk,mki,mkj->mij", w, d, d)
    cov += np.eye(3) * 1e-18
    vals, vecs = np.linalg.eigh(cov)
    return centre, vecs[:, :, 0], h[:, 0], vals / np.maximum(w.sum(1, keepdims=True), 1e-12)


def _planes_chunked(cloud, query, k, weighted=True):
    m = len(query)
    step = max(1, 2_000_000 // max(k, 1))
    parts = [_local_planes(cloud, query[a:a + step], k, weighted) for a in range(0, m, step)]
    if not parts:
        return np.zeros((0, 3)), np.zeros((0, 3)), np.zeros(0), np.zeros((0, 3))
    return tuple(np.concatenate([p[i] for p in parts]) for i in range(4))


@node("point_cloud_normal", inputs=["geo"], category="modify", params=[
    Int("neighbors", 12, min=3, soft_max=64, doc="Points used for each plane fit"),
    Enum("orient", ["keep", "outward", "inward", "viewpoint", "up"],
         doc="Which way the normal faces: keep: toward the existing N when there is one (else unchanged); outward / inward: away from / "
             "toward the middle of the cloud; viewpoint: toward the point below; up: +Z side"),
    Vec3("viewpoint", (0.0, 0.0, 10.0)),
    Float("radius", 0.0, min=0.0, soft_max=5.0, doc="Only neighbours within this distance count (0: just the nearest 'neighbors'). Points with fewer than "
                                                  "3 neighbours keep their old N"),
    Bool("reverse", False, doc="Flip every normal"),
    Str("group", ""),
])
def point_cloud_normal(geo, neighbors, orient, viewpoint, radius, reverse, group):
    """Estimate a normal for every point of a point cloud from its neighbours (Houdini's Point Cloud Normal): the plane that fits them
    best; the sign is chosen by 'orient' since a plane has no inside."""
    n = geo.npoints
    if n < 3:
        return geo
    P = geo.P.astype(np.float64)
    _, N, _h, _v = _planes_chunked(P, P, min(int(neighbors) + 1, n))
    if radius > 0:                                                       # a plane through fewer than 3 points inside the radius is not trustworthy
        few = spatial.count_within(P, P, float(radius)) < 3
    else:
        few = None
    if orient == "outward":
        ref = P - P.mean(0)
    elif orient == "inward":
        ref = P.mean(0) - P
    elif orient == "viewpoint":
        ref = np.asarray(viewpoint, np.float64) - P
    elif orient == "up":
        ref = np.tile([0.0, 0.0, 1.0], (n, 1))
    else:
        old = geo.point.get("N")
        ref = old.astype(np.float64) if old is not None and old.shape == (n, 3) else None
    if ref is not None:
        N = np.where((np.einsum("ij,ij->i", N, ref) < 0)[:, None], -N, N)
    if reverse:
        N = -N
    N = normalize_rows(N).astype(np.float32)
    old = geo.point.get("N")
    if few is not None and old is not None and old.shape == (n, 3):
        N = np.where(few[:, None], old, N)
    elif few is not None:
        N = np.where(few[:, None], 0.0, N).astype(np.float32)
    out = geo.copy()
    out.point["N"] = N
    return groups.restrict(geo, out, "point", group)


@node("point_cloud_surface", inputs=["points", Input("cloud", optional=True, doc="The point cloud that defines the surface (default: the points themselves)")],
      category="modify", params=[
    Int("neighbors", 12, min=3, soft_max=64),
    Int("iterations", 1, min=1, soft_max=10, doc="Repeat the projection (moving least squares converges in a few)"),
    Float("strength", 1.0, min=0.0, max=1.0, doc="How far toward the surface each point moves (1: onto it)"),
    Float("max_distance", 0.0, min=0.0, soft_max=10.0, doc="Points farther than this from the surface stay (0: no limit)"),
    Bool("update_normals", True, doc="Write the surface normal at each point's new place as N"),
    Str("group", ""),
])
def point_cloud_surface(points, cloud, neighbors, iterations, strength, max_distance, update_normals, group):
    """Snap points onto the smooth surface a point cloud describes (Houdini's Point Cloud Surface): each point is projected onto a
    plane fitted to the cloud points around it, a few times over."""
    n = points.npoints
    ref = (cloud if cloud is not None else points)
    if ref.npoints < 3 or n == 0:
        return points
    C = ref.P.astype(np.float64)
    P = points.P.astype(np.float64)
    start, N = P.copy(), None
    k = min(int(neighbors), len(C))
    for _ in range(int(iterations)):
        centre, N, _h, _v = _planes_chunked(C, P, k)
        dist = np.einsum("ij,ij->i", P - centre, N)
        P = P - N * (dist * float(strength))[:, None]
    if max_distance > 0:
        far = np.linalg.norm(P - start, axis=1) > max_distance
        P[far] = start[far]
    out = points.copy()
    out.point["P"] = P.astype(np.float32)
    if update_normals and N is not None:
        N = normalize_rows(N)
        refN = points.point.get("N")
        if refN is not None and refN.shape == (n, 3):
            N = np.where((np.einsum("ij,ij->i", N, refN) < 0)[:, None], -N, N)
        out.point["N"] = N.astype(np.float32)
    return groups.restrict(points, out, "point", group)


@node("point_cloud_measure", inputs=["geo", Input("sample", optional=True, doc="Count and measure these points around each point of the first "
                                                                             "input instead of the first input's own points")],
      category="modify", params=[
    Float("radius", 0.0, min=0.0, soft_max=5.0, doc="Search radius for 'neighbors' and 'density' (0: three times the typical point spacing)"),
    Int("neighbors_k", 8, min=1, soft_max=64, doc="Points averaged for 'mean_dist'"),
    Bool("shape", False, doc="Also fit a plane to the neighbours and write curvature (surface variation), linear, planar and spherical "
                             "(how line-like, flat or ball-like the neighbourhood is; they add up to 1)"),
    Str("group", ""),
])
def point_cloud_measure(geo, sample=None, *, radius, neighbors_k, shape, group):
    """Measure how crowded each point is (Houdini's Point Cloud Measure): ``neighbors`` (points within the radius), ``density`` (those
    per unit volume), ``nn_dist`` (distance to the nearest other point) and ``mean_dist`` (mean distance to the nearest k)."""
    n = geo.npoints
    out = geo.copy()
    if n < 2:
        out.point["neighbors"] = np.zeros(n, np.int32)
        out.point["density"] = np.zeros(n, np.float32)
        out.point["nn_dist"] = np.zeros(n, np.float32)
        out.point["mean_dist"] = np.zeros(n, np.float32)
        return out
    P = geo.P.astype(np.float64)
    S = sample.P.astype(np.float64) if sample is not None else P
    own = sample is None                                                   # the point itself is in its own neighbourhood
    r = float(radius) or 3.0 * _spacing(S)
    cnt = spatial.count_within(S, P, r) - (1 if own else 0)
    k = min(int(neighbors_k), len(S) - (1 if own else 0))
    if k < 1:
        raise ValueError("the sample input needs at least one point")
    _idx, dist = spatial.knn(S, P, k + (1 if own else 0))
    d = dist[:, 1:] if own else dist
    out.point["neighbors"] = cnt.astype(np.int32)
    out.point["density"] = (cnt / (4.0 / 3.0 * np.pi * r ** 3)).astype(np.float32)
    out.point["nn_dist"] = d[:, 0].astype(np.float32)
    out.point["mean_dist"] = np.where(np.isfinite(d), d, 0).mean(1).astype(np.float32)
    if shape:
        kk = min(max(int(neighbors_k), 3) + 1, len(S))
        _c, _nrm, _h, vals = _planes_chunked(S, P, kk, weighted=False)
        total = np.maximum(vals.sum(1), 1e-30)
        l3, l2, l1 = vals[:, 0], vals[:, 1], vals[:, 2]                    # ascending
        out.point["curvature"] = (l3 / total).astype(np.float32)
        out.point["linear"] = ((l1 - l2) / np.maximum(l1, 1e-30)).astype(np.float32)
        out.point["planar"] = ((l2 - l3) / np.maximum(l1, 1e-30)).astype(np.float32)
        out.point["spherical"] = (l3 / np.maximum(l1, 1e-30)).astype(np.float32)
    return groups.restrict(geo, out, "point", group)


def _poisson_keep(P, r, priority):
    from .. import pcg
    i, j, _d = spatial.pairs_within(P, r)
    return pcg.greedy_independent(len(P), i, j, priority)


@node("point_cloud_reduce", inputs=["geo"], category="modify", params=[
    Enum("mode", ["distance", "count", "percent", "voxel"], doc="distance: keep points no closer than 'distance'; count: about this many points, "
                                                      "evenly spread; percent: keep this share, at random; voxel: one point per cell of 'voxel_size'"),
    Float("voxel_size", 0.1, min=1e-6, soft_max=2.0, doc="voxel: the cell size"),
    Enum("voxel_point", ["average", "nearest_center", "first"], doc="voxel: the point made for a cell: the average of its points "
                                                                  "(attributes averaged too), the one nearest the cell's middle, or its first"),
    Float("distance", 0.1, min=1e-6, soft_max=2.0),
    Int("count", 1000, min=1, soft_max=100000),
    Float("percent", 50.0, min=0.0, max=100.0),
    Str("priority", "", doc="distance / count: scalar point attribute; higher values survive (default: random)"),
    Int("seed", 0),
])
def point_cloud_reduce(geo, mode, voxel_size, voxel_point, distance, count, percent, priority, seed):
    """Thin a point cloud (Houdini's Point Cloud Reduce, Remove Points by distance): an even, blue-noise-like subset. Attributes follow
    the points that stay."""
    n = geo.npoints
    if n < 2:
        return geo
    rng = np.random.default_rng([int(seed) & 0xFFFFFFFF, 17])
    P = geo.P.astype(np.float64)
    if mode == "voxel":
        cell = np.floor((P - P.min(0)) / float(voxel_size)).astype(np.int64)
        _u, inv = np.unique(cell, axis=0, return_inverse=True)
        inv = inv.reshape(-1)
        m = int(inv.max()) + 1
        if voxel_point == "average":
            cnt = np.maximum(np.bincount(inv, minlength=m), 1)
            first = np.zeros(m, np.int64)
            first[inv[::-1]] = np.arange(n)[::-1]
            out = take_points(geo, first)
            for name in geo.point.names():
                a = np.asarray(geo.point[name])
                if a.dtype.kind != "f":
                    continue
                flat = a.reshape(n, -1).astype(np.float64)
                avg = np.stack([np.bincount(inv, weights=flat[:, c], minlength=m) for c in range(flat.shape[1])], 1) / cnt[:, None]
                out.point[name] = avg.reshape((m,) + a.shape[1:]).astype(a.dtype)
            return out
        if voxel_point == "first":
            first = np.zeros(m, np.int64)
            first[inv[::-1]] = np.arange(n)[::-1]
            return take_points(geo, first)
        centre = (_u + 0.5) * float(voxel_size) + P.min(0)
        d = np.linalg.norm(P - centre[inv], axis=1)
        order = np.lexsort((np.arange(n), d, inv))
        firsts = order[np.r_[True, inv[order][1:] != inv[order][:-1]]]
        return take_points(geo, np.sort(firsts))
    if mode == "percent":
        keep = np.sort(rng.permutation(n)[:int(round(n * percent / 100.0))])
        return take_points(geo, keep)
    pri = rng.random(n)
    if priority:
        a = geo.point.get(priority)
        if a is None or a.ndim != 1:
            raise ValueError(f"no scalar point attribute {priority!r} for priority")
        pri = a.astype(np.float64) + pri * 1e-9
    if mode == "distance":
        keep = _kept(_poisson_keep(P, float(distance), pri))
    else:
        if count >= n:
            return geo
        lo, hi = 0.0, float(np.ptp(P, axis=0).max()) + 1e-9
        keep = np.arange(n)
        for _ in range(16):                                              # bisect the spacing that leaves about `count`
            mid = (lo + hi) / 2
            k = _kept(_poisson_keep(P, mid, pri))
            if len(k) > count:
                lo = mid
            else:
                hi = mid
                keep = k
        if len(keep) > count:
            keep = keep[:count]
    return take_points(geo, np.sort(keep))


def _kept(result):
    """``greedy_independent`` returns the kept point numbers or a mask: normalise to indices."""
    r = np.asarray(result)
    return np.flatnonzero(r) if r.dtype == bool else r.astype(np.int64)


# -- Points from Volume --------------------------------------------------------------------------------------------------
@node("points_from_volume", inputs=["geo"], category="generate", params=[
    Enum("mode", ["grid", "random", "poisson"], doc="grid: a regular lattice; random: random points; poisson: random points no closer "
                                                   "than the spacing"),
    Float("spacing", 0.1, min=1e-6, soft_max=1.0, doc="Distance between points (grid / poisson); random uses it for pscale only"),
    Float("jitter", 0.0, min=0.0, max=1.0, doc="grid: shake each point by this fraction of the spacing"),
    Int("count", 1000, min=1, soft_max=100000, doc="random / poisson: number of candidate points"),
    Bool("shell_only", False, doc="Only points within one spacing of the surface"),
    Float("radius_scale", 0.5, min=0.0, soft_max=2.0, doc="pscale = spacing x this"),
    Int("seed", 0),
])
def points_from_volume(geo, mode, spacing, jitter, count, shell_only, radius_scale, seed):
    """Fill the inside of a closed mesh with points (Houdini's Points from Volume, for polygon volumes). Each point has a
    ``pscale`` of half the spacing."""
    from ..raycast import TriGrid, inside
    from ._build import require_polygons
    require_polygons(geo, "points_from_volume")
    tris, _ = fan_triangulate(geo.topology)
    if len(tris) == 0:
        raise ValueError("points_from_volume needs a closed polygon mesh")
    P = geo.P.astype(np.float64)
    grid = TriGrid(P[geo.topology.vertex_point[tris]])
    lo, hi = P.min(0), P.max(0)
    ext = hi - lo
    s = float(spacing)
    rng = np.random.default_rng([int(seed) & 0xFFFFFFFF, 21])
    if mode == "grid":
        counts = np.maximum(np.floor(ext / s + 1e-9).astype(np.int64), 1)             # cell centres: nothing sits on the surface
        guard_count(int(np.prod(counts)), "grid cells", "use a larger spacing")
        axes = [lo[a] + (ext[a] - (counts[a] - 1) * s) / 2 + np.arange(counts[a]) * s for a in range(3)]
        cand = np.stack(np.meshgrid(*axes, indexing="ij"), -1).reshape(-1, 3)
        if jitter > 0:
            cand = cand + (rng.random(cand.shape) * 2 - 1) * (s * jitter / 2)
    else:
        guard_count(int(count), "points", "use a smaller count")
        cand = lo + rng.random((int(count) * 3, 3)) * ext
    m = inside(grid, cand, float(ext.max()) + 1.0)
    pts = cand[m]
    if mode != "grid":
        pts = pts[:int(count)]
        if mode == "poisson" and len(pts) > 1:
            pts = pts[_kept(_poisson_keep(pts, s, rng.random(len(pts))))]
    if shell_only and len(pts):
        _, d, _t = grid.closest(pts)
        pts = pts[d <= s]
    return _cloud(pts, pscale=np.full(len(pts), s * float(radius_scale), np.float32), id=np.arange(len(pts), dtype=np.int32))


# -- Scatter in Texture Mask ---------------------------------------------------------------------------------------------
@node("scatter_in_texture_mask", inputs=["geo", Input("mask", doc="The mask image as a pixel grid (blender_image, image_file)")],
      category="generate", params=[
    Int("count", 1000, min=0, soft_max=100000, doc="How many points to place"),
    Enum("channel", ["luminance", "red", "green", "blue", "alpha"], doc="Which part of the image is the mask"),
    Str("uv_attrib", "uv", doc="The surface's UV coordinates (a vertex or point attribute); they pick the pixel"),
    Float("gamma", 1.0, min=0.01, soft_max=4.0, doc="Contrast: the chance of keeping a point is the mask value to this power"),
    Float("threshold", 0.0, min=0.0, max=1.0, doc="Mask values at or below this never get points"),
    Bool("invert", False),
    Float("density_scale", 1.0, min=0.0, soft_max=2.0, doc="Multiplies the mask: below 1 thins the points everywhere"),
    Int("seed", 0),
])
def scatter_in_texture_mask(geo, mask, count, channel, uv_attrib, gamma, threshold, invert, density_scale, seed):
    """Scatter points on a surface where an image is bright (Houdini's Scatter in Texture Mask). Output: ``N``, ``primnum``, ``uv`` and the
    mask value as ``mask``."""
    cols, rows = images.lattice(mask)
    cd = mask.point.get("Cd")
    if cd is None:
        raise ValueError("the mask has no Cd colour: feed it a pixel grid from blender_image / image_file")
    cd = cd.astype(np.float32)
    if channel == "luminance":
        vals = images.luminance(cd)
    elif channel == "alpha":
        a = mask.point.get("Alpha")
        vals = a.astype(np.float32) if a is not None else np.ones(len(cd), np.float32)
    else:
        vals = cd[:, ("red", "green", "blue").index(channel)]
    img = vals.reshape(rows, cols)
    topo = geo.topology
    tris, tri_prim = fan_triangulate(topo)
    if len(tris) == 0:
        raise ValueError("the surface has no faces to scatter on")
    uv_v = geo.vertex.get(uv_attrib)
    uv_p = geo.point.get(uv_attrib)
    if uv_v is not None and uv_v.ndim == 2 and uv_v.shape[1] >= 2:
        corner_uv = uv_v[:, :2].astype(np.float64)
    elif uv_p is not None and uv_p.ndim == 2 and uv_p.shape[1] >= 2:
        corner_uv = uv_p[topo.vertex_point, :2].astype(np.float64)
    else:
        raise ValueError(f"the surface has no {uv_attrib!r} (UV) attribute to look the mask up with")
    P = geo.P.astype(np.float64)
    pts = topo.vertex_point[tris]
    a, b, c = P[pts[:, 0]], P[pts[:, 1]], P[pts[:, 2]]
    cross = np.cross(b - a, c - a)
    area = 0.5 * np.linalg.norm(cross, axis=1)
    cdf = np.cumsum(area)
    if cdf[-1] <= 0:
        raise ValueError("the surface has no area")
    rng = np.random.default_rng([int(seed) & 0xFFFFFFFF, 23])
    guard_count(int(count), "points", "use a smaller count")
    want = int(count)
    got_pos, got_t, got_uv, got_m = [], [], [], []
    have, tries = 0, 0
    rate = 0.5
    while have < want and tries < 40:
        tries += 1
        m_ = int(min(max(256, (want - have) / max(rate, 0.01) * 1.3), 4_000_000))
        t = np.minimum(np.searchsorted(cdf, rng.random(m_) * cdf[-1], side="right"), len(area) - 1)
        r1, r2 = rng.random(m_), rng.random(m_)
        flip = r1 + r2 > 1
        r1, r2 = np.where(flip, 1 - r1, r1), np.where(flip, 1 - r2, r2)
        pos = a[t] + (b[t] - a[t]) * r1[:, None] + (c[t] - a[t]) * r2[:, None]
        uv = corner_uv[tris[t, 0]] * (1 - r1 - r2)[:, None] + corner_uv[tris[t, 1]] * r1[:, None] + corner_uv[tris[t, 2]] * r2[:, None]
        fx = np.clip(uv[:, 0], 0, 1) * (cols - 1)
        fy = np.clip(uv[:, 1], 0, 1) * (rows - 1)
        x0, y0 = np.minimum(fx.astype(np.int64), cols - 2 if cols > 1 else 0), np.minimum(fy.astype(np.int64), rows - 2 if rows > 1 else 0)
        x1, y1 = np.minimum(x0 + 1, cols - 1), np.minimum(y0 + 1, rows - 1)
        tx, ty = fx - x0, fy - y0
        mv = (img[y0, x0] * (1 - tx) * (1 - ty) + img[y0, x1] * tx * (1 - ty) + img[y1, x0] * (1 - tx) * ty + img[y1, x1] * tx * ty)
        mv = np.clip((1.0 - mv if invert else mv) * float(density_scale), 0.0, 1.0)
        ok = (mv > threshold) & (rng.random(m_) < mv ** gamma)
        rate = max(float(ok.mean()), 1e-3)
        got_pos.append(pos[ok]); got_t.append(t[ok]); got_uv.append(uv[ok]); got_m.append(mv[ok])
        have += int(ok.sum())
    if not have:
        raise ValueError("the mask rejected every candidate point: it is black here (or the threshold is too high)")
    pos = np.concatenate(got_pos)[:want]
    t = np.concatenate(got_t)[:want]
    out = _cloud(pos, N=normalize_rows(cross[t].astype(np.float32)), primnum=tri_prim[t].astype(np.int32),
                 uv=np.concatenate(got_uv)[:want].astype(np.float32), mask=np.concatenate(got_m)[:want].astype(np.float32))
    return out


# -- Particle Trail ------------------------------------------------------------------------------------------------------
@node("particle_trail", inputs=["geo"], category="generate", params=[
    Str("velocity", "v", doc="Vector point attribute the trail follows backwards (where the point was a moment ago)"),
    Float("length", 0.5, min=0.0, soft_max=5.0, doc="How far back in time the trail reaches (seconds)"),
    Int("samples", 8, min=2, soft_max=64, doc="Points per trail"),
    Float("gravity", 0.0, soft_max=20.0, doc="Constant downward acceleration (m/s^2) bending the trail; 0 for a straight one"),
    Bool("keep_points", False, doc="Also keep the original points (as separate points)"),
])
def particle_trail(geo, velocity, length, samples, gravity, keep_points):
    """A short line behind every point, along its velocity (Houdini's Particle Trail, in its stateless form: it needs no earlier
    frames). Each trail is one open curve from the point backwards; ``trail`` is 0 at the point and 1 at the far end, and the point
    attributes are copied onto the trail's points."""
    v = geo.point.get(velocity)
    if v is None or v.ndim != 2 or v.shape[1] != 3:
        raise ValueError(f"no vector point attribute {velocity!r}: run point_velocity first")
    n, k = geo.npoints, int(samples)
    guard_count(n * k, "trail points", "use fewer samples")
    t = np.linspace(0.0, 1.0, k) * float(length)                         # seconds back in time
    P = geo.P.astype(np.float64)[:, None, :] - v.astype(np.float64)[:, None, :] * t[None, :, None]
    if gravity:
        P[..., 2] += 0.5 * float(gravity) * t[None, :] ** 2
    src = np.repeat(np.arange(n), k)
    out = Geometry.from_arrays(P.reshape(-1, 3).astype(np.float32), np.arange(n * k, dtype=np.int32),
                               (np.arange(n + 1) * k).astype(np.int32), closed=np.zeros(n, bool))
    for name in geo.point.names():
        if name != "P":
            out.point[name] = np.asarray(geo.point[name])[src]
    out.point["trail"] = np.tile(np.linspace(0.0, 1.0, k, dtype=np.float32), n)
    if keep_points and n:
        from .merge import merge
        out = merge([out, take_points(geo, np.arange(n))])
    return out
