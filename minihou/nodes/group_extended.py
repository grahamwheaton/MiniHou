"""Additional Houdini-style group operations over MiniHou polygon geometry.

These nodes deliberately expose only the documented controls which have a
meaningful implementation here. In particular, ordered Houdini groups and
surface-geodesic closest-element queries are not represented by our geometry.
"""
from __future__ import annotations

import fnmatch
import heapq
import re

import numpy as np

from .. import spatial, topo
from ..sdk import Bool, Enum, Float, Str, node
from . import group_rules as rules


def _source_names(geo, domain, pattern):
    tokens = (pattern or "*").replace(",", " ").split()
    return [name for name in rules.names(geo, domain)
            if any(fnmatch.fnmatchcase(name, token) for token in tokens)]


def _target_name(geo, domain, proposed, conflict):
    existing = set(rules.names(geo, domain))
    if proposed not in existing:
        return proposed
    if conflict == "skip":
        return None
    if conflict == "overwrite":
        return proposed
    if conflict == "suffix":
        suffix = 2
        while f"{proposed}{suffix}" in existing:
            suffix += 1
        return f"{proposed}{suffix}"
    raise ValueError(f"unsupported group name conflict mode {conflict!r}")


def _attribute_match(source, destination, domain, name):
    if not name:
        return np.arange(rules.count(destination, domain), dtype=np.int64)
    a = source.table(domain).get(name)
    b = destination.table(domain).get(name)
    if a is None or b is None:
        raise ValueError(f"{domain} match attribute {name!r} must exist on both inputs")
    if a.ndim != 1 or b.ndim != 1 or a.dtype.kind not in "iu" or b.dtype.kind not in "iu":
        raise ValueError(f"{domain} match attribute {name!r} must be a scalar integer on both inputs")
    first = {}
    for i, value in enumerate(a.tolist()):
        first.setdefault(value, i)
    return np.asarray([first.get(value, -1) for value in b.tolist()], np.int64)


@node("group_copy", inputs=["geo", "source"], category="group", params=[
    Str("point_groups", "*"), Str("point_prefix", ""), Str("point_match_attribute", ""),
    Str("prim_groups", "*"), Str("prim_prefix", ""), Str("prim_match_attribute", ""),
    Str("edge_groups", "*"), Str("edge_prefix", ""),
    Str("vertex_groups", "*"), Str("vertex_prefix", ""), Str("vertex_match_attribute", ""),
    Enum("conflict", ["overwrite", "skip", "suffix"]),
    Bool("copy_empty", True),
])
def group_copy(geo, source, point_groups, point_prefix, point_match_attribute,
               prim_groups, prim_prefix, prim_match_attribute, edge_groups, edge_prefix,
               vertex_groups, vertex_prefix, vertex_match_attribute, conflict, copy_empty):
    """Copy groups by element number, or by equal scalar point/prim/vertex attribute."""
    out = geo
    configs = (("point", point_groups, point_prefix, point_match_attribute),
               ("prim", prim_groups, prim_prefix, prim_match_attribute),
               ("edge", edge_groups, edge_prefix, ""),
               ("vertex", vertex_groups, vertex_prefix, vertex_match_attribute))
    for domain, pattern, prefix, matching in configs:
        src_n = rules.count(source, domain)
        dst_n = rules.count(geo, domain)
        indices = _attribute_match(source, geo, domain, matching) if matching else np.arange(dst_n, dtype=np.int64)
        valid = (indices >= 0) & (indices < src_n)
        for name in _source_names(source, domain, pattern):
            proposed = prefix + name
            target = _target_name(out, domain, proposed, conflict)
            if target is None:
                continue
            membership = rules.membership(source, domain, name)
            selected = np.zeros(dst_n, bool)
            selected[valid] = membership[indices[valid]]
            if selected.any() or copy_empty:
                out = rules.write(out, domain, target, selected)
    return out


