"""Convex hull, dissolve (merging flat faces into n-gons), skin (loft) and revolve."""
import numpy as np

from .. import hull as hl
from .. import topo
from ..geometry import Geometry
from ..outgroups import OutGroup, OutGroups
from ..sdk import Bool, Enum, Float, Int, Str, Vec3, node
from . import _build as B
from ._util import normalize_rows
from .model import _prim_sel, compact_points


@node("convex_hull", inputs=["geo"], category="modify")
def convex_hull(geo):
    """The smallest convex shape holding all the input points: a closed triangle mesh facing outwards, on those of
    the points that are corners of the hull (they keep their attributes). Primitives are ignored. With fewer than four
    points, or points that are all in one plane, there is no volume and the points come back unchanged."""
    P = geo.P.astype(np.float64)
    tri = hl.convex_hull(P)
    if not len(tri):
        return Geometry.points_only(geo.P) if geo.nprims == 0 else B.make(
            geo.P, np.zeros(0, np.int32), np.zeros(1, np.int32), dict(geo.point.items()), detail=geo.detail)
    hull = B.make(geo.P, tri.reshape(-1).astype(np.int32), np.arange(len(tri) + 1, dtype=np.int32) * 3,
                  dict(geo.point.items()), detail=geo.detail)
    return compact_points(hull)


# -- dissolve ---------------------------------------------------------------------------------------------------
def _flat_groups(geo, sel, cos_tol):
    """Representative polygon per polygon: neighbours across a shared edge join one flat region when their normals
    agree (within the angle), flattest pairs first, and every region stays within the angle of its first polygon's
    plane (so a gently curved surface merges into patches, not into one slab)."""
    from .bevel import twins
    from .normal import prim_normals
    nprims = geo.nprims
    pn = normalize_rows(prim_normals(geo).astype(np.float64))
    pv = topo.prim_of_vertex(geo)
    tw = twins(geo)
    he = np.flatnonzero(tw >= 0)
    a, b = pv[he], pv[tw[he]]
    keep = (a < b) & sel[a] & sel[b]
    a, b = a[keep], b[keep]
    dots = np.einsum("ij,ij->i", pn[a], pn[b])
    ok = dots >= cos_tol
    a, b, dots = a[ok], b[ok], dots[ok]
    order = np.argsort(-dots, kind="stable")
    parent = np.arange(nprims)
    ref = pn                                                 # plane normal of each region: its seed polygon's

    def find(x):
        while parent[x] != x:
            parent[x] = parent[parent[x]]
            x = parent[x]
        return x

    for k in order.tolist():
        ra, rb = find(int(a[k])), find(int(b[k]))
        if ra == rb:
            continue
        if float(ref[ra] @ ref[rb]) < cos_tol:
            continue
        lo, hi = (ra, rb) if ra < rb else (rb, ra)
        parent[hi] = lo
    return np.array([find(i) for i in range(nprims)], np.int64)


@node("dissolve", inputs=["geo"], category="modify", params=[
    Str("group", "", doc="Primitives to merge (pattern; empty: all)"),
    Float("angle", 1.0, min=0.0, max=89.0, soft_max=30.0, doc="Largest angle in degrees between neighbouring faces "
                                                              "that still count as one flat region"),
])
def dissolve(geo, group, angle):
    """Merge neighbouring faces that lie in one plane (within the angle) into single n-gons, e.g. the triangles that
    a boolean, a triangulate or a convex hull leave on flat sides. Points stay where they are; the merged face takes
    the attributes of the lowest-numbered face of its region. Regions whose outline would pinch or enclose a hole are
    left as they are."""
    from .boolean import _merge_pieces
    B.require_polygons(geo, "dissolve")
    if geo.nprims < 2:
        return geo
    sel = _prim_sel(geo, group)
    rep = _flat_groups(geo, sel, float(np.cos(np.radians(angle))))
    if np.array_equal(rep, np.arange(geo.nprims)):
        return geo
    t = geo.topology
    sizes = t.prim_sizes().astype(np.int64)
    vp = t.vertex_point.astype(np.int64)
    cidx, msz, mrep = _merge_pieces(vp, sizes, rep, geo.P.astype(np.float64))
    order = np.argsort(mrep, kind="stable")                  # keep the faces in the order of their representatives
    starts = np.cumsum(msz) - msz
    cidx = np.concatenate([cidx[starts[o]:starts[o] + msz[o]] for o in order]) if len(order) else cidx
    msz, mrep = msz[order], mrep[order]
    return B.make(geo.P, vp[cidx], B.offsets_from_sizes(msz), dict(geo.point.items()),
                  {n: arr[cidx] for n, arr in geo.vertex.items()}, B.gather(geo.prim, mrep), geo.detail)


