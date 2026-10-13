"""The boolean node (Houdini's Boolean): union, intersect and subtract of closed polygon meshes, exact (see
:mod:`minihou.boolean`)."""
import numpy as np

from .. import boolean as BL
from .. import groups, topo
from ..materials import merge_sources, unify as unify_materials
from ..outgroups import OutGroup, OutGroups
from ..sdk import Enum, node
from . import _build as B
from .model import compact_points
from .normal import normal


def _triangles(geo):
    """(point triples, vertex triples, source prim) for every primitive of a polygon mesh. Strictly convex polygons
    fan; polygons with concave or straight (collinear) corners are ear-clipped, so no triangle has zero area."""
    from .. import topo
    from .normal import prim_normals
    t = geo.topology
    offs = t.prim_offsets.astype(np.int64)
    sizes = np.diff(offs)
    pv = topo.prim_of_vertex(geo)
    P = geo.P.astype(np.float64)
    pos = P[t.vertex_point]
    nxt, prv = topo.next_vertex(geo), topo.prev_vertex(geo)
    n = prim_normals(geo).astype(np.float64)
    a, b = pos - pos[prv], pos[nxt] - pos
    turn = np.einsum("ij,ij->i", np.cross(a, b), n[pv])
    scale = np.linalg.norm(a, axis=1) * np.linalg.norm(b, axis=1) * np.linalg.norm(n[pv], axis=1)
    bad = np.bincount(pv, weights=(turn <= 1e-9 * scale), minlength=t.nprims) > 0
    fan = (sizes >= 3) & ~(bad & (sizes > 3))
    counts = np.where(fan, sizes - 2, 0)
    first = np.repeat(offs[:-1], counts)
    k = np.arange(counts.sum()) - np.repeat(np.cumsum(counts) - counts, counts)
    tv = [np.stack([first, first + k + 1, first + k + 2], 1)]
    tp = [np.repeat(np.arange(t.nprims), counts)]
    for q in np.flatnonzero(~fan & (sizes > 3)):
        verts = np.arange(offs[q], offs[q + 1])
        nq = n[q] / max(np.linalg.norm(n[q]), 1e-300)
        u = np.cross(nq, [1.0, 0, 0] if abs(nq[0]) < 0.9 else [0, 1.0, 0])
        u /= np.linalg.norm(u)
        xy = np.stack([pos[verts] @ u, pos[verts] @ np.cross(nq, u)], 1)
        tris = BL.earclip(list(range(len(verts))), xy)
        if tris:
            tv.append(verts[np.asarray(tris, np.int64)])
            tp.append(np.full(len(tris), q))
    tv, tp = np.concatenate(tv), np.concatenate(tp)
    return t.vertex_point.astype(np.int64)[tv], tv, tp


def _blend(table, idx, w):
    """Attributes of `table` at rows idx (m, 3) mixed with weights w (m, 3): floats blend, others take the
    heaviest row."""
    out = {}
    heavy = idx[np.arange(len(idx)), np.argmax(w, 1)] if len(idx) else np.zeros(0, np.int64)
    for name, a in table.items():
        if a.dtype.kind == "f":
            v = a[idx].astype(np.float64)
            ww = w.reshape(w.shape + (1,) * (a.ndim - 1))
            out[name] = (v * ww).sum(1).astype(a.dtype)
        else:
            out[name] = a[heavy]
    return out


def _combine(parts, sizes):
    """Concatenate attribute dicts, zero-filling names a part lacks."""
    from .merge import _concat_table
    names = []
    for p in parts:
        names += [n for n in p if n not in names]
    return {n: _concat_table(parts, sizes, n) for n in names}


OUT = OutGroups(
    OutGroup("from_a", "Faces from A", "from_a", "prim", default=True, doc="The result's faces that came from input A"),
    OutGroup("from_b", "Faces from B", "from_b", "prim", default=True, doc="The result's faces that came from input B"),
    OutGroup("seam", "Intersection Edges", "intersection", "edge",
             doc="The edges where the two inputs meet: where a face from A touches a face from B"),
    OutGroup("seampoints", "Intersection Points", "intersectionpoints", "point", doc="The points on those intersection edges"),
)