@node("group_transfer", inputs=["geo", "source"], category="group", params=[
    Str("point_groups", "*"), Str("point_prefix", ""),
    Str("prim_groups", "*"), Str("prim_prefix", ""),
    Str("edge_groups", "*"), Str("edge_prefix", ""),
    Enum("conflict", ["overwrite", "skip", "suffix"]),
    Float("distance_threshold", 1e9, min=0.0), Bool("create_empty", True),
])
def group_transfer(geo, source, point_groups, point_prefix, prim_groups, prim_prefix,
                   edge_groups, edge_prefix, conflict, distance_threshold, create_empty):
    """Transfer groups by nearest point, primitive centroid, or edge midpoint.

    Centroids/midpoints are a deliberate mesh approximation to Houdini's
    closest-surface queries and are described in the node documentation.
    """
    out = geo
    for domain, pattern, prefix in (("point", point_groups, point_prefix),
                                    ("prim", prim_groups, prim_prefix),
                                    ("edge", edge_groups, edge_prefix)):
        names = _source_names(source, domain, pattern)
        if not names:
            continue
        src_pos = rules.representative_positions(source, domain)
        dst_pos = rules.representative_positions(geo, domain)
        indices, distance = spatial.nearest(src_pos, dst_pos)
        valid = (indices >= 0) & (distance <= distance_threshold)
        for name in names:
            target = _target_name(out, domain, prefix + name, conflict)
            if target is None:
                continue
            membership = rules.membership(source, domain, name)
            selected = np.zeros(len(dst_pos), bool)
            selected[valid] = membership[indices[valid]]
            if selected.any() or create_empty:
                out = rules.write(out, domain, target, selected)
    return out


def _point_adjacency(geo):
    e = topo.edges(geo).points
    adjacent = [[] for _ in range(geo.npoints)]
    for a, b in e:
        a, b = int(a), int(b)
        if a == b:
            continue
        w = float(np.linalg.norm(geo.P[a].astype(np.float64) - geo.P[b]))
        adjacent[a].append((b, w))
        adjacent[b].append((a, w))
    for row in adjacent:
        row.sort()
    return adjacent


def _edge_adjacency(geo):
    edges = topo.edges(geo).points
    at_point = [[] for _ in range(geo.npoints)]
    for i, (a, b) in enumerate(edges):
        at_point[int(a)].append(i)
        at_point[int(b)].append(i)
    midpoint = rules.representative_positions(geo, "edge")
    adjacent = [[] for _ in range(len(edges))]
    for row in at_point:
        for i in row:
            for j in row:
                if i != j:
                    adjacent[i].append((j, float(np.linalg.norm(midpoint[i] - midpoint[j]))))
    return [sorted(set(row)) for row in adjacent]


def _ordered_elements(geo, domain, pattern):
    # Group storage is a bool mask, so explicitly listed numeric tokens retain
    # order, while a named group is visited in element-number order.
    tokens = (pattern or "").replace(",", " ").split()
    if not tokens:
        raise ValueError("base_group needs at least two elements")
    if all(re.fullmatch(r"\d+", token) for token in tokens):
        indices = [int(token) for token in tokens]
        if any(i >= rules.count(geo, domain) for i in indices):
            raise ValueError("base_group contains an element outside the geometry")
        return indices
    return np.flatnonzero(rules.select(geo, domain, pattern)).tolist()


def _shortest_path(adjacent, start, end, forbidden):
    if start == end:
        return [start]
    if forbidden[start] or forbidden[end]:
        raise ValueError("path endpoint is in collision_group")
    # Equal distances choose fewer hops, then lower element numbers.
    queue = [(0.0, 0, (start,), start)]
    best = {start: (0.0, 0, (start,))}
    while queue:
        distance, hops, path, u = heapq.heappop(queue)
        if best.get(u) != (distance, hops, path):
            continue
        if u == end:
            return list(path)
        for v, length in adjacent[u]:
            if forbidden[v]:
                continue
            candidate = (distance + length, hops + 1, path + (v,))
            if v not in best or candidate < best[v]:
                best[v] = candidate
                heapq.heappush(queue, (*candidate, v))
    raise ValueError(f"no path between elements {start} and {end}")


