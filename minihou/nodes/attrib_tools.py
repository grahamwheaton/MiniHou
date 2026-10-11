"""Attribute toolkit: create, randomize, promote, transfer, blur, connectivity, measure, color, sort.

Houdini's attribute SOPs. Nodes that take a ``group`` change only the selected elements (see :mod:`minihou.groups`).
"""
import numpy as np

from .. import groups, spatial, topo
from ..sdk import Bool, Enum, Float, Int, Str, Vec3, node
from . import _build as B
from .normal import prim_normals
from ._util import normalize_rows

DOMAINS = {"points": "point", "primitives": "prim", "vertices": "vertex", "detail": "detail"}
TYPES = {"float": (np.float32, 1), "int": (np.int32, 1), "vector": (np.float32, 3), "vector2": (np.float32, 2),
         "color": (np.float32, 3), "vector4": (np.float32, 4)}


def _size(geo, dom):
    return {"point": geo.npoints, "prim": geo.nprims, "vertex": geo.nvertices}[dom]


def _set(geo, dom, name, arr, group=""):
    """Write an attribute on a domain (moving it there, like the wrangle), limited to a group."""
    out = geo.copy()
    for other in ("point", "prim", "vertex"):
        if other != dom and name in out.table(other):
            del out.table(other)[name]
    out.table(dom)[name] = arr
    if group and dom in ("point", "prim"):
        return groups.restrict(geo, out, dom, group)
    return out


def _check_name(name):
    if not name or not name.replace("_", "a").isalnum() or name[0].isdigit():
        raise ValueError(f"invalid attribute name {name!r}")


# -- create -----------------------------------------------------------------------------------------------------
@node("attribute_create", inputs=["geo"], category="attribute", params=[
    Str("name", "attrib1"),
    Enum("domain", ["points", "primitives", "vertices", "detail"]),
    Enum("type", ["float", "int", "vector", "vector2", "color", "vector4"]),
    Vec3("value", (0.0, 0.0, 0.0), doc="Value (float/int use the first component; vector4 adds w = 1)"),
    Str("group", "", doc="Only set it on these elements (others get 0); points or primitives"),
])
def attribute_create(geo, name, domain, type, value, group):
    """Add an attribute with a constant value (Houdini's Attribute Create)."""
    _check_name(name)
    dom = DOMAINS[domain]
    dtype, width = TYPES[type]
    v = np.asarray(list(value) + [1.0], np.float64)[:width]
    if dom == "detail":
        out = geo.copy()
        out.detail[name] = v[0].item() if width == 1 else v.tolist()
        return out
    n = _size(geo, dom)
    arr = np.full((n, width), v, dtype) if width > 1 else np.full(n, v[0], dtype)
    if group and dom in ("point", "prim"):
        sel = groups.mask(geo, dom, group)
        arr = np.where(sel.reshape((-1,) + (1,) * (arr.ndim - 1)), arr, 0).astype(dtype)
    return _set(geo, dom, name, arr)


# -- randomize --------------------------------------------------------------------------------------------------
@node("attribute_randomize", inputs=["geo"], category="attribute", params=[
    Str("name", "Cd"),
    Enum("domain", ["points", "primitives", "vertices"]),
    Enum("type", ["color", "float", "vector", "int"], doc="int: whole numbers from min to max inclusive"),
    Vec3("min", (0.0, 0.0, 0.0)),
    Vec3("max", (1.0, 1.0, 1.0)),
    Enum("distribution", ["uniform", "normal"], doc="normal: mean (min+max)/2, 99.7% within min..max"),
    Int("seed", 0),
    Str("by", "", doc="Optional int attribute (e.g. 'class' from connectivity): equal values get equal random values"),
    Str("group", "", doc="Only these elements (points or primitives)"),
])
def attribute_randomize(geo, name, domain, type, min, max, distribution, seed, by, group):
    """Random values per element, or per piece with 'by' (Houdini's Attribute Randomize). Deterministic per seed."""
    _check_name(name)
    dom = DOMAINS[domain]
    n = _size(geo, dom)
    width = 1 if type in ("float", "int") else 3
    if by:
        src = geo.table(dom).get(by)
        if src is None:
            raise ValueError(f"no {dom} attribute {by!r} to randomize by")
        keys = np.asarray(src).reshape(n, -1)[:, 0].astype(np.int64)
        uniq, inv = np.unique(keys, return_inverse=True)
        rng = np.random.default_rng([int(seed) & 0xFFFFFFFF, 7])
        r = _draw(rng, len(uniq), width, distribution)[inv.reshape(-1)]
    else:
        r = _draw(np.random.default_rng(int(seed) & 0xFFFFFFFF), n, width, distribution)
    lo, hi = np.asarray(min, np.float64)[:width], np.asarray(max, np.float64)[:width]
    if type == "int":
        arr = np.floor(lo[0] + r[:, 0] * (hi[0] - lo[0] + 1)).clip(lo[0], hi[0]).astype(np.int32)
    else:
        arr = (lo + r * (hi - lo)).astype(np.float32)
        arr = arr[:, 0] if width == 1 else arr
    return _set(geo, dom, name, arr, group)


