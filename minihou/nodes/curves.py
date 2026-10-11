"""Curves: open polylines and the nodes that make and use them (Houdini's Line, Curve, Resample, Carve, Sweep,
PolyPath, Convert Line).

A curve is a primitive whose topology flag ``closed`` is False (see :class:`minihou.geometry.Topology`). Closed
polygons count as closed curves where it makes sense (resample/carve walk their outline; sweep makes a ring tube).
In Blender, curves arrive as loose edges of the output mesh (Geometry Nodes' *Mesh to Curve* turns them into real
curves); sweep them here for render geometry.
"""
import numpy as np

from .. import groups, topo
from ..sdk import Bool, Enum, Float, Int, Str, Vec3, node
from . import _build as B
from ._util import normalize_rows


# -- helpers ----------------------------------------------------------------------------------------------------
def curve_arrays(geo, prims=None):
    """Per selected primitive: its vertex range, closedness and arc lengths, as flat arrays.

    Returns (prims, starts, sizes, closed, vert_index, seg_len) where ``vert_index`` lists the vertices of the chosen
    primitives in order, ``seg_len[i]`` is the length from vertex i to the next one along its curve (0 after the last
    vertex of an open curve)."""
    t = geo.topology
    prims = np.arange(t.nprims) if prims is None else np.asarray(prims, np.int64)
    offs = t.prim_offsets.astype(np.int64)
    starts, sizes = offs[prims], np.diff(offs)[prims]
    closed = t.closed_mask()[prims]
    local = np.arange(sizes.sum()) - np.repeat(np.cumsum(sizes) - sizes, sizes)
    vert = np.repeat(starts, sizes) + local
    P = geo.P.astype(np.float64)[t.vertex_point[vert]]
    last = local == np.repeat(sizes - 1, sizes)
    nxt = np.where(last, np.repeat(np.cumsum(sizes) - sizes, sizes), np.arange(len(vert)) + 1)
    seg = np.linalg.norm(P[nxt] - P, axis=1)
    seg = np.where(last & ~np.repeat(closed, sizes), 0.0, seg)
    return prims, starts, sizes, closed, vert, seg


def _curves_geometry(P, sizes, closed, point=None, prim=None, detail=None, vertex=None):
    """Geometry with one curve per entry of ``sizes`` over consecutive points."""
    n = int(np.sum(sizes))
    return B.make(P, np.arange(n), B.offsets_from_sizes(sizes), point, vertex, prim, detail, closed)


def chain_segments(npoints, a, b):
    """Join 2-point segments a[i]-b[i] into polylines. Returns (list of point-index arrays, closed flags). Points with
    more than two segments end chains there (branches become separate curves)."""
    a, b = np.asarray(a, np.int64), np.asarray(b, np.int64)
    deg = np.bincount(np.r_[a, b], minlength=npoints)
    adj = [[] for _ in range(npoints)]
    for i, (x, y) in enumerate(zip(a.tolist(), b.tolist())):
        adj[x].append((y, i))
        adj[y].append((x, i))
    used = np.zeros(len(a), bool)
    chains, closed = [], []

    def walk(start, first_edge):
        pts = [start]
        e, p = first_edge, start
        while True:
            used[e] = True
            q = a[e] if b[e] == p else b[e]
            pts.append(int(q))
            if deg[q] != 2:
                return pts, False
            nxt = [(r, j) for r, j in adj[q] if not used[j]]
            if not nxt:
                return pts, pts[0] == pts[-1]
            p, e = q, nxt[0][1]

    for s in np.flatnonzero((deg != 2) & (deg > 0)).tolist():   # chains from every end / branch point
        for _, e in adj[s]:
            if not used[e]:
                pts, _ = walk(s, e)
                chains.append(np.asarray(pts, np.int64))
                closed.append(False)
    for e in range(len(a)):                                       # what is left are closed loops
        if not used[e]:
            pts, _ = walk(int(a[e]), e)
            chains.append(np.asarray(pts[:-1], np.int64))
            closed.append(True)
    return chains, np.asarray(closed, bool)


