"""Polygon modelling: reverse, facet, divide, mirror, clip, polyfill, polyextrude (Houdini's SOPs of the same names).

Everything is vectorised over all primitives at once (the only Python loops are ear clipping of *concave* n-gons in
``divide`` and walking hole boundaries in ``polyfill``). Attributes follow the rules in :mod:`._build`.
"""
import numpy as np

from .. import groups, topo
from ..outgroups import OutGroup, OutGroups
from ..sdk import Bool, Enum, Float, Guide, Handle, Str, Vec3, node
from . import _build as B
from .normal import prim_normals
from ._util import normalize_rows


def _prim_sel(geo, pattern):
    m = groups.mask(geo, "prim", pattern)
    return np.ones(geo.nprims, bool) if m is None else m


def compact_points(geo):
    """Drop points no primitive uses (keeps order)."""
    used = np.zeros(geo.npoints, bool)
    used[geo.topology.vertex_point] = True
    if used.all():
        return geo
    remap = (np.cumsum(used) - 1).astype(np.int32)
    return B.make(geo.P[used], remap[geo.topology.vertex_point], geo.topology.prim_offsets,
                  B.gather(geo.point, np.flatnonzero(used)), dict(geo.vertex.items()), dict(geo.prim.items()), geo.detail,
                  geo.topology.closed)


# -- reverse ----------------------------------------------------------------------------------------------------
@node("reverse", inputs=["geo"], category="modify", params=[
    Str("group", "", doc="Primitives to reverse (pattern; empty: all)"),
    Bool("reverse_normals", True, doc="Also negate N on the reversed primitives (and their points and vertices)"),
])
def reverse(geo, group, reverse_normals):
    """Reverse the winding of primitives, which flips the side they face (Houdini's Reverse)."""
    sel = _prim_sel(geo, group)
    t = geo.topology
    offs = t.prim_offsets.astype(np.int64)
    pv = topo.prim_of_vertex(geo)
    v = np.arange(t.nvertices)
    order = np.where(sel[pv], offs[pv] + offs[pv + 1] - 1 - v, v)
    vert = B.gather(geo.vertex, order)
    point, prim = dict(geo.point.items()), dict(geo.prim.items())
    if reverse_normals:
        vsel = sel[pv][order]
        if "N" in vert:
            vert["N"] = np.where(vsel[:, None], -vert["N"], vert["N"])
        if "N" in prim:
            prim["N"] = np.where(sel[:, None], -prim["N"], prim["N"])
        if "N" in point:
            psel = groups.convert(geo, sel, "prim", "point")
            point["N"] = np.where(psel[:, None], -point["N"], point["N"])
    return B.make(geo.P, t.vertex_point[order], t.prim_offsets, point, vert, prim, geo.detail, t.closed)


# -- facet ------------------------------------------------------------------------------------------------------
@node("facet", inputs=["geo"], category="modify", params=[
    Bool("unique_points", True, doc="Give every vertex its own point (faces no longer share points)"),
    Bool("compute_normals", False, doc="Write N per point from its face (flat); otherwise N is left as it is"),
])
def facet(geo, unique_points, compute_normals):
    """Split shared points so each face stands alone (Houdini's Facet 'Unique Points'). Useful before moving faces
    apart; for flat shading in Blender, sharp_face (a primitive int) is cheaper."""
    out = geo
    if unique_points:
        vp = geo.topology.vertex_point
        out = B.make(geo.P[vp], np.arange(len(vp)), geo.topology.prim_offsets, B.gather(geo.point, vp),
                     dict(geo.vertex.items()), dict(geo.prim.items()), geo.detail, geo.topology.closed)
    if compute_normals:
        out = out.copy() if out is geo else out
        fn = normalize_rows(prim_normals(out))
        if unique_points:
            pn = np.zeros((out.npoints, 3), np.float32)
            pn[out.topology.vertex_point] = fn[topo.prim_of_vertex(out)]
            out.point["N"] = pn
        else:
            out.prim["N"] = fn
    return out