# -- skin -------------------------------------------------------------------------------------------------------
SKIN_OUT = OutGroups(
    OutGroup("surface", "Surface", "skinsurface", "prim", doc="The lofted quads between the curves"),
    OutGroup("cap", "Cap Polygons", "skincap", "prim", doc="The faces closing the first and last curve"),
)


@node("skin", inputs=["geo"], category="modify", params=[
    Int("samples", 0, min=0, soft_max=200, doc="Points per curve after resampling by length (0: the most any curve has)"),
    Bool("loop", False, doc="Join the last curve back to the first (a tube from rings)"),
    Bool("cap", False, doc="Fill the first and last curve with a face when the curves are closed (ignored with loop)"),
    *SKIN_OUT.params(),
])
def skin(geo, samples, loop, cap, **og):
    """Stretch a surface across a row of curves (Houdini's Skin / loft): every curve is resampled to the same number
    of points by arclength and consecutive curves are joined with quads, in primitive order. Closed curves give
    tubes (closed in the other direction too with `loop`). Writes `uv` (u along the curves, v across them)."""
    t = geo.topology
    if t.nprims < 2:
        raise ValueError("skin needs at least two curves (primitives)")
    offs = t.prim_offsets.astype(np.int64)
    P = geo.P.astype(np.float64)
    vp = t.vertex_point.astype(np.int64)
    closed = t.closed_mask()
    if closed.any() and not closed.all():
        raise ValueError("skin: mix of open and closed curves; all curves must be open or all closed")
    sizes = np.diff(offs)
    if (sizes < 2).any():
        raise ValueError("skin: every curve needs at least two points")
    isclosed = bool(closed[0])
    n = int(samples) or int(sizes.max())
    n = max(n, 3 if isclosed else 2)
    rows = []
    for p in range(t.nprims):
        pts = P[vp[offs[p]:offs[p + 1]]]
        if isclosed:
            pts = np.concatenate([pts, pts[:1]])
        seg = np.linalg.norm(np.diff(pts, axis=0), axis=1)
        s = np.concatenate([[0.0], np.cumsum(seg)])
        total = s[-1] if s[-1] > 0 else 1.0
        u = np.arange(n) / n if isclosed else np.linspace(0.0, 1.0, n)
        rows.append(np.stack([np.interp(u * total, s, pts[:, a]) for a in range(3)], 1))
    M = len(rows)
    pos = np.concatenate(rows)
    nu = n if isclosed else n - 1                           # quads along a curve
    nv = M if loop else M - 1                               # quads across
    i, j = np.meshgrid(np.arange(nv), np.arange(nu), indexing="ij")
    i, j = i.ravel(), j.ravel()
    i1, j1 = (i + 1) % M, (j + 1) % n
    quads = np.stack([i * n + j, i * n + j1, i1 * n + j1, i1 * n + j], 1)
    sizes_out = [4] * len(quads)
    verts = [quads.reshape(-1)]
    if cap and isclosed and not loop:
        verts += [np.arange(n)[::-1], (M - 1) * n + np.arange(n)]
        sizes_out += [n, n]
    uv = np.stack([np.tile(np.arange(n) / (n if isclosed else n - 1), M),
                   np.repeat(np.arange(M) / (M if loop else M - 1), n)], 1).astype(np.float32)
    vertex_point = np.concatenate(verts).astype(np.int32)
    out = B.make(pos, vertex_point, B.offsets_from_sizes(sizes_out), {"uv": uv})
    out = SKIN_OUT.write(out, og, "surface", lambda: np.arange(out.nprims) < len(quads))
    return SKIN_OUT.write(out, og, "cap", lambda: np.arange(out.nprims) >= len(quads))


# -- revolve ----------------------------------------------------------------------------------------------------
REVOLVE_OUT = OutGroups(
    OutGroup("endcaps", "End Caps Group", "endcaps", "prim", doc="The faces that close the two ends of a partial revolve"),
)