# -- sources ----------------------------------------------------------------------------------------------------
@node("line", category="source", params=[
    Vec3("origin", (0.0, 0.0, 0.0)),
    Vec3("direction", (0.0, 0.0, 1.0)),
    Float("length", 1.0, soft_min=0.0, soft_max=10.0),
    Int("points", 10, min=2, soft_max=1000),
])
def line(origin, direction, length, points):
    """A straight open curve (Houdini's Line)."""
    d = np.asarray(direction, np.float64)
    d = d / max(np.linalg.norm(d), 1e-12)
    t = np.linspace(0.0, length, points)
    P = np.asarray(origin, np.float64) + t[:, None] * d
    return _curves_geometry(P.astype(np.float32), [points], [False])


@node("helix", category="source", params=[
    Float("radius", 1.0, min=0.0, soft_max=10.0),
    Float("radius_end", -1.0, doc="Radius at the top (negative: same as radius) - a cone spiral"),
    Float("height", 2.0, soft_min=0.0, soft_max=10.0),
    Float("turns", 3.0, min=0.0, soft_max=20.0),
    Int("points", 120, min=2, soft_max=5000),
])
def helix(radius, radius_end, height, turns, points):
    """A spiral curve around +Z (springs, vines, stair rails)."""
    t = np.linspace(0.0, 1.0, points)
    r1 = radius if radius_end < 0 else radius_end
    r = radius + (r1 - radius) * t
    a = t * turns * 2 * np.pi
    P = np.stack([r * np.cos(a), r * np.sin(a), t * height], 1)
    return _curves_geometry(P.astype(np.float32), [points], [False])


@node("curve", category="source", params=[
    Str("points", "0,0,0  1,0,0  1,1,0  2,1,1", doc="Point positions 'x,y,z' separated by spaces"),
    Bool("closed", False),
])
def curve(points, closed):
    """A curve through typed-in points (Houdini's Curve SOP, as text)."""
    try:
        P = np.array([[float(c) for c in tok.split(",")] for tok in points.split()], np.float64)
    except ValueError:
        raise ValueError("points must look like 'x,y,z x,y,z ...'") from None
    if P.ndim != 2 or P.shape[1] != 3 or len(P) < 2:
        raise ValueError("give at least two points, each as x,y,z")
    return _curves_geometry(P.astype(np.float32), [len(P)], [bool(closed)])


# -- conversion -------------------------------------------------------------------------------------------------
@node("convert_line", inputs=["geo"], category="curve", params=[
    Bool("join", True, doc="Join the edges into long polylines (else one 2-point curve per edge)"),
])
def convert_line(geo, join):
    """Turn every edge into curves (Houdini's Convert Line), e.g. for wireframes with sweep. Point attributes are kept."""
    e = topo.edges(geo)
    if join:
        chains, closed = chain_segments(geo.npoints, e.points[:, 0], e.points[:, 1])
        vp = np.concatenate(chains) if chains else np.zeros(0, np.int64)
        sizes = [len(c) for c in chains]
    else:
        vp, sizes, closed = e.points.reshape(-1), np.full(len(e.points), 2), np.zeros(len(e.points), bool)
    if not len(sizes):
        return B.make(geo.P, np.zeros(0, np.int64), [0], dict(geo.point.items()), detail=geo.detail)
    return B.make(geo.P, vp, B.offsets_from_sizes(sizes), dict(geo.point.items()), None, None, geo.detail,
                  np.asarray(closed, bool))


@node("polypath", inputs=["geo"], category="curve", params=[])
def polypath(geo):
    """Join curves that share end points into single polylines (Houdini's PolyPath). Polygons pass through."""
    t = geo.topology
    if not t.has_open:
        return geo
    openp = np.flatnonzero(~t.closed)
    _, _, sizes, _, vert, _ = curve_arrays(geo, openp)
    vp = t.vertex_point.astype(np.int64)[vert]
    first = np.repeat(np.cumsum(sizes) - sizes, sizes)
    is_last = np.arange(len(vert)) - first == np.repeat(sizes - 1, sizes)
    a, b = vp[~is_last], vp[np.flatnonzero(~is_last) + 1]
    chains, closed = chain_segments(geo.npoints, a, b)
    keep = np.flatnonzero(t.closed)
    poly = keep_polygons(geo, keep)
    cv = np.concatenate(chains) if chains else np.zeros(0, np.int64)
    out_vp = np.concatenate([poly.topology.vertex_point.astype(np.int64), cv])
    sizes_all = np.concatenate([poly.topology.prim_sizes(), [len(c) for c in chains]]).astype(np.int64)
    pad = lambda table, k: {n: np.concatenate([a_, np.zeros((k,) + a_.shape[1:], a_.dtype)]) for n, a_ in table.items()}  # noqa: E731
    return B.make(geo.P, out_vp, B.offsets_from_sizes(sizes_all), dict(geo.point.items()), pad(poly.vertex, len(cv)),
                  pad(poly.prim, len(chains)), geo.detail, np.r_[np.ones(len(keep), bool), closed])