# -- divide (triangulate) ---------------------------------------------------------------------------------------
def _ear_clip(pts2d):
    """Triangulate a simple polygon (2-D, any winding) -> list of index triples into it."""
    n = len(pts2d)
    idx = list(range(n))
    area = 0.5 * sum(pts2d[i][0] * pts2d[(i + 1) % n][1] - pts2d[(i + 1) % n][0] * pts2d[i][1] for i in range(n))
    sign = 1.0 if area >= 0 else -1.0
    tris = []

    def cross(a, b, c):
        return ((b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])) * sign

    guard = 0
    while len(idx) > 3 and guard < 10 * n:
        guard += 1
        m = len(idx)
        for k in range(m):
            i0, i1, i2 = idx[k - 1], idx[k], idx[(k + 1) % m]
            a, b, c = pts2d[i0], pts2d[i1], pts2d[i2]
            if cross(a, b, c) <= 1e-12:
                continue                                    # reflex or degenerate corner: not an ear
            inside = False
            for j in idx:
                if j in (i0, i1, i2):
                    continue
                p = pts2d[j]
                if cross(a, b, p) >= 0 and cross(b, c, p) >= 0 and cross(c, a, p) >= 0:
                    inside = True
                    break
            if not inside:
                tris.append((i0, i1, i2))
                del idx[k]
                break
        else:                                               # no ear found (self-intersecting): fan the rest
            break
    for k in range(1, len(idx) - 1):
        tris.append((idx[0], idx[k], idx[k + 1]))
    return tris


def triangulate(geo, sel):
    """(triangle vertex triples (T, 3) into geo's vertices, owning primitive (T,)) for selected primitives; others are
    returned whole as (vertex lists). Convex polygons fan; concave ones are ear-clipped."""
    t = geo.topology
    offs = t.prim_offsets.astype(np.int64)
    sizes = np.diff(offs)
    pv = topo.prim_of_vertex(geo)
    P = geo.P.astype(np.float64)
    nxt, prv = topo.next_vertex(geo), topo.prev_vertex(geo)
    pos = P[t.vertex_point]
    fn = prim_normals(geo)
    turn = np.einsum("ij,ij->i", np.cross(pos - pos[prv], pos[nxt] - pos), fn[pv])
    concave = np.bincount(pv, weights=(turn < -1e-12), minlength=t.nprims) > 0
    work = sel & (sizes > 3) & t.closed_mask()             # open curves are left as they are
    fan = work & ~concave
    counts = np.where(fan, sizes - 2, 0)
    first = np.repeat(offs[:-1], counts)
    k = np.arange(counts.sum()) - np.repeat(np.cumsum(counts) - counts, counts)
    tri_v = np.stack([first, first + k + 1, first + k + 2], 1)
    tri_p = np.repeat(np.arange(t.nprims), counts)
    extra_v, extra_p = [], []
    for p in np.flatnonzero(work & concave):
        verts = np.arange(offs[p], offs[p + 1])
        n = fn[p] / max(np.linalg.norm(fn[p]), 1e-30)
        u = np.cross(n, [1.0, 0, 0] if abs(n[0]) < 0.9 else [0, 1.0, 0])
        u /= np.linalg.norm(u)
        w = np.cross(n, u)
        q = pos[verts]
        pts = list(zip((q @ u).tolist(), (q @ w).tolist()))
        for a, b, c in _ear_clip(pts):
            extra_v.append((verts[a], verts[b], verts[c]))
            extra_p.append(p)
    if extra_v:
        tri_v = np.concatenate([tri_v, np.asarray(extra_v, np.int64)])
        tri_p = np.concatenate([tri_p, np.asarray(extra_p, np.int64)])
    return tri_v, tri_p, ~work


