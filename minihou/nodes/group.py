"""Groups: create, convert and delete by group (Houdini's Group Create, Group Promote, Blast)."""
import fnmatch

import numpy as np

from .. import groups, topo
from ..geometry import Geometry, Topology
from ..sdk import Bool, Enum, Float, Int, Str, Vec3, node
from . import _build as B
from .delete import keep_points
from .normal import prim_normals
from ._util import normalize_rows

DOMAIN = {"points": "point", "primitives": "prim"}


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


@node("group_create", inputs=["geo"], category="group", params=[
    Str("name", "group1", doc="Group name; wrangles read it as @group_<name>"),
    Enum("domain", ["points", "primitives"], doc="What the group holds"),
    Enum("method", ["pattern", "bounding_box", "normal", "random", "boundary"],
         doc="pattern: groups, ranges or an expression like '@P.z > 1'; bounding_box: inside a box; normal: facing a "
             "direction; random: a percentage; boundary: on open edges"),
    Str("pattern", "*", doc="pattern method: group names, ^exclusions, numbers/ranges ('0-9'), or '@P.z > 1'"),
    Vec3("box_center", (0.0, 0.0, 0.0), doc="bounding_box method"),
    Vec3("box_size", (1.0, 1.0, 1.0), doc="bounding_box method"),
    Vec3("direction", (0.0, 0.0, 1.0), doc="normal method: elements whose normal is within 'angle' of this"),
    Float("angle", 45.0, min=0.0, max=180.0, doc="normal method, in degrees"),
    Float("percent", 50.0, min=0.0, max=100.0, doc="random method"),
    Int("seed", 0, doc="random method"),
    Enum("combine", ["replace", "union", "intersect", "subtract"], doc="How to merge with an existing group of the name"),
])
def group_create(geo, name, domain, method, pattern, box_center, box_size, direction, angle, percent, seed, combine):
    """Make a named group of points or primitives (stored as the int attribute group_<name>, 1 = member). Most nodes
    take a group field, and Blender gets each group as a boolean attribute."""
    dom = DOMAIN[domain]
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
    return groups.write(geo, dom, name, m, combine)


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
    Enum("domain", ["primitives", "points"], doc="Delete primitives, or points (and the primitives using them)"),
    Bool("delete_non_selected", False, doc="Keep the selection and delete everything else"),
    Bool("keep_unused_points", False, doc="primitives: keep points no remaining primitive uses"),
])
def blast(geo, group, domain, delete_non_selected, keep_unused_points):
    """Delete a selection of primitives or points (Houdini's Blast). Attributes and groups are compacted to match."""
    dom = DOMAIN[domain]
    m = groups.mask(geo, dom, group)
    if m is None:                                            # empty field selects nothing (unlike most nodes)
        m = np.zeros(geo.npoints if dom == "point" else geo.nprims, bool)
    keep = m if delete_non_selected else ~m
    if keep.all():
        return geo
    if dom == "point":
        return keep_points(geo, keep)
    return keep_prims(geo, keep, remove_unused_points=not keep_unused_points)
