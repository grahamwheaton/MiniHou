"""Poly bevel: chamfer or round polygon mesh edges with segmented profiles.

The faces around each point are split into *wedges* by the bevelled edges (faces joined across an unbevelled edge
stay together). Every wedge gets its own copy of the point, moved into the wedge so it sits ``distance`` from the
bevelled edges that bound it: along the bisector between two bevelled edges, or sliding along a mesh border or an
unbevelled edge when that is where the wedge continues (so a subdivided box bevels cleanly). Each bevelled edge becomes
a quad between the copies on its two sides, and points where three or more bevelled edges meet get a corner polygon (with several divisions,
a grid of quads on the sphere through the strips' profiles, see ``_segments``).
"""
import numpy as np

from .. import edges as edgelib, groups, spatial, topo
from ..outgroups import OutGroup, OutGroups, edge_mask
from ..sdk import Bool, Enum, Float, Input, Int, Str, node
from . import _build as B
from .normal import prim_normals

_EPS = 1e-12


def _unit(v):
    return v / np.maximum(np.linalg.norm(v, axis=-1, keepdims=True), _EPS)


def twins(geo):
    """For each half-edge (vertex i -> next vertex), the vertex starting the opposite half-edge, or -1 on borders
    and non-manifold edges (shared by more than two faces, or by two faces wound the same way)."""
    vp = geo.topology.vertex_point.astype(np.int64)
    nx = topo.next_vertex(geo)
    n = max(geo.npoints, 1)
    key = vp * n + vp[nx]
    order = np.argsort(key, kind="stable")
    sk = key[order]
    counts = np.unique(sk, return_counts=True)[1]
    single = np.zeros(len(key), bool)
    single[order] = np.repeat(counts == 1, counts)
    want = vp[nx] * n + vp
    pos = np.clip(np.searchsorted(sk, want), 0, max(len(sk) - 1, 0))
    twin = np.where((len(sk) > 0) & (sk[pos] == want), order[pos] if len(sk) else -1, -1)
    ok = (twin >= 0) & single
    ok &= np.where(twin >= 0, single[np.maximum(twin, 0)], False)
    return np.where(ok, twin, -1)


def _wedge_points(geo, d, w, vp, pr, nx, pv, twin, bev, bound_in, bound_out, fn):
    """Position of each wedge's copy of its point (see the module docstring)."""
    P = geo.P.astype(np.float64)
    nw = int(w.max()) + 1 if len(w) else 0
    corner = np.full(nw, -1, np.int64)
    corner[w[::-1]] = np.arange(len(w))[::-1]               # first corner of each wedge
    out = P[vp[corner]].copy()
    # each corner contributes its bounding edges: (wedge, direction, bevelled?, undirected edge id)
    vin = np.flatnonzero(bound_in)
    vout = np.flatnonzero(bound_out)
    bw = np.concatenate([w[vin], w[vout]])
    bdir = np.concatenate([P[vp[pr[vin]]] - P[vp[vin]], P[vp[nx[vout]]] - P[vp[vout]]])
    bbev = np.concatenate([bev[pr[vin]], bev[vout]])
    he = np.concatenate([pr[vin], vout])                    # half-edge ids; the same edge from both sides matches
    bedge = np.where(twin[he] >= 0, np.minimum(he, twin[he]), he)
    order = np.argsort(bw, kind="stable")
    bw, bdir, bbev, bedge = bw[order], bdir[order], bbev[order], bedge[order]
    nb = np.bincount(bw, minlength=nw)
    start = np.concatenate([[0], np.cumsum(nb)])[:-1]
    # interior (unbevelled, two-sided) edges inside each wedge, and the angle the wedge spans
    inner = ~bound_in
    a_in = _unit(P[vp[pr]] - P[vp])
    a_out = _unit(P[vp[nx]] - P[vp])
    ii = np.flatnonzero(inner)
    inner_dir = np.stack([np.bincount(w[ii], weights=a_in[ii, c], minlength=nw) for c in range(3)], 1)
    n_inner = np.bincount(w[ii], minlength=nw)
    ang = np.arccos(np.clip(np.einsum("ij,ij->i", a_in, a_out), -1.0, 1.0))
    span = np.bincount(w, weights=ang, minlength=nw)
    k = np.flatnonzero(nb == 2)
    st = start[k]
    k = k[(bbev[st] | bbev[st + 1]) & (bedge[st] != bedge[st + 1])]   # skip: no bevel, or the end of a bevelled run
    st = start[k]
    a, b = _unit(bdir[st]), _unit(bdir[st + 1])
    ea, eb = bbev[st], bbev[st + 1]
    v = out[k]

    def slide(c, e):                                        # along c until d away from the line e
        return c * np.minimum(d / np.maximum(np.linalg.norm(np.cross(c, e), axis=1), 0.05), 4 * d)[:, None]

    border = ea != eb                                       # slide along the border, away from the bevelled edge
    c_border = np.where(ea[:, None], b, a)
    e_border = np.where(ea[:, None], a, b)
    has_inner = (n_inner[k] > 0) & (np.linalg.norm(inner_dir[k], axis=1) > 1e-6)   # several faces: inner edges
    m = a + b
    bis = np.linalg.norm(m, axis=1) > 0.2                   # one face: the bisector, d away from both edges
    sign = np.where(span[k] <= np.pi + 1e-6, 1.0, -1.0)
    half = np.sqrt(np.clip((1.0 - np.einsum("ij,ij->i", a, b)) / 2.0, 0.0, 1.0))
    ci = corner[k]                                          # a straight corner of one face: step into the face,
    inward = _unit(np.cross(fn[pv[ci]], a_out[ci])) * d     # to the left of its outgoing edge (CCW faces)
    off = np.where(border[:, None], slide(c_border, e_border),
                   np.where(has_inner[:, None], slide(_unit(inner_dir[k]), a),
                            np.where(bis[:, None], _unit(m) * (sign * d / np.maximum(half, 0.05))[:, None], inward)))
    out[k] = v + off
    return out, corner