@node("revolve", inputs=["geo"], category="generate", params=[
    Vec3("origin", (0.0, 0.0, 0.0), doc="A point on the axis"),
    Vec3("axis", (0.0, 0.0, 1.0), doc="Direction of the axis of revolution"),
    Float("angle", 360.0, min=-360.0, max=360.0, soft_min=-360.0, soft_max=360.0,
          doc="Degrees to sweep (360: a closed surface; negative turns the other way)"),
    Int("divisions", 32, min=1, max=4096, soft_max=128, doc="Steps around the axis", proxy="count"),
    Float("start_angle", 0.0, soft_min=-360.0, soft_max=360.0, doc="Where the sweep starts, in degrees (the end is start + angle)"),
    Bool("end_caps", False, doc="Close both ends of a partial revolve with a face per profile (nothing to close at 360 degrees)"),
    *REVOLVE_OUT.params(),
])
def revolve(geo, origin, axis, angle, divisions, start_angle=0.0, end_caps=False, **og):
    """Spin curves (or polygon outlines) around an axis into a surface (Houdini's Revolve): a profile of a vase, a wheel
    or a goblet becomes the solid. Each segment of a profile makes a ring of quads; points on the axis are shared, so a
    profile that runs from the axis to the axis gives a closed shell with triangles at the poles. Writes corner `uv`
    (u around the axis, v along the profile by length); point attributes follow their points, primitive attributes
    their profile."""
    t = geo.topology
    ax = np.asarray(axis, np.float64)
    if not np.linalg.norm(ax):
        raise ValueError("revolve needs a non-zero axis")
    ax = ax / np.linalg.norm(ax)
    o = np.asarray(origin, np.float64)
    offs = t.prim_offsets.astype(np.int64)
    vp = t.vertex_point.astype(np.int64)
    sizes = np.diff(offs)
    closed = t.closed_mask()
    usable = np.flatnonzero(sizes >= 2)
    if not len(usable):
        return B.make(np.zeros((0, 3), np.float32), np.zeros(0, np.int32), np.zeros(1, np.int32))
    full = abs(angle) >= 360.0 - 1e-9
    steps = int(divisions)
    copies = steps if full else steps + 1                            # distinct rotations of every point
    P = geo.P.astype(np.float64)
    rel = P - o
    along = rel @ ax
    radial = rel - np.outer(along, ax)
    on_axis = np.linalg.norm(radial, axis=1) <= 1e-7 * max(float(np.ptp(P, 0).max()) if len(P) else 1.0, 1e-9)
    th = np.radians(start_angle) + np.radians(angle) * np.arange(copies) / steps
    # Rodrigues rotation of every point about the axis for every step: (copies, npoints, 3)
    cos, sin = np.cos(th)[:, None, None], np.sin(th)[:, None, None]
    kx = np.cross(ax, radial)
    rot = radial[None] * cos + kx[None] * sin
    pos = (o + along[:, None] * ax)[None] + rot
    n = geo.npoints
    pos = pos.reshape(copies * n, 3)
    pid = np.arange(copies * n).reshape(copies, n)
    pid[:, on_axis] = pid[0, on_axis]                                # poles are single points
    # profile segments (a -> b along each prim) with their parameter along the profile
    seg_a, seg_b, seg_p, seg_ta, seg_tb = [], [], [], [], []
    for p in usable.tolist():
        v = vp[offs[p]:offs[p + 1]]
        w = np.concatenate([v, v[:1]]) if closed[p] else v
        length = np.linalg.norm(np.diff(P[w], axis=0), axis=1)
        s = np.concatenate([[0.0], np.cumsum(length)])
        s = s / s[-1] if s[-1] > 0 else np.linspace(0.0, 1.0, len(w))
        seg_a.append(w[:-1])
        seg_b.append(w[1:])
        seg_p.append(np.full(len(w) - 1, p))
        seg_ta.append(s[:-1])
        seg_tb.append(s[1:])
    seg_a, seg_b, seg_p = np.concatenate(seg_a), np.concatenate(seg_b), np.concatenate(seg_p)
    seg_ta, seg_tb = np.concatenate(seg_ta), np.concatenate(seg_tb)
    j = np.arange(steps)
    j1 = j + 1
    wrap = j1 % copies if full else j1                               # a full turn closes on the first copy
    # quads (segment, step): (j,a) (j+1,a) (j+1,b) (j,b)
    A0, A1 = pid[j[None, :], seg_a[:, None]], pid[wrap[None, :], seg_a[:, None]]
    B1, B0 = pid[wrap[None, :], seg_b[:, None]], pid[j[None, :], seg_b[:, None]]
    quad = np.stack([A0, A1, B1, B0], -1).reshape(-1, 4)
    u0, u1 = np.tile(j / steps, len(seg_a)), np.tile(j1 / steps, len(seg_a))
    va, vb = np.repeat(seg_ta, steps), np.repeat(seg_tb, steps)
    quad_uv = np.stack([np.stack([u0, va], 1), np.stack([u1, va], 1), np.stack([u1, vb], 1), np.stack([u0, vb], 1)], 1)
    quad_prim = np.repeat(seg_p, steps)
    eq01, eq23 = quad[:, 0] == quad[:, 1], quad[:, 2] == quad[:, 3]
    keep = ~(eq01 & eq23)                                            # both ends on the axis: nothing left
    quad, quad_uv, quad_prim, eq01, eq23 = quad[keep], quad_uv[keep], quad_prim[keep], eq01[keep], eq23[keep]
    pick = np.tile(np.arange(4), (len(quad), 1))
    pick[eq01] = [0, 2, 3, -1]                                       # a on the axis: triangle (j,a) (j+1,b) (j,b)
    pick[eq23] = [0, 1, 2, -1]                                       # b on the axis: triangle (j,a) (j+1,a) (j+1,b)
    rows = np.arange(len(quad))[:, None]
    ok = pick >= 0
    flat_v = quad[rows, np.where(ok, pick, 0)][ok]
    flat_uv = quad_uv[rows, np.where(ok, pick, 0)][ok]
    sizes_out = ok.sum(1)
    if angle < 0:                                                    # turning the other way: keep the faces facing out
        starts = np.cumsum(sizes_out) - sizes_out
        order = np.concatenate([starts[i] + np.arange(sizes_out[i])[::-1] for i in range(len(sizes_out))]) \
            if len(sizes_out) else np.zeros(0, np.int64)
        flat_v, flat_uv = flat_v[order], flat_uv[order]
    point_src = np.tile(np.arange(n), copies)
    point = B.gather(geo.point, point_src)
    if "N" in geo.point and geo.point["N"].ndim == 2 and geo.point["N"].shape[1] == 3:   # normals turn with their points
        N = geo.point["N"].astype(np.float64)
        Nal = (N @ ax)[:, None] * ax
        Nrad = N - Nal
        Nrot = Nal[None] + Nrad[None] * cos + np.cross(ax, Nrad)[None] * sin
        point["N"] = Nrot.reshape(-1, 3).astype(np.float32)
    prim_src = quad_prim
    ncap = 0
    if end_caps and not full:
        cap_v, cap_sizes, cap_prim = [], [], []
        for p in usable.tolist():
            v = vp[offs[p]:offs[p + 1]]
            if len(v) < 3:
                continue
            # the side quads of segment a -> b run (j,b) -> (j,a) along the start ring and (j+1,a) -> (j+1,b) along the end ring
            # (reversed for a negative angle), so the start cap must walk the profile forwards and the end cap backwards
            for copy, forwards in ((0, angle >= 0), (steps, angle < 0)):
                ids = pid[copy, v]
                cap_v.append(ids if forwards else ids[::-1])
                cap_sizes.append(len(ids))
                cap_prim.append(p)
        if cap_v:
            flat_v = np.concatenate([flat_v] + cap_v)
            flat_uv = np.concatenate([flat_uv, np.zeros((sum(cap_sizes), 2))])
            sizes_out = np.concatenate([sizes_out, cap_sizes])
            prim_src = np.concatenate([quad_prim, cap_prim])
            ncap = len(cap_sizes)
    nbody = len(sizes_out) - ncap
    out = B.make(pos, flat_v.astype(np.int32), B.offsets_from_sizes(sizes_out), point, {"uv": flat_uv.astype(np.float32)},
                 B.gather(geo.prim, prim_src), geo.detail)
    out = REVOLVE_OUT.write(out, og, "endcaps", lambda: np.arange(out.nprims) >= nbody)
    return compact_points(out)