@node("boolean", inputs=["a", "b"], category="model", params=[
    Enum("operation", ["union", "subtract", "intersect"], doc="subtract: A minus B"),
    *OUT.params(),
])
def boolean(a, b, operation, **og):
    """Exact boolean of two closed polygon meshes (Houdini's Boolean). The result is watertight; faces keep their
    attributes (vertex attributes such as uv are interpolated across cuts), cut points blend point attributes, and
    faces from each input are in groups ``from_a`` / ``from_b``. Inputs must be closed and consistently wound."""
    for g, nm in ((a, "A"), (b, "B")):
        B.require_polygons(g, f"boolean input {nm}")
    a, b = unify_materials([a, b])                          # both inputs' material indices share one name list
    if a.nprims == 0 or b.nprims == 0:
        empty = {"union": a if b.nprims == 0 else b, "subtract": a, "intersect": None}[operation]
        return empty if empty is not None else B.make(np.zeros((0, 3), np.float32), [], [0])
    TA, va, pa = _triangles(a)
    TB, vb, pb = _triangles(b)
    PA, PB = a.P.astype(np.float64), b.P.astype(np.float64)
    allp = np.concatenate([PA, PB])
    eps = 1e-6 * float(np.linalg.norm(np.ptp(allp, axis=0)))
    NB = normal(b, "point").point["N"].astype(np.float64)
    PBg = PB + NB * eps                                     # B grown a hair: flush faces cut cleanly
    # and nudged generically (well under the growth): no edge of B stays exactly on an edge or diagonal of A
    size = max(float(np.ptp(allp, axis=0).max()), 1e-12)
    axis = np.array([0.5369, 0.2875, 0.7932])
    ang = 0.15 * eps / size
    c = PBg.mean(0)
    K = np.array([[0, -axis[2], axis[1]], [axis[2], 0, -axis[0]], [-axis[1], axis[0], 0]]) / np.linalg.norm(axis)
    R = np.eye(3) + np.sin(ang) * K + (1 - np.cos(ang)) * (K @ K)
    PBg = (PBg - c) @ R.T + c + eps * np.array([0.0731, -0.1123, 0.0917])
    r = BL.boolean(PA, TA, PBg, TB, operation, weld=3.0 * eps)
    P, keys, et = r["P"], r["keys"], r["edge_t"]
    nA, nB = len(PA), len(PB)
    P[nA:nA + nB] = PB                                      # B's points at their true positions
    P = P[r["remap"]]                                       # welded points sit where their representative is
    # points: A's, B's, then cut points blending their edge's point attributes
    ka, kb = keys[:, 0] == 0, keys[:, 0] == 1
    pts_x = [B.lerp(g.point, keys[m, 1], keys[m, 2], et[m]) for g, m in ((a, ka), (b, kb))]
    point = _combine([dict(a.point.items()), dict(b.point.items()), pts_x[0], pts_x[1]],
                     [nA, nB, int(ka.sum()), int(kb.sum())])
    order_x = np.concatenate([np.flatnonzero(ka), np.flatnonzero(kb)])
    perm = np.concatenate([np.arange(nA + nB), nA + nB + np.argsort(order_x, kind="stable")])
    inv = np.empty_like(perm)
    inv[perm] = np.arange(len(perm))
    point = {n: arr[inv] for n, arr in point.items()}
    point["P"] = P.astype(np.float32)
    verts, sizes, owner, src = r["verts"], r["sizes"], r["owner"], r["src"]
    whole, bary, flipped = r["whole"], r["bary"], r["flipped"]
    first = np.cumsum(sizes) - sizes
    nTA = len(TA)
    parts_v, parts_vert, parts_prim, parts_size, part_side = [], [], [], [], []
    for side, (g, tv, tp, base, off) in enumerate(((a, va, pa, 0, 0), (b, vb, pb, nA, nTA))):
        mine = np.flatnonzero(owner == side)
        if not len(mine):
            continue
        tri = src[mine] - off                               # source triangle within this input
        prim_of = tp[tri]
        # source primitives the cut never touched come back as their original polygons
        ntri = np.bincount(tp, minlength=g.nprims)
        nwhole = np.bincount(prim_of[whole[mine]], minlength=g.nprims)
        intact = (nwhole == ntri) & (ntri > 0)
        as_tri = mine[~intact[prim_of]]
        prims = np.flatnonzero(intact)
        offs = g.topology.prim_offsets.astype(np.int64)
        flip_side = bool(flipped[mine].any())
        if len(prims):
            sz = offs[prims + 1] - offs[prims]
            loc = np.arange(sz.sum()) - np.repeat(np.cumsum(sz) - sz, sz)
            vv = np.repeat(offs[prims], sz) + (np.repeat(sz, sz) - 1 - loc if flip_side else loc)
            parts_v.append(g.topology.vertex_point.astype(np.int64)[vv] + base)
            parts_vert.append(B.gather(g.vertex, vv))
            parts_prim.append(B.gather(g.prim, prims))
            parts_size.append(sz)
            part_side.append(np.full(len(prims), side))
        if len(as_tri):
            sz = sizes[as_tri]
            corner = np.repeat(first[as_tri], sz) + (np.arange(sz.sum()) - np.repeat(np.cumsum(sz) - sz, sz))
            vattr = _blend(g.vertex, tv[np.repeat(src[as_tri] - off, sz)], bary[corner])   # per piece corner
            # pieces of one source polygon sharing uncut edges merge back into one n-gon
            # (only for planar source polygons: pieces of a bent quad lie on its two triangles' planes)
            tri_src = src[as_tri] - off
            grp = np.where(_planar(g)[tp[tri_src]], tp[tri_src], g.nprims + tri_src)
            cidx, msz, mgrp = _merge_pieces(verts[corner], sz, grp, P)
            mprim = np.where(mgrp < g.nprims, mgrp, tp[np.clip(mgrp - g.nprims, 0, len(tp) - 1)])
            parts_v.append(verts[corner][cidx])
            parts_vert.append({n: arr[cidx] for n, arr in vattr.items()})
            parts_prim.append(B.gather(g.prim, mprim))
            parts_size.append(msz)
            part_side.append(np.full(len(msz), side))
    if not parts_size:
        return B.make(np.zeros((0, 3), np.float32), np.zeros(0, np.int64), np.zeros(1, np.int64))
    sizes_o = np.concatenate(parts_size)
    vert = _combine(parts_vert, [int(x.sum()) for x in parts_size])
    prim = _combine(parts_prim, [len(x) for x in parts_size])
    side = np.concatenate(part_side)
    out = B.make(point["P"], np.concatenate(parts_v), B.offsets_from_sizes(sizes_o), point, vert, prim, a.detail)
    merge_sources([a, b], out)
    out = OUT.write(out, og, "from_a", side == 0)
    out = OUT.write(out, og, "from_b", side == 1)
    if OUT.any_enabled(og, "seam", "seampoints"):
        seam = _seam_edges(out, side)
        out = OUT.write(out, og, "seam", seam)
        e = topo.edges(out)
        pts = np.zeros(out.npoints, bool)
        pts[e.points[seam].ravel()] = True
        out = OUT.write(out, og, "seampoints", pts)
    for table in (out.point, out.vertex, out.prim):
        if "N" in table:
            del table["N"]
    return compact_points(out)