def _limit_overlap(geo, pos, w, home, pv, N):
    """Where moving the wedge points would fold an original face over (its normal turning by more than 90 degrees,
    e.g. long slivers next to a bevelled edge), halve those points' moves until nothing folds: Houdini's
    'limit to avoid overlap'."""
    P = geo.P.astype(np.float64)
    start = P[home]
    nv = geo.topology.nvertices
    for _ in range(12):
        Q = pos[w]                                          # each original corner at its wedge's new position
        nxt = np.r_[np.arange(1, nv), 0]
        offs = geo.topology.prim_offsets.astype(np.int64)
        last = offs[1:] - 1
        ok = np.diff(offs) > 0
        nxt[last[ok]] = offs[:-1][ok]
        cr = np.cross(Q, Q[nxt])
        n_new = np.stack([np.bincount(pv, weights=cr[:, k], minlength=geo.nprims) for k in range(3)], 1)
        folded = np.einsum("ij,ij->i", n_new, N) <= 0.05 * np.linalg.norm(n_new, axis=1)
        moved = np.linalg.norm(pos - start, axis=1) > 0
        bad = np.zeros(len(pos), bool)
        bad[w[folded[pv]]] = True
        bad &= moved
        if not bad.any():
            break
        pos = np.where(bad[:, None], start + (pos - start) * 0.5, pos)
    return pos


# Houdini's PolyBevel "Output Groups" section: every one off by default, with these names.
OUT = OutGroups(
    OutGroup("pointfilletpolys", "Point Fillet Polygons", "pointfilletpolys", "prim",
             doc="The corner polygons made where three or more bevelled edges meet at a point"),
    OutGroup("pointfilletedges", "Point Fillet Edges", "pointfilletedges", "edge", doc="The edges of those corner polygons"),
    OutGroup("edgefilletpolys", "Edge Fillet Polygons", "edgefilletpolys", "prim", doc="The strips (quads) that replace each bevelled edge"),
    OutGroup("offsetedges", "Offset Edges", "offsetedges", "edge",
             doc="The new edges where each strip meets the original faces: the offset copies of the bevelled edges"),
    OutGroup("offsetpoints", "Offset Points", "offsetpoints", "point", doc="The new points created by offsetting the bevelled edges"),
    OutGroup("mergedpoints", "Merged Points", "mergedpoints", "point",
             doc="Points merged where offsets collide. This bevel limits an offset instead of merging, so the group is always empty"),
)


