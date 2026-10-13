"""Groups: create, convert and delete by group (Houdini's Group Create, Group Promote, Blast)."""
import fnmatch

import numpy as np

from .. import edges as edgelib, groups, topo
from ..geometry import Geometry, Topology
from ..sdk import Bool, Enum, Float, Guide, Handle, Input, Int, Str, Vec3, node
from . import _build as B
from .delete import keep_points
from .normal import prim_normals
from ._util import normalize_rows

DOMAIN = {"points": "point", "primitives": "prim", "edges": "edge"}


def _positions(geo, dom):
    return geo.P.astype(np.float64) if dom == "point" else topo.prim_centroids(geo)


def _normals(geo, dom):
    if dom == "prim":
        return normalize_rows(prim_normals(geo)).astype(np.float64)
    if "N" in geo.point and geo.point["N"].ndim == 2:
        return geo.point["N"].astype(np.float64)
    fn = prim_normals(geo)                                   # area-weighted point normals on the fly
    acc = topo.segment_sum(fn[topo.prim_of_vertex(geo)], geo.topology.vertex_point.astype(np.int64), geo.npoints)
    return normalize_rows(acc).astype(np.float64)


def _hash01(values, seed):
    """A repeatable pseudo-random number in [0, 1) per value (splitmix64), so a seed *attribute* gives each element its own."""
    x = (np.asarray(values).astype(np.int64).astype(np.uint64) + np.uint64((int(seed) * 0x9E3779B97F4A7C15) & 0xFFFFFFFFFFFFFFFF))
    with np.errstate(over="ignore"):
        x = (x + np.uint64(0x9E3779B97F4A7C15))
        x = (x ^ (x >> np.uint64(30))) * np.uint64(0xBF58476D1CE4E5B9)
        x = (x ^ (x >> np.uint64(27))) * np.uint64(0x94D049BB133111EB)
        x = x ^ (x >> np.uint64(31))
    return (x >> np.uint64(11)).astype(np.float64) / float(1 << 53)


def _element_points(geo, dom):
    """The points of each element, as (element index per entry, point index per entry)."""
    if dom == "point":
        return np.arange(geo.npoints), np.arange(geo.npoints)
    if dom == "prim":
        return topo.prim_of_vertex(geo), geo.topology.vertex_point.astype(np.int64)
    e = topo.edges(geo).points
    return np.repeat(np.arange(len(e)), 2), e.reshape(-1)


def _edge_normals(geo):
    """Average unit normal of the faces on each edge, and the angle between them (degrees; 0 for border edges)."""
    e = topo.edges(geo)
    pv = topo.prim_of_vertex(geo)
    ok = e.of_vertex >= 0
    N = normalize_rows(prim_normals(geo)).astype(np.float64)
    acc = np.stack([np.bincount(e.of_vertex[ok], weights=N[pv[ok], c], minlength=len(e.points)) for c in range(3)], 1)
    avg = normalize_rows(acc).astype(np.float64)
    # the angle between the two faces: the widest pair of normals around the edge
    first = np.full(len(e.points), -1, np.int64)
    first[e.of_vertex[ok][::-1]] = pv[ok][::-1]
    cosmin = np.ones(len(e.points))
    n_first = N[np.maximum(first, 0)]
    cos_all = np.einsum("ij,ij->i", N[pv[ok]], n_first[e.of_vertex[ok]])
    np.minimum.at(cosmin, e.of_vertex[ok], cos_all)
    return avg, np.degrees(np.arccos(np.clip(cosmin, -1.0, 1.0)))


def region_from(bounds, center, size):
    """The bounding region of a group node: the bounding box of the optional ``bounds`` input (Houdini's Bounding
    Object input) when it has points, else the node's own centre and size."""
    if bounds is not None and bounds.npoints:
        P = bounds.P.astype(np.float64)
        lo, hi = P.min(0), P.max(0)
        return tuple((lo + hi) / 2), tuple(np.maximum(hi - lo, 1e-9))
    return center, size