def keep_polygons(geo, prims):
    """The given primitives (all points kept)."""
    from .group import keep_prims
    m = np.zeros(geo.nprims, bool)
    m[prims] = True
    return keep_prims(geo, m, remove_unused_points=False)


# -- resample / carve -------------------------------------------------------------------------------------------
def _sample(geo, prims, s_lists):
    """Sample curves at arc-length positions. ``s_lists[i]`` are positions along prim ``prims[i]``; returns the new
    curve geometry (one curve per prim) with point attributes interpolated and prim attributes copied."""
    _, starts, sizes, closed, vert, seg = curve_arrays(geo, prims)
    vp = geo.topology.vertex_point.astype(np.int64)
    cum_start = np.cumsum(sizes) - sizes
    total = np.add.reduceat(seg, cum_start) if len(sizes) else np.zeros(0)
    # global cumulative arc length with a gap of 1 between curves so searchsorted never crosses curves
    offset = np.r_[0.0, np.cumsum(total + 1.0)[:-1]] if len(sizes) else np.zeros(0)
    curve_of_v = np.repeat(np.arange(len(sizes)), sizes)
    before = np.cumsum(seg) - seg                                       # arc length up to each vertex, all curves
    cum = before - before[np.repeat(cum_start, sizes)] + offset[curve_of_v]
    counts = np.array([len(s) for s in s_lists], np.int64)
    s_all = np.concatenate(s_lists) if len(s_lists) else np.zeros(0)
    owner = np.repeat(np.arange(len(sizes)), counts)
    g = s_all + offset[owner]
    k = np.searchsorted(cum, g, side="right") - 1
    k = np.clip(k, cum_start[owner], cum_start[owner] + sizes[owner] - 1)
    last = k == cum_start[owner] + sizes[owner] - 1
    k_next = np.where(last, cum_start[owner], k + 1)                    # closed curves wrap to the start
    seg_k = seg[k]
    tt = np.where(seg_k > 0, (g - cum[k]) / np.where(seg_k > 0, seg_k, 1.0), 0.0).clip(0, 1)
    pa, pb = vp[vert[k]], vp[vert[k_next]]
    pts = B.lerp(geo.point, pa, pb, tt)
    pts["P"] = (geo.P[pa] * (1 - tt[:, None]) + geo.P[pb] * tt[:, None]).astype(np.float32)
    u = np.where(total[owner] > 0, s_all / np.where(total[owner] > 0, total[owner], 1.0), 0.0)
    pts["curveu"] = u.astype(np.float32)
    return _curves_geometry(pts["P"], counts, closed, pts, B.gather(geo.prim, prims), geo.detail), total


@node("resample", inputs=["geo"], category="curve", params=[
    Enum("by", ["length", "count"], doc="length: segments of about 'length'; count: exactly 'segments' per curve"),
    Float("length", 0.1, min=1e-6, soft_max=2.0),
    Int("segments", 20, min=1, soft_max=1000),
    Str("group", "", doc="Primitives to resample (others are dropped); empty: all"),
])
def resample(geo, by, length, segments, group):
    """Even spacing along curves (Houdini's Resample). Closed polygons are resampled along their outline. Adds
    curveu (0 at the start, 1 at the end) for ramps, carve and sweep."""
    m = groups.mask(geo, "prim", group)
    prims = np.flatnonzero(m) if m is not None else np.arange(geo.nprims)
    prims = prims[geo.topology.prim_sizes()[prims] >= 2]
    _, _, _, closed, _, seg = curve_arrays(geo, prims)
    sizes = geo.topology.prim_sizes()[prims]
    total = np.add.reduceat(seg, np.cumsum(sizes) - sizes) if len(prims) else np.zeros(0)
    s_lists = []
    for L, c in zip(total.tolist(), closed.tolist()):
        n = segments if by == "count" else max(int(np.ceil(L / length - 1e-9)), 1)
        s = np.linspace(0.0, L, n + 1)
        s_lists.append(s[:-1] if c else s)
    out, _ = _sample(geo, prims, s_lists)
    return out