def profile_samples(curve, count):
    """The cross-section curve as ``count + 1`` evenly spaced (along, bulge) pairs: its first-to-last point is the chord
    (along 0 to 1), and the bulge is the offset from it in chord lengths, positive towards the corner being rounded."""
    t = curve.topology
    sizes = t.prim_sizes()
    ok = np.flatnonzero(sizes >= 2)
    if not len(ok):
        raise ValueError("polybevel: the cross-section curve input has no curve")
    pr = int(ok[0])
    o = int(t.prim_offsets[pr])
    pts = curve.P.astype(np.float64)[t.vertex_point[o:o + int(sizes[pr])].astype(np.int64)]
    a, z = pts[0], pts[-1]
    chord = z - a
    L = float(np.linalg.norm(chord))
    if L < 1e-12:
        raise ValueError("polybevel: the cross-section curve starts and ends at the same point")
    x = chord / L
    q = pts - a
    # the bulge axis: the direction in the curve's plane perpendicular to the chord (the farthest point's side is +)
    perp = q - np.outer(q @ x, x)
    k = int(np.argmax(np.linalg.norm(perp, axis=1)))
    y = perp[k] / max(float(np.linalg.norm(perp[k])), 1e-30) if np.linalg.norm(perp[k]) > 1e-12 else np.zeros(3)
    u = np.stack([(q @ x) / L, (q @ y) / L], 1)
    seg = np.r_[0.0, np.cumsum(np.linalg.norm(np.diff(u, axis=0), axis=1))]
    want = np.linspace(0.0, seg[-1], count + 1)
    return np.stack([np.interp(want, seg, u[:, 0]), np.interp(want, seg, u[:, 1])], 1)