def _filters(geo, dom, name, p):
    """Houdini's Group Create filters, in its order: a base group, then (each when enabled) a bounding region, normals,
    edges and random chance keep only what passes all of them."""
    n = {"point": geo.npoints, "prim": geo.nprims, "edge": len(topo.edges(geo).points)}[dom]
    m = np.ones(n, bool)
    if p["base_enable"]:
        if dom == "edge":
            m = edgelib.mask(geo, p["pattern"])
        else:
            b = groups.mask(geo, dom, p["pattern"])
            m = np.ones(n, bool) if b is None else b
    elem, pts = _element_points(geo, dom)
    P = geo.P.astype(np.float64)
    if p["bounds_enable"]:
        c = np.asarray(p["box_center"], np.float64)
        half = np.maximum(np.abs(np.asarray(p["box_size"], np.float64)) / 2, 1e-12)
        q = (P[pts] - c) / half
        inside = (np.all(np.abs(q) <= 1.0 + 1e-9, axis=1) if p["bounds_type"] == "box" else (q * q).sum(1) <= 1.0 + 1e-9)
        cnt = np.bincount(elem, weights=inside, minlength=n)
        tot = np.maximum(np.bincount(elem, minlength=n), 1)
        m &= (cnt > 0) if p["bounds_partial"] else (cnt >= tot)
    if p["normals_enable"]:
        if dom == "edge":
            N, _ = _edge_normals(geo)
        else:
            N = _normals(geo, dom)
        d = np.asarray(p["direction"], np.float64)
        d = d / max(np.linalg.norm(d), 1e-12)
        cos = N @ d
        keep = cos >= np.cos(np.radians(p["angle"])) - 1e-9
        if p["normals_opposite"]:
            keep |= -cos >= np.cos(np.radians(p["angle"])) - 1e-9
        if (p["backface_from"] or "").strip():                      # drop what faces away from a viewpoint
            try:
                eye = np.array([float(x) for x in p["backface_from"].replace(",", " ").split()], np.float64)
                assert eye.shape == (3,)
            except (ValueError, AssertionError):
                raise ValueError("backface_from must be a position like '0 5 0'") from None
            pos = _positions(geo, dom) if dom != "edge" else P[topo.edges(geo).points].mean(1)
            keep &= np.einsum("ij,ij->i", N, eye - pos) > 0
        m &= keep
    if p["edges_enable"]:
        if dom == "edge":
            e = topo.edges(geo)
            _, ang = _edge_normals(geo)
            length = np.linalg.norm(P[e.points[:, 0]] - P[e.points[:, 1]], axis=1)
            if p["min_edge_angle_on"]:
                m &= ang >= p["min_edge_angle"] - 1e-9
            if p["max_edge_angle_on"]:
                m &= ang <= p["max_edge_angle"] + 1e-9
            if p["min_edge_length_on"]:
                m &= length >= p["min_edge_length"]
            if p["max_edge_length_on"]:
                m &= length <= p["max_edge_length"]
            if p["unshared_edges"]:
                m &= e.count == 1
        elif p["unshared_edges"]:                                    # points / primitives on an open border
            m &= topo.boundary_points(geo) if dom == "point" else topo.boundary_prims(geo)
    if p["random_enable"]:
        if (p["seed_attrib"] or "").strip():
            table = geo.table({"point": "point", "prim": "prim", "edge": "vertex"}[dom])
            a = table.get(p["seed_attrib"].strip())
            if a is None or dom == "edge":
                raise ValueError(f"no {'point' if dom == 'point' else 'primitive'} attribute {p['seed_attrib']!r} to seed the random choice")
            r = _hash01(a if a.ndim == 1 else a[:, 0], p["seed"])
        else:
            r = np.random.default_rng(int(p["seed"]) & 0xFFFFFFFF).random(n)
        m &= r < p["percent"] / 100.0
    return m


def _boundary_edge_mask(geo, dom, member):
    """Edges between the group and the rest (or on an open border): the outline of what was selected."""
    e = topo.edges(geo)
    if dom == "edge":
        return member & (e.count == 1)
    ok = e.of_vertex >= 0
    side = (member[topo.prim_of_vertex(geo)] if dom == "prim" else member[geo.topology.vertex_point.astype(np.int64)])
    inside = np.bincount(e.of_vertex[ok & side], minlength=len(e.points))
    total = np.bincount(e.of_vertex[ok], minlength=len(e.points))
    if dom == "prim":
        return (inside > 0) & (inside < total) | ((inside > 0) & (e.count == 1))
    both = member[e.points].all(axis=1)
    return both & (e.count == 1)