@node("carve", inputs=["geo"], category="curve", params=[
    Float("start", 0.0, min=0.0, max=1.0, doc="Keep from this fraction of each curve's length..."),
    Float("end", 1.0, min=0.0, max=1.0, doc="...to this one (animate it to grow a curve)"),
])
def carve(geo, start, end):
    """Keep a portion of each open curve by length (Houdini's Carve). Original points inside are kept exactly; the
    ends are cut with interpolated attributes. Polygons pass through."""
    t = geo.topology
    openp = np.flatnonzero(~t.closed_mask() & (t.prim_sizes() >= 2))
    if not len(openp):
        return geo
    lo, hi = sorted((float(start), float(end)))
    _, _, sizes, _, _, seg = curve_arrays(geo, openp)
    first = np.cumsum(sizes) - sizes
    s_lists = []
    for i, L in enumerate(np.add.reduceat(seg, first).tolist()):
        cum = np.r_[0.0, np.cumsum(seg[first[i]:first[i] + sizes[i] - 1])]
        a, b = lo * L, hi * L
        inner = cum[(cum > a + 1e-12) & (cum < b - 1e-12)]
        s_lists.append(np.unique(np.r_[a, inner, b]) if b > a else np.array([a, a]))
    curves, _ = _sample(geo, openp, s_lists)
    rest = np.flatnonzero(t.closed_mask())
    if not len(rest):
        return curves
    from .merge import merge
    return merge([keep_polygons(geo, rest), curves])


# -- sweep ------------------------------------------------------------------------------------------------------
def _frames(P, sizes, closed):
    """Rotation-minimising frames (double reflection, Wang et al. 2008) along every curve: tangents and normals."""
    first = np.cumsum(sizes) - sizes
    n = len(P)
    curve = np.repeat(np.arange(len(sizes)), sizes)
    local = np.arange(n) - first[curve]
    prev = np.where(local == 0, np.where(closed[curve], first[curve] + sizes[curve] - 1, np.arange(n)), np.arange(n) - 1)
    nxt = np.where(local == sizes[curve] - 1, np.where(closed[curve], first[curve], np.arange(n)), np.arange(n) + 1)
    T = normalize_rows(P[nxt] - P[prev]).astype(np.float64)
    N = np.zeros_like(T)
    # first frame: any vector perpendicular to the first tangent
    t0 = T[first]
    helper = np.where((np.abs(t0[:, 2]) < 0.9)[:, None], [[0.0, 0, 1]], [[1.0, 0, 0]])
    N[first] = normalize_rows(np.cross(np.cross(t0, helper), t0)).astype(np.float64)
    for k in range(1, int(sizes.max()) if len(sizes) else 0):         # march all curves one point at a time
        cur = first[sizes > k] + k
        p0, p1 = P[cur - 1], P[cur]
        v1 = p1 - p0
        c1 = np.maximum((v1 * v1).sum(1), 1e-30)
        rL = N[cur - 1] - (2 / c1 * (v1 * N[cur - 1]).sum(1))[:, None] * v1
        tL = T[cur - 1] - (2 / c1 * (v1 * T[cur - 1]).sum(1))[:, None] * v1
        v2 = T[cur] - tL
        c2 = np.maximum((v2 * v2).sum(1), 1e-30)
        N[cur] = rL - (2 / c2 * (v2 * rL).sum(1))[:, None] * v2
    N = normalize_rows(N - (N * T).sum(1)[:, None] * T).astype(np.float64)
    return T, N


