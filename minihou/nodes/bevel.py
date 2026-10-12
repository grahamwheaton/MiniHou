"""Poly bevel (Houdini's PolyBevel, one segment): chamfer edges of a polygon mesh.

The faces around each point are split into *wedges* by the bevelled edges (faces joined across an unbevelled edge
stay together). Every wedge gets its own copy of the point, moved into the wedge so it sits ``distance`` from the
bevelled edges that bound it: along the bisector between two bevelled edges, or sliding along a mesh border or an
unbevelled edge when that is where the wedge continues (so a subdivided box bevels cleanly). Each bevelled edge becomes
a quad between the copies on its two sides, and points where three or more bevelled edges meet get a corner polygon.
"""
import numpy as np

from .. import groups, spatial, topo
from ..sdk import Float, Str, node
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


def _check_ends(geo, vp, bev, twin):
    """A bevel cannot stop in the middle of a surface: a selected edge that ends at an interior point where no other
    selected edge continues it would leave the faces around that point unsplit. Say so, naming the points."""
    n = geo.npoints
    count = np.bincount(vp[bev], minlength=n)
    open_pt = np.zeros(n, bool)
    open_pt[vp[twin < 0]] = True
    dangling = np.flatnonzero((count == 1) & ~open_pt)
    if len(dangling):
        raise ValueError(f"a bevel cannot end inside the surface (at point{'s' if len(dangling) > 1 else ''} "
                         f"{', '.join(map(str, dangling[:6].tolist()))}{'...' if len(dangling) > 6 else ''}): select a whole "
                         f"edge loop, edges that meet other selected edges, or edges that run to an open border")


@node("polybevel", inputs=["geo"], category="model", params=[
    Float("distance", 0.1, min=0.0, soft_max=1.0, doc="How far the bevel cuts into the faces on each side"),
    Float("min_angle", 30.0, min=0.0, max=180.0,
          doc="Only bevel edges whose faces meet at least this many degrees off flat (0: every interior edge)"),
    Str("group", "", doc="Points: only edges with both ends in the group are bevelled (empty: all)"),
    Str("edges", "", doc="Edges to bevel as point pairs, e.g. '12-13 40-41' (the 3-D view's edge selection writes this). "
                         "When set, exactly these interior edges are bevelled and min_angle is ignored"),
    Str("output_group", "bevel", doc="Primitive group for the new bevel faces (empty: none)"),
])
def polybevel(geo, distance, min_angle, group, edges, output_group):
    """Chamfer the sharp edges of a polygon mesh (Houdini's PolyBevel with one segment). Keep ``distance`` below half
    the shortest bevelled face edge, or the cuts overlap. N is removed (recompute it with a normal node)."""
    B.require_polygons(geo, "polybevel")
    if geo.nprims == 0 or distance <= 0.0:
        return geo
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
        _check_ends(geo, vp, bev, twin)
    sel = groups.mask(geo, "point", group)
    if sel is not None:
        bev &= sel[vp] & sel[vp[nx]]
    if not bev.any():
        return geo
    bound_out = bev | ~two
    bound_in = bound_out[pr]
    # corners on either side of an unbevelled interior edge share a wedge
    join = np.flatnonzero(~bound_in)
    w = spatial.components(t.nvertices, join, twin[pr[join]])
    pos, corner = _wedge_points(geo, float(distance), w, vp, pr, nx, pv, twin, bev, bound_in, bound_out, N)
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
    ring_w = walk[walk >= 0]                                # row by row: each ring in order
    ring_src = corner[ring_w]

    v_src = np.concatenate([np.arange(t.nvertices), q_src.ravel(), ring_src])
    vpoint = w[v_src]
    sizes = np.concatenate([t.prim_sizes(), np.full(len(c1), 4), rlen]).astype(np.int64)
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
    return B.drop_degenerate(out)