@node("group_create", inputs=["geo", Input("bounds", optional=True, doc="Bounding object: when connected, its bounding box replaces box_center/box_size")],
      category="group", params=[
    Str("name", "group1", doc="Group name; wrangles read it as @group_<name>"),
    Enum("domain", ["points", "primitives", "edges"], doc="What the group holds (an edge group is stored like the edge_group node's)"),
    Enum("method", ["pattern", "bounding_box", "normal", "random", "boundary", "filters"],
         doc="pattern: groups, ranges or an expression like '@P.z > 1'; bounding_box: inside a box; normal: facing a "
             "direction; random: a percentage; boundary: on open edges; filters: Houdini's combinable sections "
             "(base group, bounding region, normals, edges, random chance: an element must pass every enabled one)"),
    Str("pattern", "*", doc="pattern method: group names, ^exclusions, numbers/ranges ('0-9'), or '@P.z > 1'"),
    Vec3("box_center", (0.0, 0.0, 0.0), doc="bounding_box method"),
    Vec3("box_size", (1.0, 1.0, 1.0), doc="bounding_box method"),
    Vec3("direction", (0.0, 0.0, 1.0), doc="normal method: elements whose normal is within 'angle' of this"),
    Float("angle", 45.0, min=0.0, max=180.0, doc="normal method, in degrees"),
    Float("percent", 50.0, min=0.0, max=100.0, doc="random method"),
    Int("seed", 0, doc="random method"),
    Enum("combine", ["replace", "union", "intersect", "subtract"], doc="How to merge with an existing group of the name (Initial Merge)"),
    Bool("base_enable", True, doc="filters: start from the base group in 'pattern' (empty: everything)"),
    Bool("bounds_enable", False, doc="filters: Keep in Bounding Regions"),
    Enum("bounds_type", ["box", "sphere"], doc="filters: the region's shape; box_size is its full size (a sphere is the ellipsoid inside it)"),
    Bool("bounds_partial", False, doc="filters: include entities that are only partly inside (otherwise all of their points must be)"),
    Bool("normals_enable", False, doc="filters: Keep by Normals (direction and angle above)"),
    Str("backface_from", "", doc="filters: a position 'x y z'; drops what faces away from it"),
    Bool("normals_opposite", False, doc="filters: also keep normals matching the opposite direction"),
    Bool("edges_enable", False, doc="filters: Include by Edges (angle and length apply to edge groups; unshared edges to all)"),
    Bool("min_edge_angle_on", False, doc="filters: only edges at least this many degrees off flat"),
    Float("min_edge_angle", 20.0, min=0.0, max=180.0),
    Bool("max_edge_angle_on", False, doc="filters: only edges at most this many degrees off flat"),
    Float("max_edge_angle", 20.0, min=0.0, max=180.0),
    Bool("min_edge_length_on", False),
    Float("min_edge_length", 0.0, min=0.0, soft_max=10.0),
    Bool("max_edge_length_on", False),
    Float("max_edge_length", 1.0, min=0.0, soft_max=10.0),
    Bool("unshared_edges", False, doc="filters: only edges used by one face (open borders); for points and primitives, those on a border"),
    Bool("boundary_groups", False, doc="filters: also make '<name>_boundary', the edge group around what was selected"),
    Bool("random_enable", False, doc="filters: Keep by Random Chance (percent and seed above)"),
    Str("seed_attrib", "", doc="filters: a point or primitive attribute that seeds each element separately (empty: by element number)"),
], handles=[Handle("box_center", axis="x", at="@box_center", comp=0, id="box_center_x", when="method=bounding_box&_inputs!=2"),
                 Handle("box_center", axis="y", at="@box_center", comp=1, id="box_center_y", when="method=bounding_box&_inputs!=2"),
                 Handle("box_center", axis="z", at="@box_center", comp=2, id="box_center_z", when="method=bounding_box&_inputs!=2"),
                 Handle("box_size", axis="x", at="@box_center", comp=0, reach=True, scale=2.0, min=0.0, id="box_size_x", when="method=bounding_box&_inputs!=2"),
                 Handle("box_size", axis="y", at="@box_center", comp=1, reach=True, scale=2.0, min=0.0, id="box_size_y", when="method=bounding_box&_inputs!=2"),
                 Handle("box_size", axis="z", at="@box_center", comp=2, reach=True, scale=2.0, min=0.0, id="box_size_z", when="method=bounding_box&_inputs!=2"),
                 Handle("box_center", axis="x", at="@box_center", comp=0, id="box_center_x_f", when="method=filters&bounds_enable&_inputs!=2"),
                 Handle("box_center", axis="y", at="@box_center", comp=1, id="box_center_y_f", when="method=filters&bounds_enable&_inputs!=2"),
                 Handle("box_center", axis="z", at="@box_center", comp=2, id="box_center_z_f", when="method=filters&bounds_enable&_inputs!=2"),
                 Handle("box_size", axis="x", at="@box_center", comp=0, reach=True, scale=2.0, min=0.0, id="box_size_x_f", when="method=filters&bounds_enable&_inputs!=2"),
                 Handle("box_size", axis="y", at="@box_center", comp=1, reach=True, scale=2.0, min=0.0, id="box_size_y_f", when="method=filters&bounds_enable&_inputs!=2"),
                 Handle("box_size", axis="z", at="@box_center", comp=2, reach=True, scale=2.0, min=0.0, id="box_size_z_f", when="method=filters&bounds_enable&_inputs!=2")],
   guides=[Guide("box", at="@box_center", size="@box_size", when="method=bounding_box", id="region", label="bounding box"),
           Guide("box", at="@box_center", size="@box_size", when="method=filters&bounds_enable&bounds_type=box", id="region_f", label="bounding region"),
           Guide("sphere", at="@box_center", size="@box_size", when="method=filters&bounds_enable&bounds_type=sphere", id="region_s", label="bounding region")])