def _draw(rng, n, width, distribution):
    if distribution == "normal":
        return np.clip(0.5 + rng.standard_normal((n, width)) / 6.0, 0.0, 1.0)
    return rng.random((n, width))


# -- promote ----------------------------------------------------------------------------------------------------
@node("attribute_promote", inputs=["geo"], category="attribute", params=[
    Str("name", "Cd", doc="Attribute to move (wildcards promote several)"),
    Enum("to", ["points", "primitives", "vertices", "detail"]),
    Enum("method", ["average", "min", "max", "sum", "first"], doc="How several values combine into one"),
    Bool("keep_original", False),
])
def attribute_promote(geo, name, to, method, keep_original):
    """Move an attribute to another domain (Houdini's Attribute Promote): points <-> primitives <-> vertices -> detail."""
    import fnmatch
    dst = DOMAINS[to]
    hits = [(d, nm) for d in ("point", "prim", "vertex") for nm in geo.table(d).names()
            if fnmatch.fnmatchcase(nm, name) and d != dst and (nm != "P" or keep_original)]   # P can only be copied
    if not hits:
        raise ValueError(f"no attribute {name!r} on another domain than {to}")
    out = geo.copy()
    vp = geo.topology.vertex_point.astype(np.int64)
    pv = topo.prim_of_vertex(geo)
    for src, nm in hits:
        a = geo.table(src)[nm]
        # express on vertices first (every domain maps to vertices), then gather to the destination
        if src == "point":
            av = a[vp]
        elif src == "prim":
            av = a[pv]
        else:
            av = a
        if dst == "vertex":
            res = av
        elif dst == "detail":
            res = _reduce(a, np.zeros(len(a), np.int64), 1, method)[0]
            out.detail[nm] = res.item() if np.ndim(res) == 0 else np.asarray(res).tolist()
            if not keep_original:
                del out.table(src)[nm]
            continue
        else:
            idx = vp if dst == "point" else pv
            nd = geo.npoints if dst == "point" else geo.nprims
            res = _reduce(av, idx, nd, method)
        if not keep_original:
            del out.table(src)[nm]
        out.table(dst)[nm] = res
    return out


def _reduce(a, idx, n, method):
    if method == "average":
        return B.mean_by({"x": a}, np.arange(len(a)), idx, n)["x"] if a.dtype.kind == "f" else \
            _reduce(a, idx, n, "first")
    flat = a.reshape(len(a), -1)
    out = np.zeros((n, flat.shape[1]), a.dtype if method != "sum" or a.dtype.kind != "b" else np.int32)
    if len(a):
        order = np.argsort(idx, kind="stable")
        g = idx[order]
        starts = np.flatnonzero(np.r_[True, g[1:] != g[:-1]])
        vals = flat[order]
        if method == "first":
            r = vals[starts]
        elif method == "sum":
            r = np.add.reduceat(vals.astype(out.dtype), starts, axis=0)
        else:
            r = (np.minimum if method == "min" else np.maximum).reduceat(vals, starts, axis=0)
        out[g[starts]] = r
    return out[:, 0] if a.ndim == 1 else out


