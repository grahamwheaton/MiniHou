"""Voronoi fracture (Houdini's Voronoi Fracture): break a closed mesh into the Voronoi cells of a set of points.

Each cell is the mesh clipped by the bisector planes between its point and the others, nearest first. A plane further
from the point than the farthest corner of the piece left cannot cut it, so the loop stops there: the cells are exact,
not limited to k neighbours. Openings left by each cut are capped (only loops lying on that plane, so holes the
input already had stay open); caps form the group ``inside`` and every face gets the int ``piece`` of its cell.
"""
import numpy as np

from .. import groups, topo
from ..geometry import rows
from ..outgroups import OutGroup, OutGroups
from ..sdk import Bool, Float, Str, Vec3, node
from . import _build as B
from .merge import merge
from .model import boundary_loops, clip


def split_pinched(loop, keys=None):
    """A loop that touches itself at a point (a bow tie) as separate simple loops: each stretch between two visits of the
    same point is a loop of its own. ``keys`` (default: the loop itself) says which entries are the same point."""
    loop = [int(x) for x in loop]
    keys = loop if keys is None else [int(k) for k in keys]
    out, stack, at = [], [], {}
    for item, key in zip(loop, keys):
        if key in at:
            cut = at[key]
            ring = stack[cut:]
            for q, _k in ring[1:]:
                at.pop(_k, None)
            stack = stack[:cut + 1]
            if len(ring) >= 3:
                out.append([x for x, _ in ring])
        else:
            at[key] = len(stack)
            stack.append((item, key))
    if len(stack) >= 3:
        out.append([x for x, _ in stack])
    return out