def group_create(geo, bounds, name, domain, method, pattern, box_center, box_size, direction, angle, percent, seed, combine,
                 base_enable=True, bounds_enable=False, bounds_type="box", bounds_partial=False, normals_enable=False,
                 backface_from="", normals_opposite=False, edges_enable=False, min_edge_angle_on=False, min_edge_angle=20.0,
                 max_edge_angle_on=False, max_edge_angle=20.0, min_edge_length_on=False, min_edge_length=0.0,
                 max_edge_length_on=False, max_edge_length=1.0, unshared_edges=False, boundary_groups=False, random_enable=False,
                 seed_attrib=""):
    """Make a named group of points, primitives or edges (stored as the int attribute group_<name>, 1 = member; edge
    groups as edge_group_<name>). Most nodes take a group field, and Blender gets each group as a boolean attribute."""
    dom = DOMAIN[domain]
    box_center, box_size = region_from(bounds, box_center, box_size)
    if method == "filters" or dom == "edge":
        m = _filters(geo, dom, name, dict(
            pattern=pattern, base_enable=base_enable or method != "filters", bounds_enable=bounds_enable, bounds_type=bounds_type,
            box_center=box_center, box_size=box_size, bounds_partial=bounds_partial, normals_enable=normals_enable,
            direction=direction, angle=angle, backface_from=backface_from, normals_opposite=normals_opposite,
            edges_enable=edges_enable, min_edge_angle_on=min_edge_angle_on, min_edge_angle=min_edge_angle,
            max_edge_angle_on=max_edge_angle_on, max_edge_angle=max_edge_angle, min_edge_length_on=min_edge_length_on,
            min_edge_length=min_edge_length, max_edge_length_on=max_edge_length_on, max_edge_length=max_edge_length,
            unshared_edges=unshared_edges, random_enable=random_enable and method == "filters", percent=percent, seed=seed,
            seed_attrib=seed_attrib) if method == "filters" else dict(
            pattern=pattern, base_enable=True, bounds_enable=method == "bounding_box", bounds_type="box", box_center=box_center,
            box_size=box_size, bounds_partial=False, normals_enable=method == "normal", direction=direction, angle=angle,
            backface_from="", normals_opposite=False, edges_enable=method == "boundary", min_edge_angle_on=False, min_edge_angle=0,
            max_edge_angle_on=False, max_edge_angle=0, min_edge_length_on=False, min_edge_length=0, max_edge_length_on=False,
            max_edge_length=0, unshared_edges=True, random_enable=method == "random", percent=percent, seed=seed, seed_attrib=""))
        out = _write_group(geo, dom, name, m, combine)
        if boundary_groups and method == "filters":
            outline = _boundary_edge_mask(geo, dom, m)
            out = edgelib.write(out, edgelib.PREFIX + _checked_name(name + "_boundary"), outline.astype(np.int32))
        return out
    n = geo.npoints if dom == "point" else geo.nprims
    if method == "pattern":
        m = groups.mask(geo, dom, pattern)
        m = np.ones(n, bool) if m is None else m
    elif method == "bounding_box":
        lo = np.asarray(box_center, np.float64) - np.abs(np.asarray(box_size, np.float64)) / 2
        hi = lo + np.abs(np.asarray(box_size, np.float64))
        p = _positions(geo, dom)
        m = np.all((p >= lo) & (p <= hi), axis=1)
    elif method == "normal":
        d = np.asarray(direction, np.float64)
        d = d / max(np.linalg.norm(d), 1e-12)
        m = _normals(geo, dom) @ d >= np.cos(np.radians(angle)) - 1e-9
    elif method == "random":
        r = np.random.default_rng(int(seed) & 0xFFFFFFFF).random(n)
        m = r < percent / 100.0
    else:
        m = topo.boundary_points(geo) if dom == "point" else topo.boundary_prims(geo)
    out = groups.write(geo, dom, name, m, combine)
    if boundary_groups and method == "filters":
        out = edgelib.write(out, edgelib.PREFIX + _checked_name(name + "_boundary"),
                            _boundary_edge_mask(geo, dom, m).astype(np.int32))
    return out