@node("divide", inputs=["geo"], category="modify", params=[
    Str("group", "", doc="Primitives to triangulate (pattern; empty: all)"),
])
def divide(geo, group):
    """Triangulate polygons (Houdini's Divide 'Triangulate'): convex ones as fans, concave ones by ear clipping.
    Vertex and primitive attributes follow; primitive order is kept."""
    sel = _prim_sel(geo, group)
    tri_v, tri_p, keep = triangulate(geo, sel)
    t = geo.topology
    offs = t.prim_offsets.astype(np.int64)
    sizes = np.diff(offs)
    # Keep untouched prims whole; order pieces by owning prim so the primitive order is stable
    whole_p = np.flatnonzero(keep)
    whole_v = np.flatnonzero(keep[topo.prim_of_vertex(geo)])
    whole_sizes = sizes[whole_p]
    owner = np.concatenate([whole_p, tri_p])
    src = np.concatenate([whole_v, tri_v.reshape(-1)])
    starts = np.concatenate([np.cumsum(whole_sizes) - whole_sizes, len(whole_v) + 3 * np.arange(len(tri_p))])
    all_sizes = np.concatenate([whole_sizes, np.full(len(tri_p), 3)])
    order = np.argsort(owner, kind="stable")
    piece_sizes, piece_starts = all_sizes[order], starts[order]
    local = np.arange(piece_sizes.sum()) - np.repeat(np.cumsum(piece_sizes) - piece_sizes, piece_sizes)
    verts = src[np.repeat(piece_starts, piece_sizes) + local]
    closed = None
    if t.has_open:                                          # whole prims keep their flag; triangles are closed
        closed = np.concatenate([t.closed[whole_p], np.ones(len(tri_p), bool)])[order]
    return B.make(geo.P, t.vertex_point[verts], B.offsets_from_sizes(piece_sizes), dict(geo.point.items()),
                  B.gather(geo.vertex, verts), B.gather(geo.prim, owner[order]), geo.detail, closed)


# -- mirror -----------------------------------------------------------------------------------------------------
MIRROR_OUT = OutGroups(
    OutGroup("mirrored", "Mirrored Primitives", "mirrored", "prim", doc="The reflected copies of the primitives"),
    OutGroup("mirroredpoints", "Mirrored Points", "mirroredpoints", "point", doc="The reflected copies of the points"),
    OutGroup("seam", "Seam Points", "seampoints", "point", doc="Points on the mirror plane that were welded to their reflections"),
)


@node("mirror", inputs=["geo"], category="modify", params=[
    Vec3("origin", (0.0, 0.0, 0.0)),
    Vec3("normal", (1.0, 0.0, 0.0), doc="Normal of the mirror plane (default: mirror across X = 0)"),
    Bool("keep_original", True),
    Bool("consolidate_seam", True, doc="Weld mirrored points that lie on the plane to the originals"),
    Float("distance", 0.001, min=0.0, soft_max=0.1, doc="How close to the plane a point must be to weld"),
    *MIRROR_OUT.params(),
], handles=[Handle("origin", axis="x", at="@origin", comp=0), Handle("origin", axis="y", at="@origin", comp=1), Handle("origin", axis="z", at="@origin", comp=2)], guides=[Guide("plane", at="@origin", axis="@normal", id="plane", label="mirror plane")])

def mirror(geo, origin, normal, keep_original, consolidate_seam, distance, **og):
    """Reflect geometry across a plane (Houdini's Mirror). The copy's winding is reversed so it faces outwards, and N
    is reflected."""
    n = np.asarray(normal, np.float64)
    n = n / max(np.linalg.norm(n), 1e-12)
    o = np.asarray(origin, np.float64)
    P = geo.P.astype(np.float64)
    d = (P - o) @ n
    Pm = (P - 2 * d[:, None] * n).astype(np.float32)
    t = geo.topology
    offs = t.prim_offsets.astype(np.int64)
    pv = topo.prim_of_vertex(geo)
    v = np.arange(t.nvertices)
    rev = offs[pv] + offs[pv + 1] - 1 - v                  # reversed winding for the copy

    def reflect(table):
        out = dict(table.items())
        if "N" in out and out["N"].ndim == 2 and out["N"].shape[1] == 3:
            N = out["N"].astype(np.float64)
            out["N"] = (N - 2 * (N @ n)[:, None] * n).astype(np.float32)
        return out

    m_point = reflect(geo.point)
    m_point["P"] = Pm
    m_vert = {k: a[rev] for k, a in reflect(geo.vertex).items()}
    m_prim = reflect(geo.prim)
    if not keep_original:
        out = B.make(Pm, t.vertex_point[rev], t.prim_offsets, m_point, m_vert, m_prim, geo.detail, t.closed)
        out = MIRROR_OUT.write(out, og, "mirrored", np.ones(out.nprims, bool))
        out = MIRROR_OUT.write(out, og, "mirroredpoints", np.ones(out.npoints, bool))
        return MIRROR_OUT.write(out, og, "seam", np.zeros(out.npoints, bool))
    np_ = geo.npoints
    point_map = np.arange(np_) + np_
    if consolidate_seam:
        on = np.abs(d) <= distance
        point_map[on] = np.flatnonzero(on)
    copy_vp = point_map[t.vertex_point[rev]]
    both = B.make(np.concatenate([geo.P, Pm]), np.concatenate([t.vertex_point, copy_vp]),
                  np.concatenate([offs, offs[1:] + offs[-1]]),
                  B.concat([dict(geo.point.items()), m_point]), B.concat([dict(geo.vertex.items()), m_vert]),
                  B.concat([dict(geo.prim.items()), m_prim]), geo.detail,
                  None if t.closed is None else np.concatenate([t.closed, t.closed]))
    seam = np.zeros(2 * np_, bool)
    if consolidate_seam:
        seam[:np_] = np.abs(d) <= distance
    both = MIRROR_OUT.write(both, og, "mirrored", lambda: np.arange(both.nprims) >= t.nprims)
    both = MIRROR_OUT.write(both, og, "mirroredpoints", lambda: (np.arange(both.npoints) >= np_) | seam)
    both = MIRROR_OUT.write(both, og, "seam", seam)
    return compact_points(both)