# -- bridge -----------------------------------------------------------------------------------------------------
def _bridge_pair(La, Lb, P, align, twist):
    """Faces joining boundary loop La to Lb (both in polyfill's fill order). Equal lengths give quads, otherwise a strip of
    triangles that always advances along whichever loop keeps the next rung shorter."""
    Lb = Lb[::-1]                                         # the two loops run opposite ways around a tube
    na, nb = len(La), len(Lb)
    s = 0
    if align and na == nb:
        best = None
        for k in range(nb):
            d = float(((P[La] - P[np.roll(Lb, -k)]) ** 2).sum())
            if best is None or d < best[0] - 1e-12:
                best = (d, k)
        s = best[1]
    s = (s + twist) % nb
    Lb = np.roll(Lb, -s)
    faces = []
    if na == nb:
        for i in range(na):
            j = (i + 1) % na
            faces.append([La[i], La[j], Lb[j], Lb[i]])
        # Lb was reversed: (a_i, a_i+1, b_i+1, b_i) is the fill-order quad (a_i, a_i+1, B_k, B_k+1)
        return faces
    i = j = 0
    while i < na or j < nb:
        a0, a1 = La[i % na], La[(i + 1) % na]
        b0, b1 = Lb[j % nb], Lb[(j + 1) % nb]
        adv_a = i < na and (j >= nb or np.linalg.norm(P[a1] - P[b0]) <= np.linalg.norm(P[a0] - P[b1]))
        if adv_a:
            faces.append([a0, a1, b0])
            i += 1
        else:
            faces.append([a0, b1, b0])
            j += 1
    return faces


