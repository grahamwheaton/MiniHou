"""Houdini modelling tools that the "C" pie menu offers and that had no node yet: Null, Twist, Linear Taper, Squash and
Stretch, Soft Peak, Group by Range, Edge Collapse, Edge Flip, Edge Cusp and PolySplit."""
import numpy as np

from .. import edges, groups, spatial, topo
from ..outgroups import OutGroup, OutGroups, edge_mask
from ..sdk import Bool, Enum, Float, Guide, Handle, Int, Str, Vec3, node
from . import _build as B
from . import group_rules as group_rules
from ._util import normalize_rows
from .bevel import twins
from .capture_deform import _apply
from .edge_tools import _rebuild
from .normal import normal, prim_normals


def _unit(v, what="direction"):
    v = np.asarray(v, np.float64)
    n = float(np.linalg.norm(v))
    if n < 1e-12:
        raise ValueError(f"{what} must not be the zero vector")
    return v / n


def _frame(d, up):
    """Orthonormal (x, y, d): d the axis, x and y perpendicular to it (y as close to ``up`` as possible)."""
    d = _unit(d)
    up = np.asarray(up, np.float64)
    y = up - d * float(up @ d)
    if np.linalg.norm(y) < 1e-9:                                  # up parallel to the axis: any perpendicular will do
        y = np.cross(d, [1.0, 0.0, 0.0] if abs(d[0]) < 0.9 else [0.0, 1.0, 0.0])
    y = y / np.linalg.norm(y)
    return np.cross(y, d), y, d


@node("null", inputs=["geo"], category="utility", params=[
    Str("label", "", doc="A note for yourself; the geometry passes through untouched"),
])
def null(geo, label=""):
    """Pass the input through unchanged (Houdini's Null): a named place to read or output from."""
    return geo


@node("twist", inputs=["geo"], category="deform", params=[
    Float("strength", 90.0, soft_min=-360.0, soft_max=360.0, doc="Degrees of rotation per unit along the axis"),
    Vec3("origin", (0.0, 0.0, 0.0), doc="The twist is zero here"),
    Vec3("direction", (0.0, 1.0, 0.0), doc="Twist axis"),
    Str("group", "", doc="Only these points"),
], handles=[Handle("strength", kind="ring", axis="@direction", at="@origin")], guides=[Guide("line", at="@origin", axis="@direction", id="axis", label="twist axis")])

def twist(geo, strength, origin, direction, group):
    """Rotate points about an axis by an angle that grows with the distance along it (Houdini's Twist)."""
    d, o = _unit(direction), np.asarray(origin, np.float64)

    def mapping(P):
        r = P - o
        t = r @ d
        a = np.radians(strength) * t
        c, s = np.cos(a)[:, None], np.sin(a)[:, None]
        return o + r * c + np.cross(d, r) * s + d * (t[:, None] * (1 - c))          # Rodrigues
    return _apply(geo, mapping, group)


@node("linear_taper", inputs=["geo"], category="deform", params=[
    Float("taper_x", 0.5, soft_min=0.0, soft_max=2.0, doc="Scale across the first perpendicular axis at the end of the length"),
    Float("taper_y", 0.5, soft_min=0.0, soft_max=2.0, doc="Scale across the second perpendicular axis at the end"),
    Float("length", 1.0, min=1e-8, doc="Distance along the axis over which the taper is reached"),
    Float("dropoff", 1.0, min=0.01, soft_max=4.0, doc="1 is linear; larger holds the shape longer then tapers faster"),
    Vec3("origin", (0.0, 0.0, 0.0), doc="No scaling at this point"),
    Vec3("direction", (0.0, 1.0, 0.0), doc="Taper axis"),
    Vec3("up", (0.0, 0.0, 1.0), doc="Orients the two perpendicular axes"),
    Bool("symmetric", False, doc="Taper the same way on the other side of the origin"),
    Str("group", "", doc="Only these points"),
], handles=[Handle("length", axis="@direction", at="@origin", reach=True, min=1e-6)], guides=[Guide("line", at="@origin", axis="@direction", size="@length", id="axis", label="taper length"), Guide("plane", at="@origin", axis="@direction", id="start", label="taper start")])