@node("sweep", inputs=["geo"], category="curve", params=[
    Float("radius", 0.05, min=0.0, soft_max=1.0, doc="Tube radius; multiplied by point 'width' or 'pscale' if present"),
    Int("divisions", 8, min=3, soft_max=64, doc="Sides around the tube"),
    Bool("caps", True, doc="Close the ends of open curves"),
    Float("twist", 0.0, soft_min=-720.0, soft_max=720.0, doc="Degrees of twist from start to end"),
])
def sweep(geo, radius, divisions, caps, twist):
    """Tubes along curves (Houdini's Sweep / PolyWire): rotation-minimising frames (no flips), radius scaled by
    point width or pscale, uv along (u) and around (v) the tube, curve point attributes kept on the rings.
    Closed primitives (closed curves, polygon outlines) make closed rings."""
    t = geo.topology
    prims = np.flatnonzero(t.prim_sizes() >= 2)
    if not len(prims):
        raise ValueError("sweep needs curves; make them with line, curve, helix, convert_line or resample")
    _, _, sizes, closed, vert, seg = curve_arrays(geo, prims)
    vp = t.vertex_point.astype(np.int64)[vert]
    P = geo.P.astype(np.float64)[vp]
    T, N = _frames(P, sizes, closed)
    Bn = np.cross(T, N)
    first = np.cumsum(sizes) - sizes
    curve = np.repeat(np.arange(len(sizes)), sizes)
    local = np.arange(len(vp)) - first[curve]
    total = np.add.reduceat(seg, first)
    cum = np.cumsum(seg) - seg
    u = (cum - cum[first][curve]) / np.where(total > 0, total, 1.0)[curve]
    w = np.full(len(vp), radius)
    for nm in ("width", "pscale"):
        if nm in geo.point:
            w = w * geo.point[nm].reshape(geo.npoints, -1)[:, 0].astype(np.float64)[vp]
            break
    D = int(divisions)
    ang = np.arange(D) * 2 * np.pi / D + np.radians(twist) * u[:, None]
    ring = P[:, None, :] + w[:, None, None] * (np.cos(ang)[..., None] * N[:, None, :] + np.sin(ang)[..., None] * Bn[:, None, :])
    newP = ring.reshape(-1, 3)
    ring_id = np.arange(len(vp))
    # quads between ring i and the next ring of the same curve
    has_next = (local < sizes[curve] - 1) | closed[curve]
    i0 = ring_id[has_next]
    i1 = np.where(local[has_next] == sizes[curve[has_next]] - 1, first[curve[has_next]], i0 + 1)
    j = np.arange(D)
    a = (i0[:, None] * D + j[None, :]).ravel()
    b = (i0[:, None] * D + (j[None, :] + 1) % D).ravel()
    c = (i1[:, None] * D + (j[None, :] + 1) % D).ravel()
    d = (i1[:, None] * D + j[None, :]).ravel()
    quads = np.stack([a, b, c, d], 1)
    # uv per corner: u along, v around (the seam closes at v = 1)
    u0 = np.repeat(u[i0], D)
    u1 = np.repeat(np.where(i1 == first[curve[i0]], 1.0, u[i1]) if len(i0) else u[i1], D)
    v0 = np.tile(j / D, len(i0))
    v1 = np.tile((j + 1) / D, len(i0))
    uv = np.stack([np.c_[u0, v0], np.c_[u0, v1], np.c_[u1, v1], np.c_[u1, v0]], 1).reshape(-1, 2)
    prim_src = np.repeat(curve[i0], D)
    polys_vp, sizes_out, uv_parts, prim_parts = [quads.ravel()], [np.full(len(quads), 4)], [uv], [prim_src]
    if caps:
        open_c = np.flatnonzero(~closed)
        if len(open_c):
            s_ring = first[open_c]
            e_ring = first[open_c] + sizes[open_c] - 1
            start_cap = (s_ring[:, None] * D + j[::-1][None, :]).ravel()       # reversed: faces backwards
            end_cap = (e_ring[:, None] * D + j[None, :]).ravel()
            polys_vp += [start_cap, end_cap]
            sizes_out += [np.full(len(open_c), D), np.full(len(open_c), D)]
            cap_uv = np.c_[0.5 + 0.5 * np.cos(j * 2 * np.pi / D), 0.5 + 0.5 * np.sin(j * 2 * np.pi / D)]
            uv_parts += [np.tile(cap_uv[::-1], (len(open_c), 1)), np.tile(cap_uv, (len(open_c), 1))]
            prim_parts += [open_c, open_c]
    point = {nm: np.repeat(a_, D, axis=0) for nm, a_ in B.gather(geo.point, vp).items() if nm not in ("P", "N")}
    out = B.make(newP.astype(np.float32), np.concatenate(polys_vp), B.offsets_from_sizes(np.concatenate(sizes_out)),
                 point, {"uv": np.concatenate(uv_parts).astype(np.float32)},
                 B.gather(geo.prim, prims[np.concatenate(prim_parts)]), geo.detail)
    out.point["curveu"] = np.repeat(u, D).astype(np.float32)
    return out