# -- clip -------------------------------------------------------------------------------------------------------
CLIP_OUT = OutGroups(
    OutGroup("above", "Above", "above", "prim", default=True, doc="Faces on the side the normal points to"),
    OutGroup("below", "Below", "below", "prim", doc="Faces on the other side"),
    OutGroup("cutpolys", "Cut Polygons", "cutpolys", "prim", doc="Faces the plane went through (trimmed pieces of them)"),
    OutGroup("cutpoints", "Cut Points", "cutpoints", "point", doc="The new points on the plane, and original points that lie on it"),
    OutGroup("cutedges", "Cut Edges", "cutedges", "edge", doc="The edges that lie in the plane: the opening a cap would fill"),
)


@node("clip", inputs=["geo"], category="modify", params=[
    Vec3("origin", (0.0, 0.0, 0.0)),
    Vec3("normal", (0.0, 0.0, 1.0), doc="Plane normal; 'above' is the side it points to"),
    Enum("keep", ["above", "below", "both"], doc="both: cut along the plane and keep both halves (sharing the seam)"),
    *CLIP_OUT.params(),
], handles=[Handle("origin", axis="x", at="@origin", comp=0), Handle("origin", axis="y", at="@origin", comp=1), Handle("origin", axis="z", at="@origin", comp=2)], guides=[Guide("plane", at="@origin", axis="@normal", id="plane", label="clip plane")])