def _seam_edges(out, side):
    """Bool per derived edge: shared by a face from A and a face from B."""
    e = topo.edges(out)
    pv = topo.prim_of_vertex(out)
    ok = e.of_vertex >= 0
    from_b = (side == 1)[pv] & ok
    from_a = (side == 0)[pv] & ok
    nb = np.bincount(e.of_vertex[from_b], minlength=len(e.points))
    na = np.bincount(e.of_vertex[from_a], minlength=len(e.points))
    return (na > 0) & (nb > 0)


def _planar(geo):
    """Bool per primitive: all its points lie on one plane (to 1e-6 of the geometry's size)."""
    from .normal import prim_normals
    t = geo.topology
    n = prim_normals(geo).astype(np.float64)
    n = n / np.maximum(np.linalg.norm(n, axis=1, keepdims=True), 1e-300)
    pv = np.repeat(np.arange(t.nprims), t.prim_sizes())
    P = geo.P.astype(np.float64)[t.vertex_point]
    c = np.stack([np.bincount(pv, weights=P[:, k], minlength=t.nprims) for k in range(3)], 1) / \
        np.maximum(t.prim_sizes(), 1)[:, None]
    d = np.abs(np.einsum("ij,ij->i", P - c[pv], n[pv]))
    far = np.zeros(t.nprims)
    np.maximum.at(far, pv, d)
    size = float(np.ptp(geo.P, axis=0).max()) if geo.npoints else 1.0
    return far <= 1e-6 * max(size, 1e-12)


def _cycles(succ, b):
    """The closed loops of an outline given as point -> outgoing corner (corner c runs to point b[c])."""
    loops, done = [], set()
    for start in succ:
        if start in done:
            continue
        loop, p = [], start
        while p not in done:
            done.add(p)
            c = succ.get(p)
            if c is None:
                return None
            loop.append(c)
            p = int(b[c])
        if p != start:
            return None
        loops.append(loop)
    return loops