def _checked_name(name):
    groups.attr_name(name)
    return name


def _write_group(geo, dom, name, m, combine):
    if dom != "edge":
        return groups.write(geo, dom, name, m, combine)
    key = edgelib.PREFIX + _checked_name(name)
    m = np.asarray(m, bool)
    old = edgelib.values(geo, key) != 0
    if combine == "union":
        m = m | old
    elif combine == "intersect":
        m = m & old
    elif combine == "subtract":
        m = old & ~m
    return edgelib.write(geo, key, m.astype(np.int32))


@node("group_promote", inputs=["geo"], category="group", params=[
    Str("name", "group1", doc="Group to convert (wildcards convert several)"),
    Enum("to", ["points", "primitives"], doc="Points of a primitive group: the points its primitives use. Primitives of "
                                             "a point group: those whose points are all in it"),
])
def group_promote(geo, name, to):
    """Convert the opposite domain to the target, replacing any same-name target group and removing the source."""
    dst = DOMAIN[to]
    src = "prim" if dst == "point" else "point"
    hits = [g for g in groups.names(geo, src) if fnmatch.fnmatchcase(g, name)]
    if not hits:
        raise ValueError(f"no {('primitive' if src == 'prim' else 'point')} group {name!r}; have "
                         f"{groups.names(geo, src) or 'none'}")
    out = geo
    for g in hits:
        m = groups.convert(geo, groups.member(geo, src, g), src, dst)
        out = groups.write(out, dst, g, m)
    return out


def _grow(geo, dom, mask, connect):
    """One step of growth: everything that touches a member (shares a point, or an edge for primitives)."""
    t = geo.topology
    vp = t.vertex_point.astype(np.int64)
    if dom == "point":
        e = topo.edges(geo).points
        out = mask.copy()
        out[e[mask[e[:, 1]], 0]] = True
        out[e[mask[e[:, 0]], 1]] = True
        return out
    pv = topo.prim_of_vertex(geo)
    if connect == "edges":
        ed = topo.edges(geo)
        ok = ed.of_vertex >= 0
        hit = np.zeros(len(ed.points), bool)
        hit[ed.of_vertex[ok & mask[pv]]] = True
        grown = np.zeros(t.nprims, bool)
        grown[pv[ok & hit[np.where(ok, ed.of_vertex, 0)]]] = True
        return grown | mask
    pt = np.zeros(geo.npoints, bool)
    pt[vp[mask[pv]]] = True
    grown = np.zeros(t.nprims, bool)
    grown[pv[pt[vp]]] = True
    return grown | mask