# -- path deform ------------------------------------------------------------------------------------------------
@node("path_deform", inputs=["geo", "curve"], category="deform", params=[
    Enum("axis", ["x", "y", "z"], doc="The geometry's length axis; it is laid along the curve from its first point"),
    Enum("frame", ["minimal_rotation", "up_vector"], doc="minimal_rotation: cross-sections turn as little as "
         "possible (up is matched at the start); up_vector: they keep `up` everywhere (roads, slides, banners)"),
    Vec3("up", (0.0, 0.0, 1.0), doc="Where the geometry's up (Z, or Y for axis z) points"),
    Float("offset", 0.0, soft_min=-10.0, soft_max=10.0, doc="Slide along the curve (distance)"),
    Bool("stretch", False, doc="Scale the length to fit the whole curve"),
    Float("twist", 0.0, soft_min=-720.0, soft_max=720.0, doc="Degrees of twist over the curve's length"),
])
def path_deform(geo, curve, axis, frame, up, offset, stretch, twist):
    """Bend geometry along a curve (Houdini's Path Deform, Blender's Curve modifier). Cross-sections follow
    rotation-minimising frames, so nothing twists unless asked; beyond the ends the geometry continues straight.
    Uses the first primitive of the curve input; rotates N."""
    if geo.npoints == 0:
        return geo
    if curve.nprims == 0:
        raise ValueError("the curve input is empty")
    prims, starts, sizes, closed, vert, seg = curve_arrays(curve, [0])
    if sizes[0] < 2:
        raise ValueError("the curve needs at least two points")
    C = curve.P.astype(np.float64)[curve.topology.vertex_point[vert]]
    if closed[0]:
        C = np.vstack([C, C[:1]])
    T, N = _frames(C, np.array([len(C)]), np.array([False]))
    s = np.concatenate([[0.0], np.cumsum(np.linalg.norm(np.diff(C, axis=0), axis=1))])
    L = float(s[-1])
    if L <= 0:
        raise ValueError("the curve has no length")
    u = np.asarray(up, np.float64)
    if frame == "up_vector":                                # up projected off each tangent (RMF where T is along up)
        upp = u[None] - (T @ u)[:, None] * T
        ok = np.linalg.norm(upp, axis=1) > 1e-6
        N = np.where(ok[:, None], normalize_rows(upp), N)
    upp = u - (u @ T[0]) * T[0]                             # align the start frame's normal with `up`
    if frame == "minimal_rotation" and np.linalg.norm(upp) > 1e-9:
        upp /= np.linalg.norm(upp)
        ang = np.arctan2(np.cross(N[0], upp) @ T[0], N[0] @ upp)
        Bv = np.cross(T, N)
        N = N * np.cos(ang) + Bv * np.sin(ang)
    Bv = np.cross(T, N)
    k = "xyz".index(axis)
    P = geo.P.astype(np.float64)
    along = P[:, k] - P[:, k].min()
    if stretch:
        along *= L / max(float(np.ptp(P[:, k])), 1e-12)
    along = along + offset
    # images of the local axes: the length axis -> T, the up axis -> N, the third keeps the frame right-handed
    i = np.clip(np.searchsorted(s, along, side="right") - 1, 0, len(s) - 2)
    f = np.clip((along - s[i]) / np.maximum(s[i + 1] - s[i], 1e-30), 0.0, 1.0)[:, None]
    base = C[i] * (1 - f) + C[i + 1] * f
    Tp = normalize_rows(T[i] * (1 - f) + T[i + 1] * f).astype(np.float64)
    Np = N[i] * (1 - f) + N[i + 1] * f
    Np = normalize_rows(Np - (Np * Tp).sum(1)[:, None] * Tp).astype(np.float64)
    extra = np.where(along < 0, along, np.where(along > L, along - L, 0.0))[:, None]
    base = base + Tp * extra                                  # past the ends: straight on
    if twist:
        a = np.radians(twist) * np.clip(along, 0.0, L) / L
        Bp = np.cross(Tp, Np)
        Np = Np * np.cos(a)[:, None] + Bp * np.sin(a)[:, None]
    Bp = np.cross(Tp, Np)
    cols = {"x": (Tp, -Bp, Np), "y": (Bp, Tp, Np), "z": (-Bp, Np, Tp)}[axis]
    F = np.stack(cols, 2)                                     # (n, 3, 3): columns = where x, y, z go
    local = P.copy()
    local[:, k] = 0.0
    out = geo.copy()
    out.point["P"] = (base + np.einsum("nij,nj->ni", F, local)).astype(np.float32)
    if "N" in geo.point:
        out.point["N"] = np.einsum("nij,nj->ni", F, geo.point["N"].astype(np.float64)).astype(np.float32)
    return out