@node("group_find_path", inputs=["geo"], category="group", params=[
    Str("name", "path"), Str("base_group", ""),
    Enum("domain", ["points", "edges"]),
    Enum("path_type", ["ordered", "pairs"]),
    Str("collision_group", ""),
])
def group_find_path(geo, name, base_group, domain, path_type, collision_group):
    """Connect ordered point/edge elements by shortest topological paths.

    Supports paths through each element and start/end pairs. Houdini's loop,
    ring, turn-minimizing, UV, and cross-domain collision modes are outside the
    represented subset and therefore absent from this node's schema.
    """
    dom = rules.domain_name(domain)
    sequence = _ordered_elements(geo, dom, base_group)
    if len(sequence) < 2:
        raise ValueError("base_group needs at least two elements")
    if path_type == "pairs" and len(sequence) % 2:
        raise ValueError("pair paths need an even number of base elements")
    adjacent = _point_adjacency(geo) if dom == "point" else _edge_adjacency(geo)
    forbidden = (rules.select(geo, dom, collision_group) if collision_group else
                 np.zeros(rules.count(geo, dom), bool))
    pairs = (list(zip(sequence[::2], sequence[1::2])) if path_type == "pairs" else
             list(zip(sequence[:-1], sequence[1:])))
    selected = np.zeros(rules.count(geo, dom), bool)
    for start, end in pairs:
        selected[_shortest_path(adjacent, start, end, forbidden)] = True
    return rules.write(geo, dom, name, selected)


def _different(a, b, tolerance):
    a, b = np.asarray(a), np.asarray(b)
    if a.dtype.kind in "fiu" and b.dtype.kind in "fiu":
        return not bool(np.all(np.abs(a.astype(np.float64) - b.astype(np.float64)) <= tolerance))
    return not bool(np.array_equal(a, b))


def _edge_sides(geo):
    edges = topo.edges(geo)
    prim = topo.prim_of_vertex(geo)
    sides = [[] for _ in range(len(edges.points))]
    for vertex, edge in enumerate(edges.of_vertex):
        if edge >= 0:
            sides[int(edge)].append((int(prim[vertex]), vertex))
    return edges, sides