@node("polybevel", inputs=["geo", Input("profile", optional=True, doc="Cross-section curve (Houdini's second input): the bevel's profile, from "
                                                                     "the first to the last point; bulging towards the corner. Replaces shape/convexity")],
      category="model", params=[
    Float("distance", 0.1, min=0.0, soft_max=1.0, doc="How far the bevel cuts into the faces on each side"),
    Int("divisions", 1, min=1, max=64, doc="Segments across each bevel strip. One preserves the original chamfer."),
    Enum("shape", ["round", "flat"], doc="Round uses a tangent circular profile; flat subdivides the chamfer."),
    Float("convexity", 1.0, min=0.0, max=1.0, doc="Blend from a flat profile (0) to the round profile (1)."),
    Bool("limit_overlap", True, doc="Reduce offsets that would fold an original face. This is not a global self-intersection solver."),
    Str("exclude_edges", "", doc="Edges to exclude: edge ids, named groups, or pA-B point pairs."),
    Float("min_angle", 30.0, min=0.0, max=180.0,
          doc="Only bevel edges whose faces meet at least this many degrees off flat (0: every interior edge)"),
    Str("group", "", doc="What to bevel, read as 'group_type' (empty: every sharp edge, see min_angle)"),
    Enum("group_type", ["points", "edges", "primitives"],
         doc="points: edges with both ends in the point group; edges: an edge group, edge numbers or pA-B pairs (e.g. the offsetedges "
             "of an earlier bevel); primitives: every interior edge of those primitives. A group selects exactly those edges"),
    Str("edges", "", doc="Edges to bevel as point pairs, e.g. '12-13 40-41' (the 3-D view's edge selection writes this). "
                         "When set, exactly these interior edges are bevelled and min_angle is ignored"),
    Str("output_group", "bevel", doc="Primitive group for all the new bevel faces (empty: none)"),
    *OUT.params(),
])
def polybevel(geo, profile=None, *, distance, min_angle, group, group_type, edges, output_group, divisions=1, shape="round", convexity=1.0, limit_overlap=True, exclude_edges="", **og):
    """Bevel polygon edges with segmented round or flat profiles. Keep ``distance`` below half
    the shortest bevelled face edge, or the cuts overlap. N is removed (recompute it with a normal node)."""
    B.require_polygons(geo, "polybevel")
    if geo.nprims == 0 or distance <= 0.0:
        return _outputs(geo, og, None)
    t = geo.topology
    vp = t.vertex_point.astype(np.int64)
    nx, pr = topo.next_vertex(geo), topo.prev_vertex(geo)
    pv = topo.prim_of_vertex(geo)
    twin = twins(geo)
    N = _unit(prim_normals(geo).astype(np.float64))
    two = twin >= 0
    cosang = np.einsum("ij,ij->i", N[pv], N[pv[np.maximum(twin, 0)]])
    bev = two & (cosang <= np.cos(np.radians(min_angle)) + (1e-6 if min_angle > 0 else 2.0))
    picked = groups.parse_edges(edges, geo.npoints)
    if len(picked):                                         # an explicit selection replaces the angle test
        n = max(geo.npoints, 1)
        want = np.isin(np.minimum(vp, vp[nx]) * n + np.maximum(vp, vp[nx]), picked[:, 0] * n + picked[:, 1])
        bev = two & want
    if group_type == "points":
        sel = groups.mask(geo, "point", group)
        if sel is not None:
            bev &= sel[vp] & sel[vp[nx]]
    elif (group or "").strip() and not len(picked):                 # an edge or primitive group selects exactly those edges
        if group_type == "edges":
            e = edgelib.mask(geo, group)
            want = np.zeros(len(vp), bool)
            have = topo.edges(geo).of_vertex >= 0
            want[have] = e[topo.edges(geo).of_vertex[have]]
        else:
            psel = groups.mask(geo, "prim", group)
            want = psel[pv] | psel[pv[np.maximum(twin, 0)]]
        bev = two & want
    if exclude_edges.strip():
        excluded = edgelib.mask(geo, exclude_edges)
        ei = topo.edges(geo).of_vertex
        bev &= ~np.where(ei >= 0, excluded[np.maximum(ei, 0)], False)
    if not bev.any():
        return _outputs(geo, og, None)
    # At a terminating bevel, split its two side-face corners from the cap fan.
    # The cap polygons gain the two offset endpoints instead of rejecting the edge.
    count = np.bincount(vp[bev], minlength=geo.npoints)
    dangling = count == 1
    side = bev | bev[pr]
    other_in = np.maximum(twin[pr], 0)
    other_out = nx[np.maximum(twin, 0)]
    bound_out = bev | ~two | (dangling[vp] & two & (side | side[other_out]))
    bound_in = bev[pr] | ~two[pr] | (dangling[vp] & two[pr] & (side | side[other_in]))
    # corners on either side of an unbevelled interior edge share a wedge
    join = np.flatnonzero(~bound_in)
    w = spatial.components(t.nvertices, join, twin[pr[join]])
    pos, corner = _wedge_points(geo, float(distance), w, vp, pr, nx, pv, twin, bev, bound_in, bound_out, N)
    if limit_overlap:
        pos = _limit_overlap(geo, pos, w, vp[corner], pv, N)
    nw = len(pos)

    # bevel quads, one per bevelled edge
    c1 = np.flatnonzero(bev & (np.arange(len(bev)) < twin))
    c2 = twin[c1]
    q_src = np.stack([nx[c1], c1, nx[c2], c2], 1)
    # corner polygons: chain the quads' sides around each point
    succ = np.full(nw, -1, np.int64)
    succ[w[nx[c2]]] = w[c1]
    succ[w[nx[c1]]] = w[c2]
    # each ring is a cycle of succ; walk all of them in step from their smallest wedge (rings are short: the valence)
    on = np.flatnonzero(succ >= 0)
    label = spatial.components(nw, on, succ[on])
    in_ring = np.zeros(nw, bool)
    in_ring[on] = True
    size = np.bincount(label, minlength=nw)
    closed = np.bincount(label, weights=in_ring, minlength=nw) == size     # every member continues: a cycle
    cand = np.flatnonzero(in_ring & closed[label] & (size[label] >= 3))   # ascending
    roots = cand[np.unique(label[cand], return_index=True)[1]]             # the smallest wedge of each ring
    rlen = size[label[roots]]
    steps = [roots]
    for _ in range(int(rlen.max()) - 1 if len(roots) else 0):
        steps.append(np.where(len(steps) < rlen, succ[steps[-1]], -1))
    walk = np.stack(steps, 1) if len(roots) else np.zeros((0, 0), np.int64)
    rings = [row[row >= 0].tolist() for row in walk]
    cap_corners = np.flatnonzero(dangling[vp] & ~side)
    for point_id in np.flatnonzero(dangling):
        caps = cap_corners[vp[cap_corners] == point_id]
        # Several cap faces share the original point. Close the remaining gap
        # with a triangle using the same orientation as the existing corner rings.
        if len(caps) > 1:
            candidates = on[vp[corner[on]] == point_id]
            if len(candidates):
                a = int(candidates[0]); b = int(succ[a]); c = int(w[caps[0]])
                if len({a,b,c}) == 3:
                    rings.append([a,b,c])
    rlen = np.asarray([len(ring) for ring in rings], dtype=np.int64)
    walk = np.full((len(rings), int(rlen.max()) if len(rlen) else 0), -1, np.int64)
    for i, ring in enumerate(rings):
        walk[i,:len(ring)] = ring
    roots = walk[:,0] if len(rings) else np.zeros(0,np.int64)
    ring_w = walk[walk >= 0]
    ring_src = corner[ring_w]

    original_points, original_sources, original_sizes = [], [], []
    for f in range(t.nprims):
        start = len(original_points)
        for v in range(int(t.prim_offsets[f]), int(t.prim_offsets[f+1])):
            if dangling[vp[v]] and not side[v]:
                a = int(w[other_in[v]]) if two[pr[v]] and side[other_in[v]] else int(w[v])
                b = int(w[other_out[v]]) if two[v] and side[other_out[v]] else int(w[v])
                original_points.append(a); original_sources.append(v)
                if b != a:
                    original_points.append(b); original_sources.append(v)
            else:
                original_points.append(int(w[v])); original_sources.append(v)
        original_sizes.append(len(original_points)-start)
    v_src = np.concatenate([np.asarray(original_sources), q_src.ravel(), ring_src])
    vpoint = np.concatenate([np.asarray(original_points), w[q_src.ravel()], w[ring_src]])
    sizes = np.concatenate([original_sizes, np.full(len(c1), 4), rlen]).astype(np.int64)
    ring_first = corner[roots]
    p_src = np.concatenate([np.arange(t.nprims), pv[c1], pv[ring_first]])
    point = B.gather(geo.point, vp[corner])
    point["P"] = pos.astype(np.float32)
    out = B.make(pos, vpoint, B.offsets_from_sizes(sizes), point, B.gather(geo.vertex, v_src),
                 B.gather(geo.prim, p_src), geo.detail)
    for table in (out.point, out.vertex, out.prim):          # old normals would be wrong on the new faces
        if "N" in table:
            del table["N"]
    if output_group:
        is_new = np.arange(out.nprims) >= t.nprims
        out = groups.write(out, "prim", output_group, is_new)
    nq, nr = len(c1), len(roots)
    npr, npt = out.nprims, out.npoints
    prim_range = lambda lo, hi: (np.arange(npr) >= lo) & (np.arange(npr) < hi)          # noqa: E731
    out = OUT.write(out, og, "edgefilletpolys", lambda: prim_range(t.nprims, t.nprims + nq))
    out = OUT.write(out, og, "pointfilletpolys", lambda: prim_range(t.nprims + nq, t.nprims + nq + nr))
    off_pts = np.unique(w[q_src.ravel()]) if nq else np.zeros(0, np.int64)
    out = OUT.write(out, og, "offsetpoints", lambda: np.isin(np.arange(npt), off_pts))
    out = OUT.write(out, og, "mergedpoints", np.zeros(npt, bool))
    # edge groups last: they are named by point pairs, which survive the removal of degenerate faces
    qw = w[q_src] if nq else np.zeros((0, 4), np.int64)
    offset_pairs = np.concatenate([qw[:, [0, 1]], qw[:, [2, 3]]])
    out = OUT.write(out, og, "offsetedges", lambda: edge_mask(out, offset_pairs))
    out = OUT.write(out, og, "pointfilletedges", lambda: edge_mask(out, _ring_pairs(walk, rlen)))
    if divisions > 1:
        out = _segments(out, t.nprims, nq, int(divisions), shape, float(convexity), geo.P[vp[corner]], og,
                         None if profile is None else profile_samples(profile, int(divisions)))
    out = B.drop_degenerate(out)
    if dangling.any():
        used = np.unique(out.topology.vertex_point)
        remap = np.full(out.npoints,-1,np.int32);remap[used]=np.arange(len(used))
        out = B.make(out.P[used],remap[out.topology.vertex_point],out.topology.prim_offsets,
                     B.gather(out.point,used),dict(out.vertex.items()),dict(out.prim.items()),out.detail)
    return out