def linear_taper(geo, taper_x, taper_y, length, dropoff, origin, direction, up, symmetric, group):
    """Scale points across an axis by an amount that blends from 1 at the origin to ``taper`` at ``length`` (Houdini's
    Linear Taper); past the length the scale keeps going."""
    x, y, d = _frame(direction, up)
    o = np.asarray(origin, np.float64)

    def mapping(P):
        r = P - o
        t = r @ d
        if symmetric:
            t = np.abs(t)
        u = np.maximum(t / length, 0.0) ** dropoff
        sx, sy = 1 + (taper_x - 1) * u, 1 + (taper_y - 1) * u
        return o + x * ((r @ x) * sx)[:, None] + y * ((r @ y) * sy)[:, None] + d * (r @ d)[:, None]
    return _apply(geo, mapping, group)


@node("squash_stretch", inputs=["geo"], category="deform", params=[
    Float("amount", 1.5, min=0.01, soft_max=4.0, doc="Scale along the axis; 1 is unchanged, above 1 stretches, below 1 squashes"),
    Bool("preserve_volume", True, doc="Scale the other two axes by 1/sqrt(amount) so the volume stays the same"),
    Vec3("center", (0.0, 0.0, 0.0)),
    Vec3("direction", (0.0, 1.0, 0.0), doc="Squash axis"),
    Str("group", "", doc="Only these points"),
], handles=[Handle("amount", kind="scale", axis="@direction", at="@center")], guides=[Guide("line", at="@center", axis="@direction", id="axis", label="squash axis")])

def squash_stretch(geo, amount, preserve_volume, center, direction, group):
    """Scale along an axis and (optionally) compensate across it (Houdini's Squash and Stretch)."""
    d, c = _unit(direction), np.asarray(center, np.float64)
    side = 1.0 / np.sqrt(amount) if preserve_volume else 1.0

    def mapping(P):
        r = P - c
        a = (r @ d)[:, None] * d
        return c + a * amount + (r - a) * side
    return _apply(geo, mapping, group)


@node("soft_peak", inputs=["geo"], category="deform", params=[
    Float("distance", 0.3, soft_min=-2.0, soft_max=2.0, doc="How far the selection moves along its normals"),
    Float("radius", 1.0, min=1e-8, doc="Falloff distance from the group (empty group: everything moves fully)"),
    Enum("falloff", ["smooth", "linear", "constant"]),
    Str("group", "", doc="The points (or faces' points) that move fully; their neighbours follow within the radius"),
])
def soft_peak(geo, distance, radius, falloff, group):
    """Peak with a soft edge (Houdini's Soft Peak): the group moves along its normals by ``distance``, points within
    ``radius`` of it follow by a falling-off share. Falloff is straight-line distance."""
    sel = groups.mask(geo, "point", group)
    N = geo.point["N"] if "N" in geo.point else normal(geo, "point").point["N"]
    P = geo.P.astype(np.float64)
    if sel is None:
        w = np.ones(geo.npoints)
    elif not sel.any():
        return geo
    else:
        _, d = spatial.nearest(P[sel], P)
        u = np.clip(1 - d / radius, 0, 1)
        w = u * u * (3 - 2 * u) if falloff == "smooth" else (u if falloff == "linear" else (d <= radius).astype(float))
    out = geo.copy()
    out.point["P"] = (P + N.astype(np.float64) * (w * distance)[:, None]).astype(np.float32)
    return out


@node("group_range", inputs=["geo"], category="group", params=[
    Str("name", "group1"),
    Enum("domain", ["primitives", "points"]),
    Str("base_group", "", doc="Apply the range within this point or primitive selection"),
    Enum("range_type", ["absolute", "relative", "length", "equal_partitions"],
         doc="Element numbers, offsets from both ends, start plus length, or equal partitions"),
    Int("start", 0, min=0, doc="First element"),
    Int("end", -1, min=-1, doc="Last element (-1: the last one in the geometry)"),
    Int("length", 1, min=0, doc="Number of elements for Start and Length"),
    Int("partition", 0, min=0, doc="Zero-based partition for Equal Partitions"),
    Int("num_partitions", 2, min=1, doc="Number of equal partitions"),
    Int("select", 1, min=1, doc="Select this many ..."),
    Int("every", 1, min=1, doc="... out of every this many (1 of 2 is every other element)"),
    Int("offset", 0, doc="Shift the pattern along"),
    Bool("invert", False),
    Bool("disconnected_separately", False, doc="Restart the range independently on each connected region"),
    Enum("combine", ["replace", "union", "intersect", "subtract"]),
])
def group_range(geo, name, domain, base_group, range_type, start, end, length, partition,
                num_partitions, select, every, offset, invert, disconnected_separately, combine):
    """Group selected point/primitive ranges, optionally per disconnected region."""
    if not name:
        return geo
    dom = "point" if domain == "points" else "prim"
    n = geo.npoints if dom == "point" else geo.nprims
    selected = group_rules.select(geo, dom, base_group)
    m = np.zeros(n, bool)
    if disconnected_separately:
        components = _range_components(geo, dom, selected)
    else:
        components = [np.flatnonzero(selected)]
    for ids in components:
        k = len(ids)
        if not k:
            continue
        if range_type == "absolute":
            first, last = start, (k - 1 if end < 0 else min(end, k - 1))
        elif range_type == "relative":
            first, last = start, k - 1 - max(end, 0)
        elif range_type == "length":
            first, last = start, start + length - 1
        else:
            if partition >= num_partitions:
                continue
            first = (partition * k + num_partitions - 1) // num_partitions
            last = ((partition + 1) * k + num_partitions - 1) // num_partitions - 1
        i = np.arange(k)
        within = (i >= first) & (i <= last)
        within &= ((i - first + offset) % every) < min(select, every)
        if invert:
            within = ~within
        m[ids[within]] = True
    return group_rules.write(geo, dom, name, m, combine)


