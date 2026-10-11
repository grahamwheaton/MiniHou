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


def _wedge_points(geo, d, w, vp, pr, nx, pv, twin, bev, bound_in, bound_out):
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
    centroids = topo.prim_centroids(geo).astype(np.float64)
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
    inward = centroids[pv[corner[k]]] - v                    # a straight corner of one face: step into the face
    inward = _unit(inward - np.einsum("ij,ij->i", inward, a)[:, None] * a) * d
    off = np.where(border[:, None], slide(c_border, e_border),
                   np.where(has_inner[:, None], slide(_unit(inner_dir[k]), a),
                            np.where(bis[:, None], _unit(m) * (sign * d / np.maximum(half, 0.05))[:, None], inward)))
    out[k] = v + off
    return out, corner


@node("polybevel", inputs=["geo"], category="model", params=[
    Float("distance", 0.1, min=0.0, soft_max=1.0, doc="How far the bevel cuts into the faces on each side"),
    Float("min_angle", 30.0, min=0.0, max=180.0,
          doc="Only bevel edges whose faces meet at least this many degrees off flat (0: every interior edge)"),
    Str("group", "", doc="Points: only edges with both ends in the group are bevelled (empty: all)"),
    Str("output_group", "bevel", doc="Primitive group for the new bevel faces (empty: none)"),
])
def polybevel(geo, distance, min_angle, group, output_group):
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
    pos, corner = _wedge_points(geo, float(distance), w, vp, pr, nx, pv, twin, bev, bound_in, bound_out)
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