def _ring_pairs(walk, rlen):
    """Edges (consecutive wedge points, cyclic) of every corner polygon."""
    if not len(rlen):
        return np.zeros((0, 2), np.int64)
    k = np.arange(walk.shape[1])[None, :]
    live = k < rlen[:, None]
    nxt = np.take_along_axis(walk, np.where(k + 1 < rlen[:, None], k + 1, 0), axis=1)
    return np.stack([walk[live], nxt[live]], 1)


def _outputs(geo, og, _unused):
    """Nothing was bevelled: the switched-on groups still exist, empty, so downstream group fields do not fail."""
    npr, npt = geo.nprims, geo.npoints
    for key, shape in (("pointfilletpolys", npr), ("edgefilletpolys", npr), ("offsetpoints", npt), ("mergedpoints", npt)):
        geo = OUT.write(geo, og, key, np.zeros(shape, bool))
    for key in ("pointfilletedges", "offsetedges"):
        geo = OUT.write(geo, og, key, lambda: np.zeros(len(topo.edges(geo).points), bool))
    return geo


def _segments(geo, first_strip, strip_count, count, shape, convexity, homes, og, profile=None):
    """Refine strips with shared profile chains; corner fans close the same chains.

    Float attributes interpolate; discrete attributes retain their source identity.
    Corners where strips meet are grids of quads on the sphere through their boundary (see ``patch``), not a general rolling-ball intersection.
    """
    P = geo.P.astype(float)
    offsets = geo.topology.prim_offsets
    vp = geo.topology.vertex_point
    faces = [vp[offsets[i]:offsets[i+1]].tolist() for i in range(geo.nprims)]
    positions = list(P)
    point_rows = {name: list(a) for name, a in geo.point.items() if name != 'P'}
    ancestry = [(i, i, 0.) for i in range(geo.npoints)]
    chains = {}
    # A two-edge miter is shared by adjacent strips. Its tangent intersection
    # is the original vertex, not a projection along whichever strip runs first.
    shared_ends = {}
    for a, b, c, d in faces[first_strip:first_strip+strip_count]:
        for end in ((a, d), (b, c)):
            key = tuple(sorted(end))
            shared_ends[key] = shared_ends.get(key, 0) + 1

    def add_point(position, a, b, fraction):
        index = len(positions)
        positions.append(position)
        ancestry.append((a, b, fraction))
        for name, array in geo.point.items():
            if name == 'P':
                continue
            point_rows[name].append(array[a] * (1-fraction) + array[b] * fraction if array.dtype.kind == 'f' else array[a])
        return index

    def chain(a, b, axis):
        key = tuple(sorted((a, b)))
        if key in chains:
            result = chains[key]
            return result if result[0] == a else result[::-1]
        left, right = P[a], P[b]
        midpoint = (left + right) / 2
        direction = _unit(np.asarray(axis))
        control = homes[a] if shared_ends[key] > 1 else homes[a] - direction * np.dot(homes[a] - midpoint, direction)
        u, v = _unit(left-control), _unit(right-control)
        weight = float(np.sqrt(max(0., (1.-np.clip(np.dot(u,v),-1.,1.))/2.)))
        result = [a]
        for j in range(1, count):
            t = j/count
            linear = left*(1-t) + right*t
            denom = (1-t)**2 + 2*weight*t*(1-t) + t*t
            arc = ((1-t)**2*left + 2*weight*t*(1-t)*control + t*t*right) / max(denom, _EPS)
            if profile is not None:
                bulge = _unit(control - midpoint) * np.linalg.norm(right - left)
                position = left + (right - left) * profile[j][0] + bulge * profile[j][1]
            else:
                position = linear if shape == 'flat' else linear + convexity*(arc-linear)
            result.append(add_point(position, a, b, t))
        result.append(b)
        chains[key] = result
        return result

    def patch(face):
        """The corner where several bevelled edges meet, as Blender's vertex mesh does it: every ring corner owns a grid of quads and the grids meet
        in the middle (a point for an even number of segments, a polygon for an odd one). The boundary is the profile chains of the strips; the inside
        is a Coons patch pushed onto the sphere through the boundary (the exact rounded corner of a box), so the corner arches like the edges do.
        None when a side is not a strip end (a cap), which keeps the plain fan."""
        m = len(face)
        sides = []
        for a, b in zip(face, face[1:] + face[:1]):
            seq = chains.get(tuple(sorted((a, b))))
            if seq is None or len(seq) != count + 1:
                return None
            sides.append(seq if seq[0] == a else seq[::-1])
        if m < 3:
            return None
        pts = [np.asarray([positions[i] for i in side], float) for side in sides]
        loop = np.concatenate([q[:-1] for q in pts])
        weight_sphere = 0.0 if shape == 'flat' else float(convexity)
        centre, radius = None, 0.0
        if weight_sphere > 0 and len(loop) >= 6:
            A = np.c_[2 * loop, np.ones(len(loop))]
            sol = np.linalg.lstsq(A, (loop * loop).sum(1), rcond=None)[0]
            r2 = sol[3] + float(sol[:3] @ sol[:3])
            if r2 > 0:
                r = float(np.sqrt(r2))
                miss = float(np.abs(np.linalg.norm(loop - sol[:3], axis=1) - r).max())
                if miss <= 0.2 * r and r <= 25 * max(float(np.ptp(loop, axis=0).max()), _EPS):      # a sphere fits (a flat or saddle corner stays a plain patch)
                    centre, radius = sol[:3], r

        def project(q, w):
            if centre is None:
                return q
            d = q - centre
            on = centre + radius * d / np.maximum(np.linalg.norm(d, axis=-1, keepdims=True), _EPS)
            return q + (weight_sphere * np.asarray(w, float))[..., None] * (on - q)

        def along(q, t):
            x = np.clip(t, 0.0, 1.0) * count
            i0 = np.minimum(np.floor(x).astype(int), count - 1)
            f = (x - i0)[..., None]
            return q[i0] * (1 - f) + q[i0 + 1] * f

        apex = project(loop.mean(axis=0), 1.0)
        mids = [along(q, np.asarray(0.5)) for q in pts]

        def seam(i, y):                                      # from the middle of side i to the apex, on the surface
            y = np.asarray(y, float)
            return project(mids[i] * (1 - y)[..., None] + apex * y[..., None], np.minimum(1.0, 8.0 * y))

        h, odd = count // 2, count % 2
        x = 2.0 * np.arange(h + 1) / count
        X, Y = np.meshgrid(x, x, indexing='ij')                 # X along side i from its corner, Y back along side i-1
        ids = {}
        grids = []
        for i in range(m):
            prev = (i - 1) % m
            bottom = along(pts[i], X / 2)
            left = along(pts[prev], 1 - Y / 2)
            right, top = seam(i, Y), seam(prev, X)
            e = lambda a: a[..., None]                       # noqa: E731
            F = (e(1 - Y) * bottom + e(Y) * top + e(1 - X) * left + e(X) * right
                 - (e((1 - X) * (1 - Y)) * pts[i][0] + e(X * (1 - Y)) * mids[i] + e((1 - X) * Y) * mids[prev] + e(X * Y) * apex))
            F = project(F, np.minimum(1.0, 8.0 * np.minimum(X, Y)))
            grid = np.empty((h + 1, h + 1), np.int64)
            for u in range(h + 1):
                for v in range(h + 1):
                    if u == 0 and v == 0:
                        grid[u, v] = face[i]
                    elif v == 0:
                        grid[u, v] = sides[i][u]
                    elif u == 0:
                        grid[u, v] = sides[prev][count - v]
                    else:
                        if not odd and u == h and v == h:
                            key = ('centre',)
                        elif not odd and u == h:
                            key = ('seam', i, v)
                        elif not odd and v == h:
                            key = ('seam', prev, u)
                        else:
                            key = ('own', i, u, v)
                        if key not in ids:
                            ids[key] = add_point(F[u, v], face[i], face[i], 0.)
                        grid[u, v] = ids[key]
            grids.append(grid)
        made = []
        for g in grids:                                        # quads wound like the ring they replace
            for u in range(h):
                for v in range(h):
                    made.append([int(g[u, v]), int(g[u + 1, v]), int(g[u + 1, v + 1]), int(g[u, v + 1])])
        if odd:                                                # bridge the grids across each side's middle, and close the middle with one polygon
            for i in range(m):
                g, nxt = grids[i], grids[(i + 1) % m]
                for v in range(h):
                    made.append([int(g[h, v]), int(nxt[v, h]), int(nxt[v + 1, h]), int(g[h, v + 1])])
            made.append([int(g[h, h]) for g in grids])
        return made

    strips = []
    for face in faces[first_strip:first_strip+strip_count]:
        a,b,c,d = face
        left = chain(a,d,P[b]-P[a])
        right = chain(b,c,P[b]-P[a])
        strips.append([[left[j],right[j],right[j+1],left[j+1]] for j in range(count)])

    new_faces, parents = [], []
    for f, face in enumerate(faces):
        if first_strip <= f < first_strip+strip_count:
            made = strips[f-first_strip]
        else:
            boundary = []
            for a,b in zip(face,face[1:]+face[:1]):
                seq = chains.get(tuple(sorted((a,b))), [a,b])
                if seq[0] != a:
                    seq = seq[::-1]
                boundary.extend(seq[:-1])
            if f >= first_strip+strip_count:
                made = patch(face)
                if made is None:                               # not a corner of strips (a cap): a fan from the middle
                    center = np.mean([positions[i] for i in boundary],axis=0)
                    center_id = add_point(center,face[0],face[0],0.)
                    made = [[a,b,center_id] for a,b in zip(boundary,boundary[1:]+boundary[:1])]
            else:
                made = [boundary]
        new_faces.extend(made)
        parents.extend([f]*len(made))

    va, vb, vt = [], [], []
    for face, parent in zip(new_faces, parents):
        mapping = dict(zip(faces[parent],range(int(offsets[parent]),int(offsets[parent+1]))))
        fallback = int(offsets[parent])
        for point in face:
            a,b,t = ancestry[point]
            va.append(mapping.get(a,fallback));vb.append(mapping.get(b,fallback));vt.append(t)
    vertex = B.lerp(geo.vertex,np.asarray(va),np.asarray(vb),np.asarray(vt))
    for name in list(vertex):
        if name.startswith(edgelib.PREFIX):
            del vertex[name]
    point = {name: np.asarray(rows,dtype=geo.point[name].dtype) for name,rows in point_rows.items()}
    out = B.make(np.asarray(positions),np.asarray([i for face in new_faces for i in face]),
                 B.offsets_from_sizes([len(face) for face in new_faces]),point,vertex,
                 B.gather(geo.prim,np.asarray(parents)),geo.detail)
    old_edges = topo.edges(geo).points
    for name in geo.vertex:
        if not name.startswith(edgelib.PREFIX):
            continue
        pairs = []
        for a,b in old_edges[edgelib.values(geo,name)>0]:
            seq = chains.get(tuple(sorted((int(a),int(b)))),[int(a),int(b)])
            pairs.extend(zip(seq,seq[1:]))
        out = edgelib.write(out,name,edge_mask(out,pairs).astype(np.int32))
    out = OUT.write(out,og,'offsetpoints',lambda:np.r_[groups.mask(geo,'point',OUT.settings(og,'offsetpoints')[1]),np.zeros(out.npoints-geo.npoints,bool)])
    return out