def _range_components(geo, dom, selected):
    """Selected point islands use edges; primitive islands share a full edge."""
    n = len(selected)
    adjacent = [[] for _ in range(n)]
    if dom == "point":
        for a, b in topo.edges(geo).points:
            a, b = int(a), int(b)
            adjacent[a].append(b)
            adjacent[b].append(a)
    else:
        edges = topo.edges(geo)
        prim = topo.prim_of_vertex(geo)
        at_edge = [[] for _ in range(len(edges.points))]
        for v, e in enumerate(edges.of_vertex):
            if e >= 0:
                at_edge[int(e)].append(int(prim[v]))
        for row in at_edge:
            for a in row:
                adjacent[a].extend(b for b in row if b != a)
    visited = np.zeros(n, bool)
    result = []
    for start in np.flatnonzero(selected):
        if visited[start]:
            continue
        stack = [int(start)]
        visited[start] = True
        region = []
        while stack:
            u = stack.pop()
            region.append(u)
            for v in adjacent[u]:
                if selected[v] and not visited[v]:
                    visited[v] = True
                    stack.append(v)
        result.append(np.asarray(sorted(region), np.int64))
    return result


# ---------------------------------------------------------------------------------------------------------- edges

COLLAPSE_OUT = OutGroups(OutGroup("collapsed", "Collapsed Points", "collapsedpoints", "point",
                                  doc="The points the selected edges were merged into"))


@node("edge_collapse", inputs=["geo"], category="model", params=[
    Str("group", "", doc="Edges to collapse: ids/ranges, p0-1 pairs, edge group names; empty does nothing"),
    *COLLAPSE_OUT.params(),
])
def edge_collapse(geo, group, **og):
    """Merge the two ends of every selected edge into one point at their average (Houdini's Edge Collapse). Chains of
    selected edges collapse to a single point; faces left with fewer than three corners disappear."""
    if not group.strip():
        return geo
    B.require_polygons(geo, "edge_collapse")
    m = edges.mask(geo, group)
    pairs = topo.edges(geo).points[m]
    if not len(pairs):
        return geo
    label = spatial.components(geo.npoints, pairs[:, 0], pairs[:, 1]).astype(np.int64)
    k = int(label.max()) + 1
    point = B.mean_by(geo.point, np.arange(geo.npoints), label, k)
    t = geo.topology
    out = B.make(point["P"], label[t.vertex_point], t.prim_offsets, point, dict(geo.vertex.items()),
                 dict(geo.prim.items()), geo.detail, t.closed)
    out = B.drop_degenerate(out)
    merged = np.bincount(label, minlength=k) > 1
    return COLLAPSE_OUT.write(out, og, "collapsed", lambda: merged)


FLIP_OUT = OutGroups(OutGroup("flipped", "Flipped Edges", "flippededges", "edge",
                              doc="The new diagonals"))