BRIDGE_OUT = OutGroups(
    OutGroup("bridge", "Bridge Polygons", "bridgepolys", "prim", doc="The faces that join the paired loops"),
)


@node("bridge", inputs=["geo"], category="modify", params=[
    Bool("align", True, doc="Rotate the second loop to the position that keeps the rungs shortest (equal-length loops)"),
    Int("twist", 0, soft_min=-8, soft_max=8, doc="Extra steps to turn the second loop"),
    Enum("pairing", ["mutual_nearest", "in_order"], doc="mutual_nearest: join two open loops of different pieces when each is "
                                                        "the other's nearest (centres); in_order: the first two, the next "
                                                        "two... (a lone loop in a single piece pairs with the next nearest)"),
    *BRIDGE_OUT.params(),
])
def bridge(geo, align, twist, pairing, **og):
    """Join pairs of open boundary loops with a band of faces (Houdini's Bridge): two pipe ends become one pipe, a hole
    in a wall and a hole in a floor a corridor. Equal loops get quads; loops of different sizes get triangles. Loops are
    paired when each is the other's nearest loop among those of other pieces (or in order); a loop with no partner is
    left open."""
    from .model import boundary_loops
    B.require_polygons(geo, "bridge")
    loops = [np.asarray(l, np.int64) for l in boundary_loops(geo)]
    if len(loops) < 2:
        raise ValueError(f"bridge needs at least two open loops, the input has {len(loops)}")
    P = geo.P.astype(np.float64)
    cent = np.array([P[l].mean(0) for l in loops])
    pairs = []
    if pairing == "in_order":
        pairs = [(i, i + 1) for i in range(0, len(loops) - 1, 2)]
    else:
        from .. import spatial, topo
        e = topo.edges(geo)
        comp = spatial.components(geo.npoints, e.points[:, 0], e.points[:, 1])
        piece = np.array([comp[l[0]] for l in loops])
        dist = np.linalg.norm(cent[:, None] - cent[None], axis=2)
        if len(set(piece.tolist())) > 1:
            dist = np.where(piece[:, None] == piece[None], np.inf, dist)      # never close a pipe on itself
        else:
            np.fill_diagonal(dist, np.inf)                                    # one piece: its own loops may be joined
        nearest = dist.argmin(1)
        for i in range(len(loops)):
            j = int(nearest[i])
            if i < j and np.isfinite(dist[i, j]) and int(nearest[j]) == i:
                pairs.append((i, j))
        if not pairs:
            raise ValueError("bridge found no two loops that are each other's nearest; wire pieces with matching ends")
    new = []
    for a, b in pairs:
        new += _bridge_pair(loops[a], loops[b], P, align, int(twist))
    sizes = np.array([len(f) for f in new], np.int64)
    t = geo.topology
    verts = np.concatenate([np.asarray(f, np.int64) for f in new])
    offs = np.concatenate([t.prim_offsets.astype(np.int64), t.prim_offsets[-1] + np.cumsum(sizes)])
    pad = lambda table, k: {n: np.concatenate([a, np.zeros((k,) + a.shape[1:], a.dtype)]) for n, a in table.items()}  # noqa: E731
    out = B.make(geo.P, np.concatenate([t.vertex_point, verts]), offs, dict(geo.point.items()),
                 pad(geo.vertex, len(verts)), pad(geo.prim, len(new)), geo.detail)
    return BRIDGE_OUT.write(out, og, "bridge", lambda: np.arange(out.nprims) >= t.nprims)