# -- transfer ---------------------------------------------------------------------------------------------------
@node("attribute_transfer", inputs=["geo", "source"], category="attribute", params=[
    Str("names", "Cd", doc="Point attributes to copy from the nearest source point, space separated (wildcards work)"),
    Float("max_distance", 1e9, min=0.0, doc="Points farther than this from the source keep their own values"),
    Float("blend_width", 0.0, min=0.0, doc="Soft edge: the source's influence fades to zero over the last blend_width "
                                           "before max_distance"),
    Str("group", "", doc="Only these points of the first input"),
])
def attribute_transfer(geo, source, names, max_distance, blend_width, group):
    """Copy point attributes from the nearest point of a second geometry (Houdini's Attribute Transfer, nearest-point
    mode). Within max_distance the source value is taken; blend_width softens the edge. Exact nearest neighbours
    (SciPy's KD-tree when installed, otherwise a numpy grid)."""
    import fnmatch
    pats = names.replace(",", " ").split()
    want = [nm for nm in source.point.names() if nm != "P" and any(fnmatch.fnmatchcase(nm, p) for p in pats)]
    if not want:
        raise ValueError(f"the source has no point attribute matching {names!r}; it has {source.point.names()}")
    idx, dist = spatial.nearest(source.P, geo.P)
    w = np.where(dist <= max_distance, 1.0, 0.0)
    if blend_width > 0:
        w = np.clip(1.0 - (dist - (max_distance - blend_width)) / blend_width, 0.0, 1.0)
    sel = groups.mask(geo, "point", group)
    if sel is not None:
        w = w * sel
    out = geo.copy()
    for nm in want:
        s = source.point[nm][np.maximum(idx, 0)]
        own = geo.point.get(nm)
        if own is None or own.shape[1:] != s.shape[1:]:
            own = np.zeros_like(s)
        if s.dtype.kind == "f":
            ww = w.reshape((-1,) + (1,) * (s.ndim - 1)).astype(np.float32)
            res = (own.astype(np.float32) * (1 - ww) + s * ww).astype(np.float32)
        else:
            res = np.where((w >= 0.5).reshape((-1,) + (1,) * (s.ndim - 1)), s, own.astype(s.dtype))
        for other in ("prim", "vertex"):
            if nm in out.table(other):
                del out.table(other)[nm]
        out.point[nm] = res
    return out


# -- blur / smooth ----------------------------------------------------------------------------------------------
@node("attribute_blur", inputs=["geo"], category="attribute", params=[
    Str("names", "P", doc="Point attributes to smooth along edges (P smooths the shape), space separated"),
    Int("iterations", 5, min=0, soft_max=100),
    Float("strength", 0.5, min=0.0, max=1.0, doc="How far each step moves towards the neighbours' average"),
    Bool("pin_boundary", True, doc="Keep points on open edges, and the ends of curves, in place"),
    Str("group", "", doc="Only these points"),
])
def attribute_blur(geo, names, iterations, strength, pin_boundary, group):
    """Laplacian smoothing of point attributes over the mesh edges (Houdini's Attribute Blur / Smooth)."""
    e = topo.edges(geo)
    a, b = e.points[:, 0], e.points[:, 1]
    n = geo.npoints
    deg = np.maximum(np.bincount(np.r_[a, b], minlength=n), 1).astype(np.float64)
    move = np.ones(n, bool) if group == "" else groups.mask(geo, "point", group)
    if pin_boundary:                                     # open polygon edges, and the ends of curves
        move &= ~topo.boundary_points(geo)
        move &= np.bincount(np.r_[a, b], minlength=n) != 1
    move &= np.bincount(np.r_[a, b], minlength=n) > 0
    out = geo.copy()
    for nm in names.replace(",", " ").split():
        arr = geo.point.get(nm)
        if arr is None:
            raise ValueError(f"no point attribute {nm!r}")
        if arr.dtype.kind != "f":
            raise ValueError(f"{nm!r} is not a float attribute; only floats can be blurred")
        v = arr.reshape(n, -1).astype(np.float64)
        for _ in range(iterations):
            s = np.stack([np.bincount(a, weights=v[b, c], minlength=n) + np.bincount(b, weights=v[a, c], minlength=n)
                          for c in range(v.shape[1])], 1)
            avg = s / deg[:, None]
            v = np.where(move[:, None], v + strength * (avg - v), v)
        out.point[nm] = v.astype(np.float32).reshape(arr.shape)
    return out