@node("group_from_attribute_boundary", inputs=["geo"], category="group", params=[
    Str("name", "boundary"), Enum("domain", ["edges", "points", "primitives", "vertices"]),
    Str("base_group", ""), Enum("base_domain", ["primitives", "points", "edges", "vertices"]),
    Str("attribute", ""), Enum("attribute_domain", ["primitives", "points", "vertices"]),
    Float("tolerance", 0.0, min=0.0), Bool("include_unshared_edges", True),
    Bool("include_all_curve_edges", False),
])
def group_from_attribute_boundary(geo, name, domain, base_group, base_domain,
                                  attribute, attribute_domain, tolerance, include_unshared_edges,
                                  include_all_curve_edges):
    """Find edges at attribute discontinuities and optionally open boundaries.

    Primitive attributes compare incident primitives. Vertex attributes compare
    both endpoint corner values on adjacent faces. Point attributes compare
    endpoint values across each edge. A base primitive group also marks the
    boundary between selected and unselected incident primitives.
    """
    out_dom = rules.domain_name(domain)
    attr_dom = rules.domain_name(attribute_domain)
    if not attribute and not base_group and not include_unshared_edges:
        raise ValueError("enable an attribute, base group, or unshared edges")
    values = None
    if attribute:
        values = geo.table(attr_dom).get(attribute)
        if values is None:
            raise ValueError(f"no {attr_dom} attribute {attribute!r}")
    edges, sides = _edge_sides(geo)
    selected = np.zeros(len(edges.points), bool)
    base = None
    if base_group:
        base = rules.select(geo, rules.domain_name(base_domain), base_group)
        base = rules.convert(geo, base, rules.domain_name(base_domain), "prim")
    next_v = topo.next_vertex(geo)
    for i, edge_sides in enumerate(sides):
        prims = [p for p, _ in edge_sides]
        if not prims:
            continue
        active = [base is None or bool(base[p]) for p in prims]
        if not any(active):
            continue
        if base is not None and any(active) and not all(active):
            selected[i] = True
        if include_unshared_edges and len(edge_sides) == 1:
            primitive = edge_sides[0][0]
            if geo.topology.closed_mask()[primitive] or include_all_curve_edges:
                selected[i] = True
        if values is None:
            continue
        if attr_dom == "point":
            a, b = edges.points[i]
            selected[i] |= _different(values[a], values[b], tolerance)
        elif attr_dom == "prim":
            for j in range(len(edge_sides)):
                for k in range(j + 1, len(edge_sides)):
                    if active[j] and active[k]:
                        selected[i] |= _different(values[prims[j]], values[prims[k]], tolerance)
        else:
            corners = []
            a, b = map(int, edges.points[i])
            for (_, vertex), is_active in zip(edge_sides, active):
                if not is_active:
                    continue
                nv = int(next_v[vertex])
                if int(geo.topology.vertex_point[vertex]) == a:
                    corners.append((values[vertex], values[nv]))
                else:
                    corners.append((values[nv], values[vertex]))
            for j in range(len(corners)):
                for k in range(j + 1, len(corners)):
                    selected[i] |= (_different(corners[j][0], corners[k][0], tolerance) or
                                    _different(corners[j][1], corners[k][1], tolerance))
    if base is not None:
        # The base selection limits the search without discarding its perimeter.
        selected &= np.array([any(base[p] for p, _ in row) for row in sides], bool)
    if out_dom == "vertex":
        result = np.zeros(geo.nvertices, bool)
        for edge_id in np.flatnonzero(selected):
            for _, vertex in sides[int(edge_id)]:
                result[vertex] = True
                result[int(next_v[vertex])] = True
    else:
        result = rules.convert(geo, selected, "edge", out_dom, contained=False)
    return rules.write(geo, out_dom, name, result)


_VALID_NAME = re.compile(r"^[A-Za-z_][A-Za-z0-9_]*$")


def _force_valid(name):
    name = re.sub(r"[^A-Za-z0-9_]", "_", name)
    return name if re.match(r"^[A-Za-z_]", name) else "_" + name


@node("groups_from_name", inputs=["geo"], category="group", params=[
    Str("attribute", "name"), Enum("domain", ["primitives", "points", "vertices"]),
    Str("prefix", ""), Enum("conflict", ["replace", "union"]),
    Enum("invalid_names", ["ignore", "force_valid"]),
])
def groups_from_name(geo, attribute, domain, prefix, conflict, invalid_names):
    """Create groups from name codes and a detail ``<attribute>_names`` table.

    MiniHou attributes are numeric. The detail table carries the string values;
    the per-element integer attribute indexes that table. Negative/out-of-range
    codes and empty string entries are ignored.
    """
    dom = rules.domain_name(domain)
    values = geo.table(dom).get(attribute)
    if values is None:
        raise ValueError(f"no {dom} name attribute {attribute!r}")
    names = geo.detail.get(attribute + "_names")
    if values.ndim != 1 or values.dtype.kind not in "iu" or not isinstance(names, (list, tuple)) or any(not isinstance(x, str) for x in names):
        raise ValueError(f"{dom} name attribute {attribute!r} needs integer codes and a detail {attribute + '_names'!r} string list")
    buckets = {}
    for index, code in enumerate(values):
        value = names[int(code)] if 0 <= int(code) < len(names) else ""
        if not value:
            continue
        name = prefix + value
        if not _VALID_NAME.fullmatch(name):
            if invalid_names == "ignore":
                continue
            name = _force_valid(name)
        buckets.setdefault(name, []).append(index)
    out = geo
    for name, indices in buckets.items():
        selected = np.zeros(rules.count(geo, dom), bool)
        selected[indices] = True
        out = rules.write(out, dom, name, selected, conflict)
    return out