def _outline_polygons(loops, Pc):
    """Simple polygons (lists of corners) for a planar outline made of loops: one loop is the polygon itself; an
    outer loop with holes is split around each hole (two bridges per hole). Pc: the 3-D position of every corner."""
    if len(loops) == 1:
        return [loops[0]]
    allc = np.concatenate([np.asarray(l) for l in loops])
    Q = Pc[allc]
    n = np.zeros(3)
    for l in loops:
        q = Pc[np.asarray(l)]
        n += np.cross(q, np.roll(q, -1, 0)).sum(0)
    if np.linalg.norm(n) < 1e-300:
        return None
    n /= np.linalg.norm(n)
    u = np.cross(n, [1.0, 0, 0] if abs(n[0]) < 0.9 else [0, 1.0, 0])
    u /= np.linalg.norm(u)
    w = np.cross(n, u)
    xy = np.zeros((int(allc.max()) + 1, 2))
    xy[allc] = np.stack([Q @ u, Q @ w], 1)
    areas = [BL._area2(l, xy) for l in loops]
    outer = int(np.argmax(areas))
    if areas[outer] <= 0 or sum(1 for a_ in areas if a_ > 0) != 1:
        return None
    holes = [list(l)[::-1] for i, l in enumerate(loops) if i != outer]   # clockwise in the outline: make CCW
    try:
        return BL.holes_into(list(loops[outer]), holes, xy)
    except BL.BooleanError:
        return None


def _merge_pieces(v, sizes, group, P):
    """Merge polygons (corners v, sizes) that share edges and the same `group` (their source primitive) into single
    polygons where their outline is one loop. Returns (corner indices into v of the merged polygons, their sizes,
    their group)."""
    first = np.cumsum(sizes) - sizes
    poly_of = np.repeat(np.arange(len(sizes)), sizes)
    local = np.arange(len(v)) - first[poly_of]
    nxt = np.where(local == sizes[poly_of] - 1, first[poly_of], np.arange(len(v)) + 1)
    a, b = v, v[nxt]
    from .. import spatial
    # polygons of one group joined across an edge they share (it appears once in each direction)
    key = {}
    for c in range(len(v)):
        key[(int(group[poly_of[c]]), int(a[c]), int(b[c]))] = c
    ii, jj = [], []
    for c in range(len(v)):
        o = key.get((int(group[poly_of[c]]), int(b[c]), int(a[c])))
        if o is not None and poly_of[o] != poly_of[c]:
            ii.append(poly_of[c])
            jj.append(poly_of[o])
    comp = spatial.components(len(sizes), np.array(ii, np.int64), np.array(jj, np.int64))
    out_c, out_s, out_g = [], [], []
    order = np.argsort(comp, kind="stable")
    bounds = np.searchsorted(comp[order], np.arange(int(comp.max()) + 2 if len(comp) else 1))
    for k in range(len(bounds) - 1):
        polys = order[bounds[k]:bounds[k + 1]]
        if len(polys) == 1:
            p = polys[0]
            out_c.append(np.arange(first[p], first[p] + sizes[p]))
            out_s.append(sizes[p])
            out_g.append(group[p])
            continue
        corners = np.concatenate([np.arange(first[p], first[p] + sizes[p]) for p in polys])
        g = int(group[polys[0]])
        succ = {}
        for c in corners:
            o = key.get((g, int(b[c]), int(a[c])))
            if o is None or comp[poly_of[o]] != k:          # an outline edge of the merged polygon
                if int(a[c]) in succ:
                    succ = None                             # the outline touches itself: keep the pieces
                    break
                succ[int(a[c])] = c
        loops = _cycles(succ, b) if succ else None
        merged = _outline_polygons(loops, P[v]) if loops else None
        if merged is None:
            merged = [np.arange(first[p], first[p] + sizes[p]) for p in polys]
        for m in merged:
            out_c.append(np.asarray(m, np.int64))
            out_s.append(len(m))
            out_g.append(g)
    return np.concatenate(out_c), np.array(out_s, np.int64), np.array(out_g, np.int64)


def _blend_corners(table, tri_verts, src, bary):
    """Vertex attributes for output corners: each corner is a barycentric mix (bary (m, 3, 3)) of the source
    triangle's three vertices."""
    idx = np.repeat(tri_verts[src], 3, axis=0)              # (m*3, 3): the source vertices for every corner
    return _blend(table, idx, bary.reshape(-1, 3))
