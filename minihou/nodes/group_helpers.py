"""Group by Cd Colour, normal direction, and proximity.

All three nodes select elements on the requested domain, optionally restrict that
selection to a base group, then apply the usual group merge operation.  They do
not change geometry, topology, or any attribute other than the result group.
"""
from __future__ import annotations

import numpy as np

from .. import spatial, topo
from ..sdk import Bool, Enum, Float, Input, Str, Vec3, node
from . import group_rules as rules
from .model import triangulate
from .normal import prim_normals


_DOMAINS = ["points", "primitives", "edges", "vertices"]
_COMBINE = ["replace", "union", "intersect", "subtract"]
_DIST_EPS = 1e-6  # point positions are stored as float32


def _start(geo, domain):
    dom = rules.domain_name(domain)
    return dom, rules.count(geo, dom)


def _finish(geo, dom, name, matches, base_group, invert, combine):
    matches = np.asarray(matches, bool)
    if invert:
        matches = ~matches
    if base_group.strip():
        matches &= rules.select(geo, dom, base_group)
    return rules.write(geo, dom, name, matches, combine)


def _rgb(a, attribute, source):
    if a is None:
        raise ValueError(f"no {source} colour attribute {attribute!r}")
    a = np.asarray(a)
    if a.ndim != 2 or a.shape[1] < 3 or not np.issubdtype(a.dtype, np.number):
        raise ValueError(f"{source} colour attribute {attribute!r} must have at least three numeric components")
    rgb = a[:, :3].astype(np.float64)
    if not np.isfinite(rgb).all():
        raise ValueError(f"{source} colour attribute {attribute!r} contains non-finite values")
    return rgb


def _mean_rows(rows, buckets, count):
    """Mean RGB/vector rows by element, with zero for an empty element."""
    sizes = np.maximum(np.bincount(buckets, minlength=count), 1)
    return np.stack([np.bincount(buckets, weights=rows[:, c], minlength=count)
                     for c in range(3)], axis=1) / sizes[:, None]


def _colors(geo, dom, attribute):
    """Read native Cd; fall back to point Cd for corners, faces and edges."""
    if dom == "point":
        a = geo.point.get(attribute)
        if a is not None:
            return _rgb(a, attribute, "point")
        a = geo.vertex.get(attribute)
        if a is not None:
            return _mean_rows(_rgb(a, attribute, "vertex"), geo.topology.vertex_point, geo.npoints)
        raise ValueError(f"no point or vertex colour attribute {attribute!r}")
    if dom == "vertex":
        a = geo.vertex.get(attribute)
        if a is not None:
            return _rgb(a, attribute, "vertex")
        return _rgb(geo.point.get(attribute), attribute, "point")[geo.topology.vertex_point]
    if dom == "prim":
        a = geo.prim.get(attribute)
        if a is not None:
            return _rgb(a, attribute, "primitive")
        point = _rgb(geo.point.get(attribute), attribute, "point")
        return _mean_rows(point[geo.topology.vertex_point], topo.prim_of_vertex(geo), geo.nprims)
    edge = topo.edges(geo)
    a = geo.vertex.get(attribute)
    if a is not None:
        valid = edge.of_vertex >= 0
        return _mean_rows(_rgb(a, attribute, "vertex")[valid], edge.of_vertex[valid], len(edge.points))
    point = _rgb(geo.point.get(attribute), attribute, "point")
    return point[edge.points].mean(axis=1)


@node("group_by_color", inputs=["geo"], category="group", params=[
    Str("name", "group1", doc="Result group name"),
    Enum("domain", _DOMAINS, doc="Elements to group"),
    Str("base_group", "", kind="group", doc="Optional group pattern limiting candidate elements"),
    Str("attribute", "Cd", doc="RGB colour attribute; native domain first, then point/vertex colour where applicable"),
    Vec3("color", (1.0, 1.0, 1.0), label="Colour", doc="Target RGB colour"),
    Float("tolerance", 0.01, min=0.0, doc="Maximum Euclidean RGB distance from Colour, inclusive"),
    Bool("invert", False, doc="Select non-matching elements within the base group"),
    Enum("combine", _COMBINE, doc="Merge with a pre-existing result group"),
])
def group_by_color(geo, name, domain, base_group, attribute, color, tolerance, invert, combine):
    """Select by Cd colour; missing or malformed colour attributes fail clearly."""
    dom, n = _start(geo, domain)
    if n == 0:
        return _finish(geo, dom, name, np.zeros(0, bool), base_group, invert, combine)
    rgb = _colors(geo, dom, attribute.strip())
    delta = rgb - np.asarray(color, np.float64)
    matches = np.einsum("ij,ij->i", delta, delta) <= float(tolerance) ** 2 + 1e-12
    return _finish(geo, dom, name, matches, base_group, invert, combine)


def _unit(rows, source):
    rows = np.asarray(rows, np.float64)
    if rows.ndim != 2 or rows.shape[1] != 3:
        raise ValueError(f"{source} N attribute must have three components")
    if not np.isfinite(rows).all():
        raise ValueError(f"{source} N attribute contains non-finite values")
    lengths = np.linalg.norm(rows, axis=1)
    return rows / np.maximum(lengths[:, None], 1e-300), lengths > 1e-12


