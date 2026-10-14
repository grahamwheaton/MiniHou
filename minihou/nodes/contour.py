"""Contour and slice: iso-lines and cut surfaces of a point attribute on a mesh (marching squares / triangles), and slices by planes."""
import numpy as np

from .. import groups
from ..outgroups import OutGroup, OutGroups
from ..sdk import Bool, Enum, Float, Guide, Handle, Int, Str, Vec3, node
from . import _build as B
from ._util import fan_triangulate, guard_count
from .curves import _curves_geometry, chain_segments


# -- shared machinery -------------------------------------------------------------------------------------------
def prepare_cells(geo, cells):
    """Polygons the contouring works on: triangles and quads (every polygon with more corners, or all with 'triangles', is
    fan-triangulated). Returns a Geometry of polygons only; vertex, primitive and point attributes are kept."""
    t = geo.topology
    sizes = t.prim_sizes()
    poly = (sizes >= 3) & t.closed_mask()
    limit = 3 if cells == "triangles" else 4
    split = poly & (sizes > limit)
    if not split.any() and poly.all():
        return geo
    keep = poly & ~split
    offs = t.prim_offsets.astype(np.int64)
    tri_rows, tri_prim = fan_triangulate(t)
    pick = split[tri_prim]
    tri_rows, tri_prim = tri_rows[pick], tri_prim[pick]
    kept_prims = np.flatnonzero(keep)
    kept_rows = np.concatenate([np.arange(offs[p], offs[p + 1]) for p in kept_prims]) if len(kept_prims) else np.zeros(0, np.int64)
    rows = np.concatenate([kept_rows, tri_rows.ravel()])
    owner = np.concatenate([kept_prims, tri_prim])
    sizes_out = np.concatenate([sizes[kept_prims], np.full(len(tri_prim), 3)])
    vp = t.vertex_point.astype(np.int64)[rows]
    return B.make(geo.P, vp, B.offsets_from_sizes(sizes_out), dict(geo.point.items()), B.gather(geo.vertex, rows), B.gather(geo.prim, owner), geo.detail)


def _canonical(a, b):
    return np.minimum(a, b), np.maximum(a, b)


def iso_segments(geo, values, level, cells):
    """Marching squares / triangles at one level over the polygons of ``geo`` (triangles and quads): the two crossed
    mesh edges of each segment, as (lo, hi) point-index pairs. Quads with four crossings are split by the centre value."""
    t = geo.topology
    vp = t.vertex_point.astype(np.int64)
    offs = t.prim_offsets.astype(np.int64)
    sizes = t.prim_sizes()
    inside = values >= level
    segs = []
    for n in (3, 4):
        prims = np.flatnonzero(sizes == n)
        if not len(prims):
            continue
        c = vp[offs[prims][:, None] + np.arange(n)[None, :]]                       # (m, n) corner points
        nx = np.roll(c, -1, axis=1)
        cross = inside[c] != inside[nx]                                              # edge i joins corner i and i + 1
        k = cross.sum(1)
        two = k == 2
        if two.any():
            order = np.argsort(~cross[two], axis=1, kind="stable")[:, :2]            # the two crossed edge numbers
            e0, e1 = order[:, 0], order[:, 1]
            r = np.flatnonzero(two)
            segs.append((c[r, e0], nx[r, e0], c[r, e1], nx[r, e1]))
        if n == 4:
            four = np.flatnonzero(k == 4)
            if len(four):
                cc = c[four]
                s0 = inside[cc[:, 0]]
                centre = values[cc].mean(1) >= level
                same = centre == s0                                                  # corners 0 and 2 are joined through the middle
                for pair_a, pair_b, use in (((0, 1), (2, 3), same), ((1, 2), (3, 0), ~same)):
                    f = four[use]
                    if len(f):
                        segs.append((c[f, pair_a[0]], nx[f, pair_a[0]], c[f, pair_a[1]], nx[f, pair_a[1]]))
                        segs.append((c[f, pair_b[0]], nx[f, pair_b[0]], c[f, pair_b[1]], nx[f, pair_b[1]]))
    if not segs:
        z = np.zeros(0, np.int64)
        return z, z, z, z
    return tuple(np.concatenate([s[i] for s in segs]) for i in range(4))


