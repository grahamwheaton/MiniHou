"""Voronoi fracture (Houdini's Voronoi Fracture): break a closed mesh into the Voronoi cells of a set of points.

Each cell is the mesh clipped by the bisector planes between its point and the others, nearest first. A plane further
from the point than the farthest corner of the piece left cannot cut it, so the loop stops there: the cells are exact,
not limited to k neighbours. Openings left by each cut are capped (only loops lying on that plane, so holes the
input already had stay open); caps form the group ``inside`` and every face gets the int ``piece`` of its cell.
"""
import numpy as np

from .. import groups
from ..geometry import rows
from ..sdk import Float, Str, Vec3, node
from . import _build as B
from .merge import merge
from .model import boundary_loops, clip


def _cap_plane(geo, origin, normal, eps, group):
    """Fill the boundary loops of ``geo`` that lie on the plane, as faces in ``group``."""
    try:
        loops = boundary_loops(geo)
    except ValueError:                                     # a pinched boundary: leave it open rather than fail
        return geo
    d = np.abs((geo.P.astype(np.float64) - origin) @ normal)
    loops = [lp for lp in loops if d[lp].max() <= eps]
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


@node("voronoi_fracture", inputs=["geo", "points"], category="model", params=[
    Str("piece_attrib", "piece", doc="Primitive int attribute with each face's cell (the index of its point)"),
    Str("inside_group", "inside", doc="Primitive group for the new inside faces (empty: none)"),
    Float("gap", 0.0, min=0.0, soft_max=0.1, doc="Shrink each cut plane towards its point: a gap between pieces"),
])
def voronoi_fracture(geo, points, piece_attrib, inside_group, gap):
    """Break a closed polygon mesh into the Voronoi cells of the ``points`` input (e.g. scattered inside it). Pieces
    are closed meshes sharing their cut faces; give each a different transform or hand them to a simulation."""
    B.require_polygons(geo, "voronoi_fracture")
    S = points.P.astype(np.float64)
    if geo.nprims == 0 or len(S) == 0:
        return geo
    size = float(np.ptp(geo.P, axis=0).max()) if geo.npoints else 1.0
    eps = 1e-6 * max(size, 1e-9)
    pieces = []
    for i in range(len(S)):
        piece = geo
        if inside_group and groups.attr_name(inside_group) not in piece.prim:
            piece = groups.write(piece, "prim", inside_group, np.zeros(piece.nprims, bool))
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
            piece = _cap_plane(piece, origin, normal, eps, inside_group)
            reach = float(np.linalg.norm(piece.P.astype(np.float64) - S[i], axis=1).max())
        if piece.nprims == 0:
            continue
        if piece_attrib:
            piece = piece.copy()                            # may still be the (cached) input itself
            piece.prim[piece_attrib] = np.full(piece.nprims, i, np.int32)
        pieces.append(piece)
    if not pieces:
        return B.make(np.zeros((0, 3), np.float32), np.zeros(0, np.int64), np.zeros(1, np.int64))
    return merge(pieces)


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