def _normals(geo, dom):
    """Existing N wins; otherwise compute face, point, corner, or edge normals."""
    if dom == "point" and "N" in geo.point:
        return _unit(geo.point["N"], "point")
    if dom == "prim" and "N" in geo.prim:
        return _unit(geo.prim["N"], "primitive")
    if dom == "vertex":
        if "N" in geo.vertex:
            return _unit(geo.vertex["N"], "vertex")
        if "N" in geo.point:
            return _unit(geo.point["N"][geo.topology.vertex_point], "point")
    edge = None
    if dom == "edge":
        edge = topo.edges(geo)
        if "N" in geo.vertex:
            valid = edge.of_vertex >= 0
            vertex_n, _ = _unit(geo.vertex["N"], "vertex")
            rows = _mean_rows(vertex_n[valid],
                              edge.of_vertex[valid], len(edge.points))
            return _unit(rows, "vertex")
        if "N" in geo.point:
            point_n, _ = _unit(geo.point["N"], "point")
            return _unit(point_n[edge.points].mean(axis=1), "point")
    faces = np.asarray(prim_normals(geo), np.float64)
    if geo.topology.has_open:
        faces = faces.copy()
        faces[~geo.topology.closed_mask()] = 0.0  # an open curve has no oriented face
    if dom == "prim":
        return _unit(faces, "computed primitive")
    pv = topo.prim_of_vertex(geo)
    if dom == "vertex":
        return _unit(faces[pv], "computed vertex")
    if dom == "point":
        return _unit(topo.segment_sum(faces[pv], geo.topology.vertex_point, geo.npoints), "computed point")
    valid = edge.of_vertex >= 0
    face_n, _ = _unit(faces, "computed primitive")
    return _unit(_mean_rows(face_n[pv[valid]], edge.of_vertex[valid], len(edge.points)), "computed edge")


@node("group_by_normal", inputs=["geo"], category="group", params=[
    Str("name", "group1", doc="Result group name"),
    Enum("domain", _DOMAINS, doc="Elements to group"),
    Str("base_group", "", kind="group", doc="Optional group pattern limiting candidate elements"),
    Vec3("direction", (0.0, 0.0, 1.0), doc="Reference direction; need not be unit length"),
    Float("angle", 45.0, min=0.0, max=180.0, doc="Maximum facing angle in degrees, inclusive"),
    Bool("both_sides", False, doc="Also match normals facing the opposite direction"),
    Bool("invert", False, doc="Select non-matching elements within the base group"),
    Enum("combine", _COMBINE, doc="Merge with a pre-existing result group"),
])
def group_by_normal(geo, name, domain, base_group, direction, angle, both_sides, invert, combine):
    """Select elements facing a direction; zero normals never face any direction."""
    dom, n = _start(geo, domain)
    d = np.asarray(direction, np.float64)
    length = np.linalg.norm(d)
    if length <= 1e-12:
        raise ValueError("direction must be nonzero")
    if n == 0:
        return _finish(geo, dom, name, np.zeros(0, bool), base_group, invert, combine)
    N, valid = _normals(geo, dom)
    dot = N @ (d / length)
    threshold = np.cos(np.radians(angle)) - 1e-9
    matches = valid & (dot >= threshold)
    if both_sides:
        matches |= valid & (-dot >= threshold)
    return _finish(geo, dom, name, matches, base_group, invert, combine)


def _segment_hits(query, a, b, radius):
    """Exact point-to-segment threshold using a bounding-box hierarchy.

    Pruning by segment bounds keeps sparse curve searches near logarithmic in
    the number of segments, without requiring SciPy or allocating a full
    query-by-segment distance array.
    """
    result = np.zeros(len(query), bool)
    if not len(query) or not len(a):
        return result
    a, b = np.asarray(a, np.float64), np.asarray(b, np.float64)
    lo, hi = np.minimum(a, b), np.maximum(a, b)
    midpoints = (a + b) * .5
    order = np.arange(len(a))
    nodes = []

    def build(start, end):
        ids = order[start:end]
        box_lo, box_hi = lo[ids].min(axis=0), hi[ids].max(axis=0)
        node_id = len(nodes)
        nodes.append((box_lo, box_hi, -1, -1, start, end))
        if end - start > 24:
            axis = int(np.argmax(box_hi - box_lo))
            order[start:end] = ids[np.argsort(midpoints[ids, axis], kind="stable")]
            half = (start + end) // 2
            left, right = build(start, half), build(half, end)
            nodes[node_id] = (box_lo, box_hi, left, right, start, end)
        return node_id

    build(0, len(a))
    radius2 = radius * radius
    ab = b - a
    length2 = np.maximum(np.einsum("ij,ij->i", ab, ab), 1e-300)
    for qid, q in enumerate(query):
        stack = [0]
        while stack:
            box_lo, box_hi, left, right, start, end = nodes[stack.pop()]
            gap = np.maximum(np.maximum(box_lo - q, q - box_hi), 0)
            if np.dot(gap, gap) > radius2:
                continue
            if left >= 0:
                stack.extend((left, right))
                continue
            ids = order[start:end]
            delta = q - a[ids]
            t = np.clip(np.einsum("ij,ij->i", delta, ab[ids]) / length2[ids], 0.0, 1.0)
            rest = delta - t[:, None] * ab[ids]
            if np.any(np.einsum("ij,ij->i", rest, rest) <= radius2):
                result[qid] = True
                break
    return result