def iso_lines(geo, values, levels, cells, fill):
    """Curves along the given levels. ``fill``: closed loops stay polygons (else they are open polylines that repeat their first point)."""
    g = prepare_cells(geo, cells)
    values = np.asarray(values, np.float64)
    ax, ay, bx, by = [], [], [], []
    lv = []
    for i, level in enumerate(levels):
        s = iso_segments(g, values, level, cells)
        if len(s[0]):
            ax.append(s[0]); ay.append(s[1]); bx.append(s[2]); by.append(s[3]); lv.append(np.full(len(s[0]), i))
    if not ax:
        return B.make(np.zeros((0, 3), np.float32), np.zeros(0, np.int32), np.zeros(1, np.int32))
    ax, ay, bx, by, lv = (np.concatenate(x) for x in (ax, ay, bx, by, lv))
    lo = np.stack(_canonical(np.r_[ax, bx], np.r_[ay, by]), 1)                       # every segment end is a crossed mesh edge
    key = np.c_[np.r_[lv, lv], lo]
    uniq, inv = np.unique(key, axis=0, return_inverse=True)
    inv = inv.reshape(-1)
    m = len(ax)
    seg_a, seg_b = inv[:m], inv[m:]
    guard_count(len(uniq), "contour points", "use fewer levels or a coarser mesh")
    lvl = np.asarray(levels, np.float64)[uniq[:, 0]]
    ia, ib = uniq[:, 1], uniq[:, 2]
    va, vb = values[ia], values[ib]
    tt = np.where(np.abs(vb - va) > 1e-30, (lvl - va) / np.where(np.abs(vb - va) > 1e-30, vb - va, 1.0), 0.5)
    P = g.P.astype(np.float64)
    pos = P[ia] + (P[ib] - P[ia]) * tt[:, None]
    point = B.lerp(g.point, ia, ib, tt)
    point.pop("N", None)
    point["level"] = lvl.astype(np.float32)
    chains, closed = chain_segments(len(uniq), seg_a, seg_b)
    seg_level = np.zeros(len(uniq))
    seg_level[seg_a] = lv
    sizes, order, is_closed = [], [], []
    for ch, cl in zip(chains, closed):
        ch = list(ch)
        if cl and not fill:
            ch = ch + [ch[0]] if ch[0] != ch[-1] else ch
        sizes.append(len(ch))
        order.extend(ch)
        is_closed.append(bool(cl) and fill)
    order = np.asarray(order, np.int64)
    out = B.make(pos[order].astype(np.float32), np.arange(len(order)), B.offsets_from_sizes(sizes),
                 {k: v[order] for k, v in point.items()}, None, {"level": np.asarray([lvl[order[o]] for o in np.cumsum([0] + sizes[:-1])], np.float32)} if sizes else None,
                 g.detail, np.asarray(is_closed, bool))
    return out


CONTOUR_OUT = OutGroups(
    OutGroup("cutpoints", "Cut Points", "contourpoints", "point", doc="surface mode: the points on the cut"),
)