def clip(geo, origin, normal, keep, **og):
    """Cut polygons with a plane (Houdini's Clip). Cut edges get new points with interpolated attributes; faces
    straddling the plane are trimmed, not deleted. Use polyfill to cap the opening."""
    B.require_polygons(geo, "clip")
    n = np.asarray(normal, np.float64)
    n = n / max(np.linalg.norm(n), 1e-12)
    d = (geo.P.astype(np.float64) - np.asarray(origin, np.float64)) @ n
    eps = 1e-7 * max(1.0, float(np.abs(d).max()) if len(d) else 1.0)
    d = np.where(np.abs(d) < eps, 0.0, d)
    t = geo.topology
    vp = t.vertex_point.astype(np.int64)
    nxt = topo.next_vertex(geo)
    pv = topo.prim_of_vertex(geo)
    dv, dn = d[vp], d[vp[nxt]]
    crosses = dv * dn < 0
    e = topo.edges(geo)
    cut_edges = np.unique(e.of_vertex[crosses])
    x_index = np.full(len(e.points), -1, np.int64)
    x_index[cut_edges] = np.arange(len(cut_edges)) + geo.npoints
    lo, hi = e.points[cut_edges, 0], e.points[cut_edges, 1]
    t_edge = d[lo] / (d[lo] - d[hi])                       # canonical direction: shared by both faces of the edge
    new_pts = B.lerp(geo.point, lo, hi, t_edge)
    new_pts["P"] = (geo.P[lo] + (geo.P[hi] - geo.P[lo]) * t_edge[:, None].astype(np.float32)).astype(np.float32)
    t_local = np.where(crosses, dv / np.where(crosses, dv - dn, 1.0), 0.0)
    sides = {"above": [1.0], "below": [-1.0], "both": [1.0, -1.0]}[keep]
    ents_v, ents_x, ents_p = [], [], []
    for s in sides:
        emit_v = s * dv >= 0
        slots_v = np.stack([np.where(emit_v, np.arange(len(vp)), -1), np.where(crosses, np.arange(len(vp)), -1)], 1)
        kind = np.tile([0, 1], len(vp))
        flat = slots_v.reshape(-1)
        valid = flat >= 0
        verts, kinds = flat[valid], kind[valid]
        prim = pv[verts]
        size = np.bincount(prim, minlength=t.nprims)
        ok = size[prim] >= 3
        ents_v.append(verts[ok])
        ents_x.append(kinds[ok])
        ents_p.append(prim[ok] + (0 if s > 0 else t.nprims))   # keeps 'above' pieces before 'below' ones
    verts, kinds, prim_key = np.concatenate(ents_v), np.concatenate(ents_x), np.concatenate(ents_p)
    order = np.argsort(prim_key, kind="stable")
    verts, kinds, prim_key = verts[order], kinds[order], prim_key[order]
    point_idx = np.where(kinds == 0, vp[verts], x_index[e.of_vertex[verts]])
    vx = B.lerp(geo.vertex, verts, nxt[verts], np.where(kinds == 1, t_local[verts], 0.0))
    prims_out, sizes_out = np.unique(prim_key, return_counts=True)
    src_prim = prims_out % max(t.nprims, 1)
    pts = B.concat([dict(geo.point.items()), new_pts])
    out = B.make(pts["P"], point_idx, B.offsets_from_sizes(sizes_out), pts, vx, B.gather(geo.prim, src_prim), geo.detail)
    out = CLIP_OUT.write(out, og, "above", lambda: prims_out < t.nprims)
    out = CLIP_OUT.write(out, og, "below", lambda: prims_out >= t.nprims)
    was_cut = np.zeros(t.nprims, bool)
    was_cut[pv[crosses]] = True
    out = CLIP_OUT.write(out, og, "cutpolys", lambda: was_cut[src_prim])
    on_plane = np.concatenate([d == 0.0, np.ones(len(cut_edges), bool)])         # original points on the plane, then the new ones
    out = CLIP_OUT.write(out, og, "cutpoints", on_plane)
    used = np.zeros(out.npoints, bool)
    used[out.topology.vertex_point] = True
    out = compact_points(out)
    if CLIP_OUT.enabled(og, "cutedges"):                     # an edge of the result with both ends on the plane lies in it
        ends = on_plane[used][topo.edges(out).points]
        out = CLIP_OUT.write(out, og, "cutedges", ends.all(axis=1))
    return out


# -- polyfill ---------------------------------------------------------------------------------------------------
def boundary_loops(geo):
    """Closed loops of boundary edges, each as point indices ordered to *fill* the hole (facing the same way as the
    surrounding faces). Raises if a point has several boundary edges leaving it (a non-manifold boundary)."""
    e = topo.edges(geo)
    vp = geo.topology.vertex_point.astype(np.int64)
    nxt = topo.next_vertex(geo)
    on = e.count[e.of_vertex] == 1
    a, b = vp[on], vp[nxt[on]]
    succ = np.full(geo.npoints, -1, np.int64)
    if len(np.unique(b)) != len(b):
        raise ValueError("the boundary touches itself (a point with several open edges); cannot fill it")
    succ[b] = a                                           # the fill face walks the boundary backwards
    loops = []
    seen = np.zeros(geo.npoints, bool)
    for start in b.tolist():
        if seen[start]:
            continue
        loop, p = [], start
        while not seen[p] and p >= 0:
            seen[p] = True
            loop.append(p)
            p = int(succ[p])
        if p == start and len(loop) >= 3:
            loops.append(loop)
    return loops