def _surface_triangles(target):
    """Actual nondegenerate polygon triangles, including already triangular faces."""
    closed = target.topology.closed_mask()
    sizes = target.topology.prim_sizes()
    tri, _, _ = triangulate(target, closed)
    already = np.flatnonzero(closed & (sizes == 3))
    if len(already):
        first = target.topology.prim_offsets[already].astype(np.int64)
        single = first[:, None] + np.arange(3)
        tri = np.concatenate((tri, single)) if len(tri) else single
    if not len(tri):
        return np.zeros((0, 3, 3), np.float64)
    P = np.asarray(target.P, np.float64)
    triangles = P[target.topology.vertex_point[tri]]
    area2 = np.linalg.norm(np.cross(triangles[:, 1] - triangles[:, 0],
                                    triangles[:, 2] - triangles[:, 0]), axis=1)
    return triangles[area2 > 1e-20]


def _surface_matches(query, target, radius, triangles=None):
    if len(query) == 0 or target.npoints == 0:
        return np.zeros(len(query), bool)
    P = np.asarray(target.P, np.float64)
    if triangles is None:
        triangles = _surface_triangles(target)
    closed = target.topology.closed_mask()
    matches = np.zeros(len(query), bool)
    if len(triangles):
        from ..raycast import TriGrid
        closest, _, _ = TriGrid(triangles).closest(query)
        delta = closest - query
        matches = np.einsum("ij,ij->i", delta, delta) <= radius * radius
    edges = topo.edges(target)
    open_prims = ~closed
    has_segments = False
    if open_prims.any():
        pv = topo.prim_of_vertex(target)
        valid = (edges.of_vertex >= 0) & open_prims[pv]
        ids = np.unique(edges.of_vertex[valid])
        if len(ids):
            has_segments = True
            pairs = edges.points[ids]
            remaining = ~matches
            matches[remaining] = _segment_hits(query[remaining], P[pairs[:, 0]], P[pairs[:, 1]], radius)
    if not len(triangles) and not has_segments:
        raise ValueError("surface target has no polygon faces or curve segments; use target_mode='points'")
    return matches


@node("group_by_proximity", inputs=["geo", Input("target", optional=True,
      doc="Optional reference geometry; auto mode uses its surface or points")], category="group", params=[
    Str("name", "group1", doc="Result group name"),
    Enum("domain", _DOMAINS, doc="Elements to group; faces use centroids and edges use midpoints"),
    Str("base_group", "", kind="group", doc="Optional group pattern limiting candidate elements"),
    Enum("target_mode", ["auto", "position", "points", "surface"],
         doc="Auto: connected target surface when present, otherwise its points; no target: Position. "
             "Points measures to target vertices; surface measures to actual polygons/curve segments"),
    Vec3("position", (0.0, 0.0, 0.0), doc="Reference position when target_mode is position or no target is connected"),
    Float("distance", 1.0, min=0.0, doc="Maximum Euclidean distance, inclusive"),
    Bool("invert", False, doc="Select non-matching elements within the base group"),
    Enum("combine", _COMBINE, doc="Merge with a pre-existing result group"),
])
def group_by_proximity(geo, target=None, *, name, domain, base_group, target_mode, position, distance, invert, combine):
    """Select representatives near a position, target points, or its actual surface."""
    dom, n = _start(geo, domain)
    if n == 0:
        return _finish(geo, dom, name, np.zeros(0, bool), base_group, invert, combine)
    query = np.asarray(rules.representative_positions(geo, dom), np.float64)
    mode = target_mode
    triangles = None
    if mode == "auto":
        if target is None:
            mode = "position"
        else:
            sizes = target.topology.prim_sizes()
            closed = target.topology.closed_mask()
            if np.any(closed & (sizes >= 3)):
                triangles = _surface_triangles(target)
            has_faces = triangles is not None and len(triangles) > 0
            has_segments = bool(np.any(~closed & (sizes >= 2)))
            mode = "surface" if has_faces or has_segments else "points"
    if mode == "position":
        distances = np.linalg.norm(query - np.asarray(position, np.float64), axis=1)
        matches = distances <= float(distance) + _DIST_EPS
    elif target is None:
        raise ValueError(f"target_mode={mode!r} requires a connected target geometry")
    elif mode == "points":
        _, distances = spatial.nearest(target.P, query)
        matches = distances <= float(distance) + _DIST_EPS
    else:
        matches = _surface_matches(query, target, float(distance) + _DIST_EPS, triangles)
    return _finish(geo, dom, name, matches, base_group, invert, combine)