# -- connectivity -----------------------------------------------------------------------------------------------
def piece_labels(geo):
    """Connected-piece number per point and per primitive (pieces share points)."""
    vp = geo.topology.vertex_point.astype(np.int64)
    nxt = topo.next_vertex(geo)
    pts = spatial.components(geo.npoints, vp, vp[nxt])
    first_vertex = geo.topology.prim_offsets[:-1]
    sizes = geo.topology.prim_sizes()
    prim = np.where(sizes > 0, pts[vp[np.minimum(first_vertex, max(len(vp) - 1, 0))]] if len(vp) else 0, -1)
    return pts, prim.astype(np.int32)


@node("connectivity", inputs=["geo"], category="attribute", params=[
    Enum("domain", ["primitives", "points"], doc="Where to write the piece number"),
    Str("name", "class"),
])
def connectivity(geo, domain, name):
    """Number the connected pieces 0, 1, 2... (Houdini's Connectivity): faces sharing points are one piece. Use it
    with attribute_randomize 'by', blast, or for_each."""
    pts, prim = piece_labels(geo)
    dom = DOMAINS[domain]
    return _set(geo, dom, name, pts if dom == "point" else prim)


# -- measure ----------------------------------------------------------------------------------------------------
@node("measure", inputs=["geo"], category="attribute", params=[
    Enum("measure", ["area", "perimeter", "curvature", "piece_area", "piece_volume"],
         doc="area / perimeter per primitive (an open curve's perimeter is its length); curvature per point (mean curvature, 1/radius: + convex, - concave); piece_area / "
             "piece_volume per primitive, summed over its connected piece"),
    Str("name", "", doc="Attribute name (default: the measure's name)"),
])
def measure(geo, measure, name):
    """Measure geometry (Houdini's Measure SOP)."""
    name = name or measure
    fn = prim_normals(geo)
    if measure == "area":
        return _set(geo, "prim", name, (np.linalg.norm(fn, axis=1) / 2).astype(np.float32))
    if measure == "perimeter":
        vp = geo.topology.vertex_point.astype(np.int64)
        P = geo.P.astype(np.float64)
        L = np.linalg.norm(P[vp[topo.next_vertex(geo)]] - P[vp], axis=1) * topo.has_next(geo)   # curves: no closing edge
        return _set(geo, "prim", name, topo.segment_sum(L, topo.prim_of_vertex(geo), geo.nprims).astype(np.float32))
    if measure == "curvature":
        e = topo.edges(geo)
        a, b = e.points[:, 0], e.points[:, 1]
        n = geo.npoints
        P = geo.P.astype(np.float64)
        deg = np.maximum(np.bincount(np.r_[a, b], minlength=n), 1)
        avg = topo.segment_sum(np.r_[P[b], P[a]], np.r_[a, b], n) / deg[:, None]
        acc = topo.segment_sum(fn[topo.prim_of_vertex(geo)], geo.topology.vertex_point.astype(np.int64), n)
        N = normalize_rows(acc).astype(np.float64)
        # a ring of neighbours at distance l on a sphere of radius R sits l^2 / 2R below the point: 1/R = 2 h / l^2
        l2 = topo.segment_sum(np.tile(((P[a] - P[b]) ** 2).sum(1), 2), np.r_[a, b], n) / deg
        k = 2 * np.einsum("ij,ij->i", P - avg, N) / np.maximum(l2, 1e-24)
        k = np.where(topo.boundary_points(geo), 0.0, k)
        return _set(geo, "point", name, k.astype(np.float32))
    _, piece = piece_labels(geo)
    k = int(piece.max()) + 1 if len(piece) else 0
    if measure == "piece_area":
        per = np.bincount(piece, weights=np.linalg.norm(fn, axis=1) / 2, minlength=k)
    else:
        c = topo.prim_centroids(geo)
        per = np.bincount(piece, weights=np.einsum("ij,ij->i", fn, c) / 6.0, minlength=k)
    return _set(geo, "prim", name, per[piece].astype(np.float32))