@node("polyfill", inputs=["geo"], category="modify", params=[
    Str("fill_group", "", doc="Optional name of a primitive group for the new faces"),
])
def polyfill(geo, fill_group):
    """Close every hole with one n-gon facing the same way as its surroundings (Houdini's PolyFill, single polygon).
    New faces get zero for primitive and vertex attributes."""
    B.require_polygons(geo, "polyfill")
    loops = boundary_loops(geo)
    if not loops:
        return geo
    t = geo.topology
    sizes = np.array([len(l) for l in loops])
    new_vp = np.concatenate([np.asarray(l, np.int64) for l in loops])
    offs = np.concatenate([t.prim_offsets.astype(np.int64), t.prim_offsets[-1] + np.cumsum(sizes)])
    pad = lambda table, k: {n: np.concatenate([a, np.zeros((k,) + a.shape[1:], a.dtype)]) for n, a in table.items()}  # noqa: E731
    out = B.make(geo.P, np.concatenate([t.vertex_point, new_vp]), offs, dict(geo.point.items()),
                 pad(geo.vertex, len(new_vp)), pad(geo.prim, len(loops)), geo.detail)
    if fill_group:
        out = groups.write(out, "prim", fill_group, np.r_[np.zeros(t.nprims, bool), np.ones(len(loops), bool)])
    return out


# -- polyextrude ------------------------------------------------------------------------------------------------
# PolyExtrude's Output Groups: Front and Side were always made (those names), Back only exists when the originals are kept
EXTRUDE_OUT = OutGroups(
    OutGroup("front", "Front", "extrudeFront", "prim", default=True, doc="The extruded faces (the selected faces, moved)"),
    OutGroup("back", "Back", "extrudeBack", "prim", doc="The original faces left behind (needs 'output_back')"),
    OutGroup("side", "Side", "extrudeSide", "prim", default=True, doc="The walls between the original outline and the front faces"),
)