def clip_surface(geo, values, level, keep_above, cells):
    """The part of every polygon where values >= level (or <= with ``keep_above`` False), cut exactly on the level."""
    g = prepare_cells(geo, cells)
    t = g.topology
    n = g.npoints
    s = np.asarray(values, np.float64) - level
    if not keep_above:
        s = -s
    vp = t.vertex_point.astype(np.int64)
    nv = t.nvertices
    pv = np.repeat(np.arange(t.nprims), t.prim_sizes())
    offs = t.prim_offsets.astype(np.int64)
    nxt = np.arange(nv) + 1
    nxt[offs[1:] - 1] = offs[:-1]                                                    # next corner of the same polygon
    a, b = vp, vp[nxt]
    ina, inb = s[a] >= 0, s[b] >= 0
    case = ina.astype(np.int64) * 2 + inb                                           # 3: keep b; 2: crossing; 1: crossing then b; 0: nothing
    cnt = np.where(case == 3, 1, np.where(case == 2, 1, np.where(case == 1, 2, 0)))
    src = np.repeat(np.arange(nv), cnt)
    k = np.arange(len(src)) - np.repeat(np.cumsum(cnt) - cnt, cnt)
    cs = case[src]
    is_cross = (cs == 2) | ((cs == 1) & (k == 0))
    sa, sb = s[a[src]], s[b[src]]
    tt = np.where(is_cross, -sa / np.where(np.abs(sb - sa) > 1e-30, sb - sa, 1.0), 0.0)
    lo, hi = np.minimum(a[src], b[src]), np.maximum(a[src], b[src])
    ckey = lo[is_cross] * max(n, 1) + hi[is_cross]
    uq, uinv = np.unique(ckey, return_inverse=True)
    cl, ch = uq // max(n, 1), uq % max(n, 1)
    scl, sch = s[cl], s[ch]
    ct = np.where(np.abs(sch - scl) > 1e-30, -scl / np.where(np.abs(sch - scl) > 1e-30, sch - scl, 1.0), 0.5)
    P = g.P.astype(np.float64)
    newP = P[cl] + (P[ch] - P[cl]) * ct[:, None]
    point = {nm: np.concatenate([arr, B.lerp({nm: arr}, cl, ch, ct)[nm]]) for nm, arr in g.point.items() if nm != "P"}
    point.pop("N", None)
    pid = np.where(is_cross, 0, b[src]).astype(np.int64)
    pid[is_cross] = n + uinv.reshape(-1)
    ia = np.where(is_cross, src, nxt[src])
    ib = nxt[src]
    vertex = B.lerp(g.vertex, ia, ib, tt)
    sizes = np.bincount(pv[src], minlength=t.nprims)
    good = sizes >= 3
    keep_corner = good[pv[src]]
    vp_out = pid[keep_corner]
    out_all = np.concatenate([P, newP])
    prim_attrs = B.gather(g.prim, np.flatnonzero(good))
    out = B.make(out_all.astype(np.float32), vp_out, B.offsets_from_sizes(sizes[good]), point,
                 B.gather(vertex, np.flatnonzero(keep_corner)), prim_attrs, g.detail)
    if out.nprims == 0:
        return out, np.zeros(out.npoints, bool)
    used = np.bincount(out.topology.vertex_point, minlength=out.npoints) > 0
    new_pts = np.zeros(out.npoints, bool)
    new_pts[n:] = True
    remap = np.cumsum(used) - 1
    point_c = {nm: a_[used] for nm, a_ in out.point.items() if nm != "P"}
    compact = B.make(out.P[used], remap[out.topology.vertex_point], out.topology.prim_offsets, point_c,
                     dict(out.vertex.items()), dict(out.prim.items()), out.detail)
    return compact, new_pts[used]