# -- color ------------------------------------------------------------------------------------------------------
@node("color", inputs=["geo"], category="attribute", params=[
    Enum("mode", ["constant", "random", "random_by_piece", "ramp"],
         doc="ramp: from color to color2 along an attribute's range"),
    Enum("domain", ["points", "primitives", "vertices"]),
    Vec3("color", (1.0, 0.5, 0.2)),
    Vec3("color2", (0.1, 0.3, 1.0), doc="ramp end color"),
    Str("ramp_attrib", "P.z", doc="ramp: attribute (with .x/.y/.z for a component), mapped from its min to its max"),
    Int("seed", 0),
    Str("group", "", doc="Only these points or primitives"),
])
def color(geo, mode, domain, color, color2, ramp_attrib, seed, group):
    """Set Cd (Houdini's Color SOP). Blender shows it as the color attribute 'Color'."""
    dom = DOMAINS[domain]
    n = _size(geo, dom)
    c1, c2 = np.asarray(color, np.float32), np.asarray(color2, np.float32)
    if mode == "constant":
        cd = np.tile(c1, (n, 1))
    elif mode in ("random", "random_by_piece"):
        rng = np.random.default_rng(int(seed) & 0xFFFFFFFF)
        if mode == "random":
            cd = rng.random((n, 3)).astype(np.float32)
        else:
            pts, prim = piece_labels(geo)
            lab = {"point": pts, "prim": prim, "vertex": prim[topo.prim_of_vertex(geo)]}[dom]
            pal = rng.random((int(lab.max()) + 1 if len(lab) else 0, 3)).astype(np.float32)
            cd = pal[lab]
    else:
        nm, _, comp = ramp_attrib.partition(".")
        from .wrangle import _Domains
        a = _Domains(geo).read(dom, nm)
        if a is None:
            raise ValueError(f"no attribute {nm!r} to ramp by")
        a = np.asarray(a, np.float64).reshape(n, -1)
        v = a[:, "xyzw".index(comp)] if comp else (a[:, 0] if a.shape[1] == 1 else np.linalg.norm(a, axis=1))
        t = (v - v.min()) / max(float(v.max() - v.min()), 1e-12) if len(v) else v
        cd = (c1 * (1 - t[:, None]) + c2 * t[:, None]).astype(np.float32)
    return _set(geo, dom, "Cd", cd.astype(np.float32), group)


# -- sort -------------------------------------------------------------------------------------------------------
@node("sort", inputs=["geo"], category="attribute", params=[
    Enum("domain", ["points", "primitives"]),
    Enum("key", ["random", "x", "y", "z", "attribute", "reverse"]),
    Str("attribute", "", doc="key = attribute: sort by this attribute (first component)"),
    Int("seed", 0),
])
def sort(geo, domain, key, attribute, seed):
    """Reorder points or primitives (Houdini's Sort): changes @ptnum / @primnum order, which matters for copies,
    ids and anything indexed by number. Topology and attributes are kept consistent."""
    dom = DOMAINS[domain]
    n = _size(geo, dom)
    if key == "random":
        order = np.random.default_rng(int(seed) & 0xFFFFFFFF).permutation(n)
    elif key == "reverse":
        order = np.arange(n)[::-1]
    elif key in "xyz":
        pos = geo.P if dom == "point" else topo.prim_centroids(geo)
        order = np.argsort(pos[:, "xyz".index(key)], kind="stable")
    else:
        a = geo.table(dom).get(attribute)
        if a is None:
            raise ValueError(f"no {dom} attribute {attribute!r}")
        order = np.argsort(np.asarray(a).reshape(n, -1)[:, 0], kind="stable")
    t = geo.topology
    if dom == "point":
        inv = np.empty(n, np.int64)
        inv[order] = np.arange(n)
        return B.make(geo.P[order], inv[t.vertex_point], t.prim_offsets, B.gather(geo.point, order),
                      dict(geo.vertex.items()), dict(geo.prim.items()), geo.detail, t.closed)
    sizes = t.prim_sizes()[order].astype(np.int64)
    offs = t.prim_offsets.astype(np.int64)
    local = np.arange(sizes.sum()) - np.repeat(np.cumsum(sizes) - sizes, sizes)
    verts = np.repeat(offs[order], sizes) + local
    return B.make(geo.P, t.vertex_point[verts], B.offsets_from_sizes(sizes), dict(geo.point.items()),
                  B.gather(geo.vertex, verts), B.gather(geo.prim, order), geo.detail, B.closed_subset(geo, order))