@node("group_expand", inputs=["geo"], category="group", params=[
    Str("group", "group1", doc="Group (or pattern) to grow or shrink"),
    Str("name", "", doc="Name for the result (empty: replace the group, if it is a plain group name)"),
    Enum("domain", ["primitives", "points"], doc="What the group holds"),
    Enum("mode", ["expand", "contract"], doc="expand: add the neighbours of members; contract: drop members that touch "
                                              "a non-member"),
    Int("steps", 1, min=1, max=1000, soft_max=10, doc="How many neighbour rings"),
    Enum("connectivity", ["points", "edges"], doc="primitives: neighbours share a point, or an edge"),
])
def group_expand(geo, group, name, domain, mode, steps, connectivity):
    """Grow or shrink a group by rings of neighbours (Houdini's Group Expand): select a face, expand once to get its
    border ring (then subtract the original), contract to pull a selection away from open edges and holes in it."""
    dom = DOMAIN[domain]
    n = geo.npoints if dom == "point" else geo.nprims
    m = groups.mask(geo, dom, group)
    m = np.ones(n, bool) if m is None else m.copy()
    for _ in range(int(steps)):
        if mode == "expand":
            m = _grow(geo, dom, m, connectivity)
        else:
            m = ~_grow(geo, dom, ~m, connectivity)
    target = name or (group if group.replace("_", "a").isalnum() and not group[0].isdigit() else "")
    if not target:
        raise ValueError("give the result a name: the group field is a pattern, not a group name")
    return groups.write(geo, dom, target, m)


def keep_prims(geo: Geometry, pkeep: np.ndarray, remove_unused_points: bool = True) -> Geometry:
    """Keep the primitives where ``pkeep`` is True (and, optionally, only the points they use)."""
    t = geo.topology
    pkeep = np.asarray(pkeep, bool)
    sizes = t.prim_sizes()
    vkeep = pkeep[topo.prim_of_vertex(geo)]
    vp = t.vertex_point[vkeep]
    offs = np.concatenate([[0], np.cumsum(sizes[pkeep])]).astype(np.int32)
    if remove_unused_points:
        used = np.zeros(geo.npoints, bool)
        used[vp] = True
        remap = (np.cumsum(used) - 1).astype(np.int32)
        out = Geometry(int(used.sum()), Topology(remap[vp], offs, B.closed_subset(geo, pkeep)))
        for name, a in geo.point.items():
            out.point[name] = a[used]
    else:
        out = Geometry(geo.npoints, Topology(vp.astype(np.int32), offs, B.closed_subset(geo, pkeep)))
        for name, a in geo.point.items():
            out.point[name] = a
    for name, a in geo.vertex.items():
        out.vertex[name] = a[vkeep]
    for name, a in geo.prim.items():
        out.prim[name] = a[pkeep]
    out.detail = dict(geo.detail)
    return out


@node("blast", inputs=["geo"], category="group", params=[
    Str("group", "", doc="What to delete: group names, ^exclusions, numbers/ranges, or an expression like '@P.z < 0'"),
    Enum("domain", ["primitives", "points", "edges"], doc="Delete faces, points, or faces touching selected edges"),
    Bool("delete_non_selected", False, doc="Keep the selection and delete everything else"),
    Bool("keep_unused_points", False, doc="primitives: keep points no remaining primitive uses"),
])
def blast(geo, group, domain, delete_non_selected, keep_unused_points):
    """Delete a selection of primitives or points (Houdini's Blast). Attributes and groups are compacted to match."""
    dom = DOMAIN[domain]
    if dom == "edge":
        selected = edgelib.mask(geo, group) if group.strip() else np.zeros(len(topo.edges(geo).points), bool)
        pairs = topo.edges(geo).points
        chosen = {tuple(sorted(e)) for e in pairs[selected]} if selected is not None else set()
        t = geo.topology
        touched = np.zeros(geo.nprims, bool)
        for i in range(geo.nprims):
            ps = t.vertex_point[t.prim_offsets[i]:t.prim_offsets[i+1]]
            count = len(ps) if t.closed_mask()[i] else max(0,len(ps)-1)
            touched[i] = any(tuple(sorted((int(ps[j]),int(ps[(j+1)%len(ps)])))) in chosen for j in range(count))
        return keep_prims(geo, touched if delete_non_selected else ~touched, remove_unused_points=not keep_unused_points)
    m = groups.mask(geo, dom, group)
    if m is None:                                            # empty field selects nothing (unlike most nodes)
        m = np.zeros(geo.npoints if dom == "point" else geo.nprims, bool)
    keep = m if delete_non_selected else ~m
    if keep.all():
        return geo
    if dom == "point":
        return keep_points(geo, keep)
    return keep_prims(geo, keep, remove_unused_points=not keep_unused_points)