@node("contour", inputs=["geo"], category="curve", params=[
    Enum("mode", ["lines", "surface"], doc="lines: curves along the level; surface: cut the mesh on the level and keep one side"),
    Str("attribute", "P.z", doc="The point attribute to contour (a float, or P.x / P.y / P.z; the length of a vector attribute)"),
    Float("level", 0.0, soft_min=-10.0, soft_max=10.0, doc="Value at which to cut"),
    Int("levels", 1, min=1, soft_max=100, doc="lines mode: how many lines (the first at level, then every spacing)"),
    Float("spacing", 1.0, min=1e-9, soft_max=10.0, doc="lines mode: distance in value between lines"),
    Enum("cells", ["polygons", "triangles"], doc="polygons: quads are contoured as quads (marching squares, saddles by the centre value); triangles: every polygon is triangulated first"),
    Bool("keep_above", True, doc="surface mode: keep where the value is above the level (off: below)"),
    Bool("fill", False, doc="lines mode: closed loops stay polygons (off: open polylines, so only the outline shows)"),
    Str("group", "", doc="Only these primitives"),
    *CONTOUR_OUT.params(),
])
def contour(geo, mode, attribute, level, levels, spacing, cells, keep_above, fill, group, **og):
    """Iso-lines or a cut surface of a point attribute over a mesh (marching squares and marching triangles in one node).
    Lines carry the point attributes interpolated, a point attribute and a primitive attribute 'level'. Surface mode keeps the
    polygons whole where it can and cuts them exactly on the level elsewhere."""
    values = _values(geo, attribute)
    src = geo
    sel = groups.mask(geo, "prim", group)
    if sel is not None:
        from .group import keep_prims
        src = keep_prims(geo, sel, remove_unused_points=False)
    if mode == "lines":
        return iso_lines(src, values, [level + i * spacing for i in range(int(levels))], cells, fill)
    out, new_pts = clip_surface(src, values, level, keep_above, cells)
    return CONTOUR_OUT.write(out, og, "cutpoints", new_pts)


def _values(geo, attribute):
    name, _, comp = attribute.partition(".")
    if name == "P":
        arr = geo.P.astype(np.float64)
    elif name in geo.point:
        arr = geo.point[name].astype(np.float64)
    else:
        raise ValueError(f"contour: no point attribute {name!r}")
    if arr.ndim == 1:
        return arr
    if comp:
        if comp not in ("x", "y", "z", "0", "1", "2"):
            raise ValueError(f"contour: component {comp!r} should be x, y or z")
        return arr[:, "xyz".index(comp) if comp in "xyz" else int(comp)]
    return np.linalg.norm(arr, axis=1)


@node("slice", inputs=["geo"], category="curve", params=[
    Vec3("origin", (0.0, 0.0, 0.0), doc="A point on the first plane"),
    Vec3("normal", (0.0, 0.0, 1.0), doc="The planes' normal; they stack along it"),
    Float("spacing", 0.25, min=1e-9, soft_max=5.0, doc="Distance between planes"),
    Int("count", 1, min=1, soft_max=200, doc="How many planes (from the origin along the normal)"),
    Bool("cover", False, doc="Ignore count: planes at every spacing through the whole height of the mesh (contour lines of a terrain, layers for printing)"),
    Bool("fill", False, doc="Closed slices stay polygons (off: outlines only)"),
    Str("group", "", doc="Only these primitives"),
], handles=[Handle("spacing", axis="@normal", at="@origin", reach=True, min=1e-6)],
   guides=[Guide("plane", at="@origin", axis="@normal", id="plane", label="first plane")])
def slice_(geo, origin, normal, spacing, count, cover, fill, group):
    """Cut a mesh with parallel planes into curves (topographic lines, layer outlines, cross-sections). Writes the point
    attribute and primitive attribute 'level' (height above the origin plane along the normal)."""
    nrm = np.asarray(normal, np.float64)
    if float(np.linalg.norm(nrm)) < 1e-12:
        raise ValueError("slice: the normal is zero")
    nrm = nrm / np.linalg.norm(nrm)
    d = (geo.P.astype(np.float64) - np.asarray(origin, np.float64)) @ nrm
    if cover:
        if not geo.npoints:
            return geo
        lo, hi = np.floor(d.min() / spacing), np.ceil(d.max() / spacing)
        steps = np.arange(lo, hi + 1) * spacing
        steps = steps[(steps >= d.min() - 1e-9) & (steps <= d.max() + 1e-9)]
    else:
        steps = np.arange(int(count)) * spacing
    guard_count(len(steps), "slice planes", "use a larger spacing")
    src = geo
    sel = groups.mask(geo, "prim", group)
    if sel is not None:
        from .group import keep_prims
        src = keep_prims(geo, sel, remove_unused_points=False)
    out = iso_lines(src, d, list(steps + 1e-9 * spacing), "polygons", fill)          # a hair off the plane keeps vertices on it from being ambiguous
    return out