def split_repeated_faces(geo):
    """Faces that visit a point twice (after welding) become simple faces; pieces with fewer than three corners vanish."""
    t = geo.topology
    vp = t.vertex_point.astype(np.int64)
    offs = t.prim_offsets.astype(np.int64)
    sizes = np.diff(offs)
    pv = topo.prim_of_vertex(geo)
    pair = pv * (int(vp.max()) + 1 if len(vp) else 1) + vp
    _, counts = np.unique(pair, return_counts=True)
    if not (counts > 1).any():
        return geo
    seen = np.unique(pair, return_counts=True)
    bad_pairs = set((seen[0][seen[1] > 1] // (int(vp.max()) + 1)).tolist())
    rows, new_sizes, src = [], [], []
    for p in range(t.nprims):
        corners = list(range(offs[p], offs[p + 1]))
        parts = split_pinched(corners, vp[corners].tolist()) if p in bad_pairs else [corners]
        for part in parts:
            rows.extend(part)
            new_sizes.append(len(part))
            src.append(p)
    rows = np.array(rows, np.int64)
    return B.make(geo.P, vp[rows], B.offsets_from_sizes(new_sizes), dict(geo.point.items()), B.gather(geo.vertex, rows),
                  B.gather(geo.prim, np.array(src, np.int64)), geo.detail)


def _cap_plane(geo, origin, normal, eps, group):
    """Fill the boundary loops of ``geo`` that lie on the plane, as faces in ``group``."""
    try:
        loops = boundary_loops(geo)
    except ValueError:                                     # a pinched boundary: leave it open rather than fail
        return geo
    d = np.abs((geo.P.astype(np.float64) - origin) @ normal)
    loops = [piece for lp in loops for piece in split_pinched(lp) if d[piece].max() <= eps]
    if not loops:
        return geo
    t = geo.topology
    sizes = np.array([len(lp) for lp in loops])
    new_vp = np.concatenate([np.asarray(lp, np.int64) for lp in loops])
    offs = np.concatenate([t.prim_offsets.astype(np.int64), t.prim_offsets[-1] + np.cumsum(sizes)])

    def pad(table, k):
        return {n: np.concatenate([a, np.zeros((k,) + a.shape[1:], a.dtype)]) for n, a in table.items()}

    prim = pad(geo.prim, len(loops))
    out = B.make(geo.P, np.concatenate([t.vertex_point, new_vp]), offs, dict(geo.point.items()),
                 pad(geo.vertex, len(new_vp)), prim, geo.detail)
    if group:
        old = groups.member(geo, "prim", group) if groups.attr_name(group) in geo.prim else np.zeros(t.nprims, bool)
        out = groups.write(out, "prim", group, np.r_[old, np.ones(len(loops), bool)])
    return out


INSIDE_TMP = "vfinsidetmp"                                  # working group: the faces the cuts created

OUT = OutGroups(
    OutGroup("inside", "Inside Faces", "inside", "prim", default=True,
             doc="The faces the cuts created (the caps): the inside surface of each piece"),
    OutGroup("outside", "Outside Faces", "outside", "prim", doc="The faces that came from the input's own surface"),
    OutGroup("cutpoints", "Cut Points", "cutpoints", "point", doc="The points that lie on a cut plane"),
    OutGroup("cutedges", "Cut Edges", "cutedges", "edge", doc="The edges that lie on a cut plane: the outline of every cut face"),
)
SPLIT_OUT = OutGroups(
    OutGroup("cutpoints", "Cut Points", "cutpoints", "point", doc="The points that lie on a cut plane"),
    OutGroup("cutedges", "Cut Edges", "cutedges", "edge", doc="The edges that lie on a cut plane (the open border each cut left)"),
)


def cut_edge_mask(geo, inside):
    """Bool per derived edge: used by at least one face in ``inside`` and at least one that is not (the seam between a
    cut face and the original surface). ``inside`` is a bool per primitive."""
    e = topo.edges(geo)
    ok = e.of_vertex >= 0
    pv = topo.prim_of_vertex(geo)[ok]
    ids = e.of_vertex[ok]
    has_in, has_out = np.zeros(len(e.points), bool), np.zeros(len(e.points), bool)
    np.logical_or.at(has_in, ids, np.asarray(inside, bool)[pv])
    np.logical_or.at(has_out, ids, ~np.asarray(inside, bool)[pv])
    return has_in & has_out


def voronoi_cells(geo, S, gap, cap, eps, prior=None):
    """Cut ``geo`` into the Voronoi cells of the points ``S``. Yields (cell index, piece, planes) for each non-empty cell;
    ``planes`` are the (origin, normal, other point) triples that cut it. With ``cap`` the openings are filled and the new
    faces are in the working group :data:`INSIDE_TMP` (``prior``: a bool per face of the input that already counts as inside)."""
    size = float(np.ptp(geo.P, axis=0).max()) if geo.npoints else 1.0
    for i in range(len(S)):
        piece, planes = geo, []
        if cap and groups.attr_name(INSIDE_TMP) not in piece.prim:
            piece = groups.write(piece, "prim", INSIDE_TMP, np.zeros(piece.nprims, bool) if prior is None else prior)
        dist = np.linalg.norm(S - S[i], axis=1)
        reach = float(np.linalg.norm(piece.P.astype(np.float64) - S[i], axis=1).max())
        for j in np.argsort(dist, kind="stable"):
            if j == i or dist[j] < 1e-9 * max(size, 1e-9):
                continue
            if dist[j] / 2.0 - gap / 2.0 > reach:          # this and all further planes miss the piece
                break
            normal = (S[j] - S[i]) / dist[j]
            origin = (S[i] + S[j]) / 2.0 - normal * (gap / 2.0)
            piece = clip(piece, origin, normal, "below", outgrp_above=False)     # no output groups wanted here
            if piece.nprims == 0:
                break
            planes.append((origin, normal, int(j)))
            if cap:
                piece = _cap_plane(piece, origin, normal, eps, INSIDE_TMP)
            reach = float(np.linalg.norm(piece.P.astype(np.float64) - S[i], axis=1).max())
        if piece.nprims:
            yield i, piece, planes


def on_planes(piece, planes, eps):
    """(bool per point lying on any of the planes, bool per edge lying in one of them)."""
    P = piece.P.astype(np.float64)
    pts = np.zeros(piece.npoints, bool)
    edg = np.zeros(len(topo.edges(piece).points), bool)
    ends = topo.edges(piece).points
    for origin, normal, *_ in planes:
        on = np.abs((P - origin) @ normal) <= eps
        pts |= on
        edg |= on[ends[:, 0]] & on[ends[:, 1]]
    return pts, edg


def weld_points(geo, tol):
    """Merge every pair of points closer than ``tol`` (exactly, by connected groups: no grid edges to straddle)."""
    from .. import spatial
    from ..geometry import Geometry, Topology
    i, j, _ = spatial.pairs_within(geo.P.astype(np.float64), tol)
    if not len(i):
        return geo
    label = spatial.components(geo.npoints, i, j)
    _, first = np.unique(label, return_index=True)
    order = np.argsort(first)
    rank = np.empty(len(order), np.int64)
    rank[order] = np.arange(len(order))
    remap = rank[label]
    keep = first[order]
    t = geo.topology
    out = Geometry(len(keep), Topology(remap[t.vertex_point].astype(np.int32), t.prim_offsets, t.closed))
    for name, a in geo.point.items():
        out.point[name] = a[keep]
    for name, a in geo.vertex.items():
        out.vertex[name] = a
    for name, a in geo.prim.items():
        out.prim[name] = a
    out.detail = dict(geo.detail)
    return out


def merge_clusters(cells, cluster, eps):
    """Cells whose points share a (non-negative) cluster number become one piece: the cut faces between them are dropped
    and their points welded. Returns the same kind of list as :func:`voronoi_cells`, the cluster under its first member."""
    from .fracture_tools import contacts
    from .group import keep_prims
    by = {}
    for i, piece, planes in cells:
        by.setdefault(int(cluster[i]) if cluster[i] >= 0 else -1 - i, []).append((i, piece, planes))
    out = []
    for key, members in by.items():
        if len(members) == 1:
            out.append(members[0])
            continue
        parts = []
        for i, piece, _planes in members:
            piece = piece.copy()
            piece.prim["vfcell"] = np.full(piece.nprims, i, np.int32)
            parts.append(piece)
        merged = merge(parts)
        inside = groups.member(merged, "prim", INSIDE_TMP).astype(bool)
        fa, fb, _ = contacts(merged, merged.prim["vfcell"].astype(np.int64))
        drop = np.zeros(merged.nprims, bool)
        drop[fa], drop[fb] = True, True
        drop &= inside                                       # only cut faces can face each other across a cluster
        merged = keep_prims(merged, ~drop, remove_unused_points=False)
        merged = weld_points(merged, max(eps * 5, 1e-9))
        del merged.prim["vfcell"]
        ids = {m[0] for m in members}
        planes = [pl for _i, _p, pls in members for pl in pls if pl[2] not in ids]
        out.append((min(ids), merged, planes))
    return sorted(out, key=lambda c: c[0])


def _finish_piece(piece, planes, i, piece_attrib, eps, og, spec, inside=None):
    """Piece attribute, then the output groups; the working group is replaced by the real one."""
    piece = piece.copy()                                    # may still be the (cached) input itself
    if piece_attrib:
        piece.prim[piece_attrib] = np.full(piece.nprims, i, np.int32)
    cache = {}

    def planar():
        if "p" not in cache:
            cache["p"] = on_planes(piece, planes, eps)
        return cache["p"]
    if inside is not None:
        piece = spec.write(piece, og, "inside", inside)
        piece = spec.write(piece, og, "outside", ~inside)
        del piece.prim[groups.attr_name(INSIDE_TMP)]
    piece = spec.write(piece, og, "cutpoints", lambda: planar()[0])
    return spec.write(piece, og, "cutedges", lambda: planar()[1])


def _empty():
    return B.make(np.zeros((0, 3), np.float32), np.zeros(0, np.int64), np.zeros(1, np.int64))


@node("voronoi_fracture", inputs=["geo", "points"], category="model", params=[
    Str("piece_attrib", "piece", doc="Primitive int attribute with each face's cell (the index of its point)"),
    Str("inside_group", "inside", doc="Name of the inside faces group; empty switches it off (the Output Groups entry does the same)"),
    Float("gap", 0.0, min=0.0, soft_max=0.1, doc="Shrink each cut plane towards its point: a gap between pieces"),
    Bool("create_inside", True, doc="Cap each cut so pieces are closed solids; off leaves them open (like voronoi_split)"),
    Bool("cluster_pieces", False, doc="Points with the same 'cluster' number (see voronoi_fracture_points) make one piece"),
    Str("cluster_attrib", "cluster", doc="The point int attribute that numbers the clusters (negative: not clustered)"),
    *OUT.params(),
])
def voronoi_fracture(geo, points, piece_attrib, inside_group, gap, create_inside=True, cluster_pieces=False,
                     cluster_attrib="cluster", **og):
    """Break a closed polygon mesh into the Voronoi cells of the ``points`` input (e.g. scattered inside it). Pieces
    are closed meshes sharing their cut faces; give each a different transform or hand them to a simulation."""
    B.require_polygons(geo, "voronoi_fracture")
    S = points.P.astype(np.float64)
    if geo.nprims == 0 or len(S) == 0:
        return geo
    if not inside_group:
        og["outgrp_inside"] = False
    elif inside_group != "inside":
        og["outgrp_inside_name"] = inside_group             # the older parameter still names the group
    size = float(np.ptp(geo.P, axis=0).max()) if geo.npoints else 1.0
    eps = 1e-6 * max(size, 1e-9)
    prior = None
    name = og.get("outgrp_inside_name", "inside")
    if create_inside and name and groups.attr_name(name) in geo.prim:      # faces already inside (an earlier fracture) stay inside
        prior = groups.member(geo, "prim", name).astype(bool)
    cells = list(voronoi_cells(geo, S, gap, bool(create_inside), eps, prior))
    if cluster_pieces:
        cl = points.point.get(cluster_attrib)
        if cl is None:
            raise ValueError(f"cluster_pieces needs a point attribute {cluster_attrib!r} on the points (voronoi_fracture_points writes it)")
        if create_inside:
            cells = merge_clusters(cells, cl.astype(np.int64), eps)
    pieces = []
    for i, piece, planes in cells:
        inside = groups.member(piece, "prim", INSIDE_TMP).astype(bool) if create_inside else None
        pieces.append(_finish_piece(piece, planes, i, piece_attrib, eps, og, OUT, inside))
    return merge(pieces) if pieces else _empty()


@node("voronoi_split", inputs=["geo", "points"], category="model", params=[
    Str("piece_attrib", "piece", doc="Primitive int attribute with each face's cell (the index of its point)"),
    *SPLIT_OUT.params(),
])
def voronoi_split(geo, points, piece_attrib, **og):
    """Split any polygon geometry (open sheets, soup, solids) by the Voronoi cells of the ``points`` input (Houdini's
    Voronoi Split): faces are cut where they cross a bisector plane and nothing is capped, so each piece keeps only
    the surface that was already there. Use it on thin or open geometry, ``voronoi_fracture`` on solids."""
    B.require_polygons(geo, "voronoi_split")
    S = points.P.astype(np.float64)
    if geo.nprims == 0 or len(S) == 0:
        return geo
    size = float(np.ptp(geo.P, axis=0).max()) if geo.npoints else 1.0
    eps = 1e-6 * max(size, 1e-9)
    pieces = [_finish_piece(piece, planes, i, piece_attrib, eps, og, SPLIT_OUT)
              for i, piece, planes in voronoi_cells(geo, S, 0.0, False, eps)]
    return merge(pieces) if pieces else _empty()


@node("exploded_view", inputs=["geo"], category="deform", params=[
    Str("piece_attrib", "piece", doc="Primitive attribute naming each face's piece"),
    Float("scale", 0.5, soft_min=0.0, soft_max=3.0, doc="Move pieces this fraction of their centre's offset outwards"),
    Vec3("center", (0.0, 0.0, 0.0), doc="Pieces move away from this point"),
])
def exploded_view(geo, piece_attrib, scale, center):
    """Push pieces apart (Houdini's Exploded View): each moves along the line from ``center`` through its centroid.
    Points shared by several pieces follow the first."""
    a = geo.prim.get(piece_attrib)
    if a is None:
        raise ValueError(f"no primitive attribute {piece_attrib!r} (fracture first, or use connectivity)")
    t = geo.topology
    key = np.unique(rows(a)[:, 0], return_inverse=True)[1].reshape(-1)
    pv = np.repeat(np.arange(t.nprims), t.prim_sizes())
    vp = t.vertex_point.astype(np.int64)
    k = int(key.max()) + 1 if len(key) else 0
    P = geo.P.astype(np.float64)
    cnt = np.maximum(np.bincount(key[pv], minlength=k), 1)
    cen = np.stack([np.bincount(key[pv], weights=P[vp, c], minlength=k) for c in range(3)], 1) / cnt[:, None]
    owner = np.full(geo.npoints, -1, np.int64)
    owner[vp[::-1]] = key[pv][::-1]                          # each point follows its first face's piece
    move = np.where(owner[:, None] >= 0, (cen[np.maximum(owner, 0)] - np.asarray(center)) * scale, 0.0)
    out = geo.copy()
    out.point["P"] = (P + move).astype(np.float32)
    return out