@node("edge_flip", inputs=["geo"], category="model", params=[
    Str("group", "", doc="Edges to flip: ids/ranges, p0-1 pairs, edge group names; empty does nothing"),
    *FLIP_OUT.params(),
])
def edge_flip(geo, group, **og):
    """Swap the diagonal of the two triangles on each selected edge (Houdini's Edge Flip). Edges that are not shared by
    exactly two triangles are left alone, and a triangle takes part in at most one flip."""
    if not group.strip():
        return geo
    B.require_polygons(geo, "edge_flip")
    t = geo.topology
    vp = t.vertex_point.astype(np.int64)
    nxt, prv = topo.next_vertex(geo), topo.prev_vertex(geo)
    pv = topo.prim_of_vertex(geo)
    tw = twins(geo)
    e = topo.edges(geo)
    chosen = edges.mask(geo, group)
    sizes = t.prim_sizes()
    used = np.zeros(t.nprims, bool)
    replace = {}                                               # prim -> its new corner rows
    new_pairs = []
    for i in np.flatnonzero((e.of_vertex >= 0) & chosen[np.maximum(e.of_vertex, 0)] & (tw >= 0)):
        j = tw[i]
        if j < 0 or j < i:
            continue
        p, q = pv[i], pv[j]
        if p == q or sizes[p] != 3 or sizes[q] != 3 or used[p] or used[q]:
            continue
        used[p] = used[q] = True
        c, d = prv[i], prv[j]
        if vp[c] == vp[d]:
            continue
        replace[p] = [i, d, c]                                 # a, d, c
        replace[q] = [d, nxt[i], c]                            # d, b, c
        new_pairs.append((vp[c], vp[d]))
    if not replace:
        return geo
    faces, sources = [], []
    for p in range(t.nprims):
        faces.append(replace.get(p, list(range(t.prim_offsets[p], t.prim_offsets[p + 1]))))
        sources.append(p)
    out = _rebuild(geo, faces, sources, t.closed)
    return FLIP_OUT.write(out, og, "flipped", lambda: edge_mask(out, new_pairs))


@node("edge_cusp", inputs=["geo"], category="model", params=[
    Str("group", "", doc="Edges to make hard: ids/ranges, p0-1 pairs, edge group names"),
    Float("cusp_angle", 180.0, min=0.0, max=180.0, doc="Edges between faces that meet at more than this many degrees are "
                                                       "hard as well (180: only the selected edges)"),
])
def edge_cusp(geo, group, cusp_angle):
    """Make the selected edges look hard and everything else smooth (Houdini's Edge Cusp): writes per-corner normals
    ``N`` that average across every edge except the cusped ones."""
    B.require_polygons(geo, "edge_cusp")
    t = geo.topology
    e = topo.edges(geo)
    hard = edges.mask(geo, group) if group.strip() else np.zeros(len(e.points), bool)
    fn = prim_normals(geo).astype(np.float64)
    unit = normalize_rows(fn).astype(np.float64)
    pv, vp = topo.prim_of_vertex(geo), t.vertex_point.astype(np.int64)
    nxt = topo.next_vertex(geo)
    tw = twins(geo)
    ok = (tw >= 0) & (e.of_vertex >= 0)
    i = np.flatnonzero(ok)
    j = tw[i]
    sharp = hard[e.of_vertex[i]] | (np.einsum("ij,ij->i", unit[pv[i]], unit[pv[j]]) < np.cos(np.radians(cusp_angle)) - 1e-9)
    i, j = i[~sharp], j[~sharp]
    # corner i (a->b) and the corner after the twin (b->a ... a) share point a; corner j and the corner after i share b
    a = np.concatenate([i, j])
    b = np.concatenate([nxt[j], nxt[i]])
    comp = spatial.components(t.nvertices, a, b)
    acc = np.stack([np.bincount(comp, weights=fn[pv, c], minlength=int(comp.max()) + 1 if len(comp) else 0)
                    for c in range(3)], 1)
    out = geo.copy()
    out.vertex["N"] = normalize_rows(acc)[comp] if len(comp) else np.zeros((0, 3), np.float32)
    for table in (out.point, out.prim):
        if "N" in table:
            del table["N"]
    return out


# ------------------------------------------------------------------------------------------------------ polysplit

SPLIT_OUT = OutGroups(
    OutGroup("points", "New Points", "splitpoints", "point", doc="The points inserted on the split edges"),
    OutGroup("edges", "New Edges", "splitedges", "edge", doc="The edges across the faces that were split"),
)