@node("polyextrude", inputs=["geo"], category="modify", params=[
    Str("group", "", doc="Primitives to extrude (pattern; empty: all)"),
    Float("distance", 0.5, soft_min=-2.0, soft_max=2.0, doc="Along the face normals (negative: inwards)"),
    Float("inset", 0.0, soft_min=0.0, soft_max=1.0, doc="Move the front edges in towards each face's centre"),
    Enum("mode", ["connected", "individual"], doc="connected: neighbouring faces extrude as one region (Houdini's "
                                                  "'keep shared edges together'); individual: every face on its own"),
    Bool("output_back", False, doc="Keep the original faces as well (a closed shell)"),
    *EXTRUDE_OUT.params(),
])
def polyextrude(geo, group, distance, inset, mode, output_back, **og):
    """Extrude faces along their normals with optional inset (Houdini's PolyExtrude). Side walls are added on the
    region's boundary; groups extrudeFront and extrudeSide hold the new faces."""
    B.require_polygons(geo, "polyextrude")
    sel = _prim_sel(geo, group)
    if not sel.any():
        for key in ("front", "back", "side"):                      # the switched-on groups still exist, empty
            geo = EXTRUDE_OUT.write(geo, og, key, np.zeros(geo.nprims, bool))
        return geo
    t = geo.topology
    vp = t.vertex_point.astype(np.int64)
    pv = topo.prim_of_vertex(geo)
    nxt = topo.next_vertex(geo)
    vsel = sel[pv]
    fn = prim_normals(geo)
    fn_unit = normalize_rows(fn).astype(np.float64)
    P = geo.P.astype(np.float64)
    np0 = geo.npoints
    if mode == "individual":
        sv = np.flatnonzero(vsel)                          # one new point per selected vertex
        new_of_vertex = np.full(t.nvertices, -1, np.int64)
        new_of_vertex[sv] = np0 + np.arange(len(sv))
        src_point = vp[sv]
        offset = fn_unit[pv[sv]] * distance
        cen = topo.prim_centroids(geo)
        base = P[src_point]
        if inset:
            to_c = cen[pv[sv]] - base
            dist = np.linalg.norm(to_c, axis=1)
            base = base + to_c * (np.minimum(inset / np.maximum(dist, 1e-12), 1.0))[:, None]
        newP = base + offset
        boundary_v = sv                                    # every edge of every face gets a wall
    else:
        used = np.zeros(np0, bool)
        used[vp[vsel]] = True
        src_point = np.flatnonzero(used)
        remap = np.full(np0, -1, np.int64)
        remap[src_point] = np0 + np.arange(len(src_point))
        new_of_vertex = np.where(vsel, remap[vp], -1)
        acc = topo.segment_sum(fn[pv[vsel]], vp[vsel], np0)[src_point]
        pn = normalize_rows(acc).astype(np.float64)
        # scale so flat-ish corners keep the requested thickness (bounded for sharp ones)
        cosang = np.einsum("ij,ij->i", pn[np.searchsorted(src_point, vp[vsel])], fn_unit[pv[vsel]])
        mincos = np.ones(len(src_point))
        np.minimum.at(mincos, np.searchsorted(src_point, vp[vsel]), np.maximum(cosang, 0.25))
        newP = P[src_point] + pn * (distance / mincos)[:, None]
        if inset:
            e = topo.edges(geo)
            cnt = np.bincount(e.of_vertex[vsel], minlength=len(e.points))
            bnd_pt = np.zeros(np0, bool)
            bnd_pt[e.points[cnt == 1].ravel()] = True
            cen = topo.prim_centroids(geo)
            to_c = topo.segment_sum(cen[pv[vsel]] - P[vp[vsel]], vp[vsel], np0)[src_point]
            k = np.maximum(np.bincount(vp[vsel], minlength=np0)[src_point], 1)[:, None]
            to_c = to_c / k
            dist = np.linalg.norm(to_c, axis=1)
            move = to_c * np.minimum(inset / np.maximum(dist, 1e-12), 1.0)[:, None]
            newP = newP + np.where(bnd_pt[src_point][:, None], move, 0.0)
        e = topo.edges(geo)
        cnt = np.bincount(e.of_vertex[vsel], minlength=len(e.points))
        boundary_v = np.flatnonzero(vsel & (cnt[e.of_vertex] == 1))
    # Front faces: the selected prims on the new points (original prims keep their attributes)
    front_vp = np.where(vsel, new_of_vertex, vp)
    # Side walls for boundary half-edges a->b: quad (a, b, b', a')
    a, b = vp[boundary_v], vp[nxt[boundary_v]]
    a2, b2 = new_of_vertex[boundary_v], new_of_vertex[nxt[boundary_v]]
    side_vp = np.stack([a, b, b2, a2], 1).reshape(-1)
    side_src_vertex = np.stack([boundary_v, nxt[boundary_v], nxt[boundary_v], boundary_v], 1).reshape(-1)
    side_prim = pv[boundary_v]
    nside = len(boundary_v)
    # Optional back faces: the original selection, reversed, on the original points
    offs = t.prim_offsets.astype(np.int64)
    parts_vp, parts_vsrc, parts_sizes, parts_prim = [front_vp], [np.arange(t.nvertices)], [t.prim_sizes()], [np.arange(t.nprims)]
    parts_vp.append(side_vp)
    parts_vsrc.append(side_src_vertex)
    parts_sizes.append(np.full(nside, 4))
    parts_prim.append(side_prim)
    nback = 0
    if output_back:
        bp = np.flatnonzero(sel)
        bsz = t.prim_sizes()[bp].astype(np.int64)
        local = np.arange(bsz.sum()) - np.repeat(np.cumsum(bsz) - bsz, bsz)
        bv = np.repeat(offs[bp + 1] - 1, bsz) - local          # each face's vertices, reversed
        parts_vp.append(vp[bv])
        parts_vsrc.append(bv)
        parts_sizes.append(t.prim_sizes()[bp])
        parts_prim.append(bp)
        nback = len(bp)
    points = B.concat([dict(geo.point.items()), B.gather(geo.point, src_point)])
    points["P"] = np.concatenate([geo.P, newP.astype(np.float32)])
    out = B.make(points["P"], np.concatenate(parts_vp), B.offsets_from_sizes(np.concatenate(parts_sizes)), points,
                 B.gather(geo.vertex, np.concatenate(parts_vsrc)), B.gather(geo.prim, np.concatenate(parts_prim)),
                 geo.detail)
    total = t.nprims + nside + nback
    front = np.zeros(total, bool)
    front[:t.nprims] = sel
    side = np.zeros(total, bool)
    side[t.nprims:t.nprims + nside] = True
    back = np.zeros(total, bool)
    back[t.nprims + nside:] = True                           # the kept originals, last
    out = EXTRUDE_OUT.write(out, og, "front", front)
    out = EXTRUDE_OUT.write(out, og, "back", back)
    out = EXTRUDE_OUT.write(out, og, "side", side)
    if "N" in out.point:                                   # stale after moving points; recompute with a normal node
        del out.point["N"]
    return compact_points(out)