def _walk(edge_faces, quads, p, f, t, start_key):
    """From the edge (f, t) step into quad ``p``, and on across its opposite edge, until a non-quad, a boundary or the start.
    Returns the traversal and whether it came back to the start."""
    steps = []
    while True:
        if p not in quads:
            return steps, False, (f, t)
        q = quads[p]
        k = q.index(f)
        if q[(k + 1) % 4] == t:
            f2, t2 = q[(k + 3) % 4], q[(k + 2) % 4]
        elif q[(k - 1) % 4] == t:
            f2, t2 = q[(k + 1) % 4], q[(k + 2) % 4]
        else:
            return steps, False, (f, t)
        steps.append((p, (f, t), (f2, t2)))
        key = (min(f2, t2), max(f2, t2))
        if key == start_key:
            return steps, True, (f2, t2)
        others = [x for x in edge_faces[key] if x != p]
        if len(others) != 1:
            return steps, False, (f2, t2)
        p, f, t = others[0], f2, t2


@node("polysplit", inputs=["geo"], category="model", params=[
    Enum("mode", ["edge_loop", "points"], doc="edge_loop: insert loops through the quad strip around each edge; "
                                              "points: cut faces between two of their points"),
    Str("split_edges", "", doc="edge_loop: the edges to cut across: ids/ranges, p0-1 pairs (direction p0 -> p1 decides where "
                         "'position' counts from), edge groups"),
    Int("loops", 1, min=1, max=64, doc="How many loops to insert, evenly spaced"),
    Float("position", 0.5, min=0.0, max=1.0, doc="Where a single loop crosses the edge, from the first point"),
    Str("split_points", "", doc="points mode: pairs like '0-2 5-7'; each cuts the face that holds both points"),
    *SPLIT_OUT.params(),
])
def polysplit(geo, mode, split_edges, loops, position, split_points, **og):
    """Insert edge loops or cut faces (Houdini's PolySplit). A loop runs through connected quads on both sides of the
    chosen edge until it meets a boundary or a face that is not a quad; that face gets the new point too, so there are
    no cracks."""
    B.require_polygons(geo, "polysplit")
    t = geo.topology
    vp = t.vertex_point.astype(np.int64)
    if mode == "points":
        return _split_points(geo, split_points, og)
    if not split_edges.strip():
        return geo
    sel = edges.mask(geo, split_edges)
    e = topo.edges(geo)
    pv = topo.prim_of_vertex(geo)
    edge_faces = {}
    for v in range(t.nvertices):
        if e.of_vertex[v] < 0:
            continue
        a, b = int(e.points[e.of_vertex[v], 0]), int(e.points[e.of_vertex[v], 1])
        edge_faces.setdefault((a, b), []).append(int(pv[v]))
    sizes = t.prim_sizes()
    quads = {p: vp[t.prim_offsets[p]:t.prim_offsets[p + 1]].tolist() for p in range(t.nprims) if sizes[p] == 4}
    ring_dir = {}                                              # undirected edge -> (from, to)
    split_quads = {}                                           # quad -> (entry from/to, opposite from/to)
    for eid in np.flatnonzero(sel):
        a, b = int(e.points[eid, 0]), int(e.points[eid, 1])
        key = (a, b)
        if key in ring_dir or len(edge_faces.get(key, [])) not in (1, 2):
            continue
        ring_dir[key] = (a, b)
        for p in edge_faces[key]:
            steps, _closed, last = _walk(edge_faces, quads, p, a, b, key)
            for q, entry, opp in steps:
                if q in split_quads:
                    continue
                split_quads[q] = (entry, opp)
                for (f, tt) in (entry, opp):
                    ring_dir.setdefault((min(f, tt), max(f, tt)), (f, tt))
    if not ring_dir:
        return geo
    n = int(loops)
    fracs = [position] if n == 1 else [(k + 1) / (n + 1) for k in range(n)]
    # new points, ordered from each edge's "from" end
    keys = sorted(ring_dir)
    ff = np.array([ring_dir[k][0] for k in keys for _ in fracs], np.int64)
    tt_ = np.array([ring_dir[k][1] for k in keys for _ in fracs], np.int64)
    fr = np.array([x for _ in keys for x in fracs], np.float64)
    npts = geo.npoints
    newid = {(k, j): npts + ki * n + j for ki, k in enumerate(keys) for j in range(n)}
    point = B.concat([dict(geo.point.items()), B.lerp(geo.point, ff, tt_, fr)])

    def along(a, b):
        """Point ids on the undirected edge listed from a to b, and the fractions measured from a."""
        key = (min(a, b), max(a, b))
        f0, _ = ring_dir[key]
        ids = [newid[(key, j)] for j in range(n)]
        fs = list(fracs)
        if a != f0:
            ids, fs = ids[::-1], [1 - x for x in fs[::-1]]
        return ids, fs

    # corner records: (row_a, row_b, frac, point); an original corner is (row, row, 0, point)
    out_faces, sources = [], []
    cut_pairs = []
    for p in range(t.nprims):
        lo, hi = int(t.prim_offsets[p]), int(t.prim_offsets[p + 1])
        rows = list(range(lo, hi))
        if p in split_quads:
            entry, opp = split_quads[p]
            # rotate the quad so it starts at the entry edge: [a, b, c, d] with (a,b) = entry, (c,d) = opposite
            q = quads[p]
            k = q.index(entry[0])
            if q[(k + 1) % 4] == entry[1]:
                order = [(k + s) % 4 for s in range(4)]
            else:                                              # entry runs against the quad: mirror, still a valid cycle
                order = [(k - s) % 4 for s in range(4)]
            ra = [rows[o] for o in order]
            pa = [q[o] for o in order]
            ids_ab, fs_ab = along(pa[0], pa[1])
            ids_dc, fs_dc = along(pa[3], pa[2])
            strip = []
            top = [(ra[0], ra[0], 0.0, pa[0])] + [(ra[0], ra[1], f, i) for i, f in zip(ids_ab, fs_ab)] + \
                  [(ra[1], ra[1], 0.0, pa[1])]
            bot = [(ra[3], ra[3], 0.0, pa[3])] + [(ra[3], ra[2], f, i) for i, f in zip(ids_dc, fs_dc)] + \
                  [(ra[2], ra[2], 0.0, pa[2])]
            reverse = order[1] != (order[0] + 1) % 4          # mirrored cycle winds the other way
            for s in range(n + 1):
                face = [top[s], top[s + 1], bot[s + 1], bot[s]]
                strip.append(face[::-1] if reverse else face)
            for s in range(1, n + 1):
                cut_pairs.append((top[s][3], bot[s][3]))
            out_faces.extend(strip)
            sources.extend([p] * len(strip))
            continue
        # a face next to a split edge gets the new points on that edge to stay watertight
        corners = []
        for c in range(lo, hi):
            a, b = int(vp[c]), int(vp[lo + (c - lo + 1) % (hi - lo)])
            corners.append((c, c, 0.0, a))
            key = (min(a, b), max(a, b))
            if key in ring_dir:
                ids, fs = along(a, b)
                nxt_row = lo + (c - lo + 1) % (hi - lo)
                corners.extend((c, nxt_row, f, i) for i, f in zip(ids, fs))
        out_faces.append(corners)
        sources.append(p)
    flat = [c for face in out_faces for c in face]
    ia = np.array([c[0] for c in flat], np.int64)
    ib = np.array([c[1] for c in flat], np.int64)
    ft = np.array([c[2] for c in flat], np.float64)
    vtx = B.lerp(geo.vertex, ia, ib, ft)
    prim = B.gather(geo.prim, np.array(sources, np.int64))
    out = B.make(point["P"], np.array([c[3] for c in flat], np.int32), B.offsets_from_sizes([len(f) for f in out_faces]),
                 point, vtx, prim, geo.detail, B.closed_subset(geo, np.array(sources, np.int64)))
    out = edges.remap(geo, out)
    out = SPLIT_OUT.write(out, og, "points", lambda: np.arange(out.npoints) >= npts)
    return SPLIT_OUT.write(out, og, "edges", lambda: edge_mask(out, cut_pairs))


def _split_points(geo, pairs_text, og):
    pairs = groups.parse_edges(pairs_text, geo.npoints)
    if not len(pairs):
        return geo
    t = geo.topology
    vp = t.vertex_point
    faces = [list(range(int(t.prim_offsets[p]), int(t.prim_offsets[p + 1]))) for p in range(t.nprims)]
    sources = list(range(t.nprims))
    cuts = []
    for a, b in pairs:
        for fi, face in enumerate(faces):
            pts = [int(vp[c]) for c in face]
            if a in pts and b in pts and len(face) > 3:
                i, j = sorted((pts.index(int(a)), pts.index(int(b))))
                if j - i < 2 or (len(face) - (j - i)) < 2:
                    continue                                   # already an edge
                faces[fi] = face[i:j + 1]
                faces.append(face[j:] + face[:i + 1])
                sources.append(sources[fi])
                cuts.append((int(a), int(b)))
                break
        else:
            raise ValueError(f"polysplit: no face has both points {a} and {b} (not neighbours)")
    closed = B.closed_subset(geo, np.array(sources, np.int64))
    out = _rebuild(geo, faces, sources, closed)
    return SPLIT_OUT.write(out, og, "edges", lambda: edge_mask(out, cuts))
