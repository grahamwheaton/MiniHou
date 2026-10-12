"""The rest of Houdini's RBD fracture toolbox: Voronoi Fracture Points, Voronoi Adjacency and the contact search that
Glue Cluster builds on. (Voronoi Fracture and Voronoi Split are in :mod:`fracture`.)"""
import numpy as np

from .. import spatial, topo
from ..geometry import Geometry
from ..outgroups import OutGroup, OutGroups
from ..sdk import Bool, Enum, Float, Input, Int, Str, node
from . import _build as B
from ._util import fan_triangulate
from .attrib_tools import piece_labels


# ------------------------------------------------------------------------------------------------------- helpers

def piece_ids(geo, attrib):
    """Int piece number per primitive: the attribute when named and present, else connectivity (shared points)."""
    if attrib:
        a = geo.prim.get(attrib)
        if a is None:
            raise ValueError(f"no primitive attribute {attrib!r}: fracture first, or leave piece attribute empty to use connectivity")
        if a.ndim != 1 or a.dtype.kind not in "iu":
            raise ValueError(f"{attrib!r} must be a scalar integer primitive attribute")
        return a.astype(np.int64)
    return piece_labels(geo)[1].astype(np.int64)


def prim_geometry(geo):
    """(centroid, unit normal, area) per primitive, float64 (area from the Newell normal)."""
    from .normal import prim_normals
    n = prim_normals(geo).astype(np.float64)
    area = 0.5 * np.linalg.norm(n, axis=1)
    unit = n / np.maximum(np.linalg.norm(n, axis=1), 1e-30)[:, None]
    return topo.prim_centroids(geo), unit, area


def contacts(geo, pieces, max_gap=0.0):
    """Faces of different pieces that lie against each other (the two sides of a cut): returns
    ``(face_a, face_b, area)`` with ``pieces[face_a] < pieces[face_b]``. Two faces touch when their normals oppose, they
    are at most ``max_gap`` apart along the normal, line up in the plane of the face, and their areas agree
    (exactly when there is no gap, within a factor of two when there is one)."""
    c, n, area = prim_geometry(geo)
    size = float(np.ptp(geo.P, axis=0).max()) if geo.npoints else 1.0
    tol = 1e-5 * max(size, 1e-9)
    # faces are matched through their centroids, which coincide in the plane even when the planes are a gap apart
    slack = tol * 4 if max_gap <= 0 else 2 * max_gap + tol * 4        # with a gap the two caps are cut differently
    i, j, _ = spatial.pairs_within(c, max_gap + slack + tol)
    if not len(i):
        return np.zeros(0, np.int64), np.zeros(0, np.int64), np.zeros(0)
    d = c[j] - c[i]
    along = np.einsum("ij,ij->i", d, n[i])
    side = np.linalg.norm(d - along[:, None] * n[i], axis=1)
    ok = (pieces[i] != pieces[j]) & (np.einsum("ij,ij->i", n[i], n[j]) < -0.999)
    ok &= (np.abs(along) <= max_gap + tol) & (side <= slack)
    ratio = np.minimum(area[i], area[j]) / np.maximum(np.maximum(area[i], area[j]), 1e-300)
    ok &= ratio >= (0.5 if max_gap > 0 else 1 - 1e-3)
    i, j = i[ok], j[ok]
    swap = pieces[i] > pieces[j]
    i, j = np.where(swap, j, i), np.where(swap, i, j)
    return i, j, np.minimum(area[i], area[j])


def piece_pairs(geo, pieces, max_gap=0.0):
    """Contacts summed per pair of pieces: ``(piece_a, piece_b, area, faces)`` sorted by pair, with a < b."""
    i, j, area = contacts(geo, pieces, max_gap)
    if not len(i):
        z = np.zeros(0, np.int64)
        return z, z, np.zeros(0), z
    a, b = pieces[i], pieces[j]
    key = np.stack([a, b], 1)
    uniq, inv = np.unique(key, axis=0, return_inverse=True)
    inv = inv.reshape(-1)
    return uniq[:, 0], uniq[:, 1], np.bincount(inv, weights=area), np.bincount(inv)


def piece_centres(geo, pieces, ids):
    """Mean of the points each piece uses, for the piece numbers ``ids``."""
    vp = geo.topology.vertex_point.astype(np.int64)
    pv = topo.prim_of_vertex(geo)
    P = geo.P.astype(np.float64)[vp]
    slot = np.searchsorted(ids, pieces[pv])
    cnt = np.maximum(np.bincount(slot, minlength=len(ids)), 1)
    return np.stack([np.bincount(slot, weights=P[:, k], minlength=len(ids)) for k in range(3)], 1) / cnt[:, None]


def link_geometry(centres, ids, a, b, attrs):
    """Points at the piece centres joined by two-point open curves between pieces ``a`` and ``b``."""
    slot = {int(p): k for k, p in enumerate(ids)}
    ends = np.array([[slot[int(x)], slot[int(y)]] for x, y in zip(a, b)], np.int64).reshape(-1, 2)
    out = B.make(centres.astype(np.float32), ends.reshape(-1), B.offsets_from_sizes(np.full(len(ends), 2)),
                 {"piece": ids.astype(np.int32)}, None, {k: v for k, v in attrs.items()}, None,
                 np.zeros(len(ends), bool))
    return out, ends


# ------------------------------------------------------------------------------------------ voronoi fracture points

POINTS_OUT = OutGroups(
    OutGroup("interior", "Interior Points", "interior", "point", doc="Points placed inside the shape"),
    OutGroup("surface", "Surface Points", "surface", "point", doc="Points placed on the shape's surface"),
    OutGroup("clustered", "Clustered Points", "clustered", "point", doc="Points placed around a cluster centre"),
    OutGroup("extra", "Extra Input Points", "extrapoints", "point", doc="Points that came in on the second input"),
)


def _shape_sampler(geo, rng, scale):
    """f(count) -> points uniformly inside the closed mesh (rejection sampling in its bounding box)."""
    from ..raycast import TriGrid, inside
    tris, _ = fan_triangulate(geo.topology)
    P = geo.P.astype(np.float64)
    grid = TriGrid(P[geo.topology.vertex_point[tris]])
    lo, ext = P.min(0), np.maximum(np.ptp(P, axis=0), 1e-12)

    def sample(count, centre=None, sigma=None):
        found, have, frac = [], 0, 0.5
        for _ in range(48):
            if have >= count:
                break
            n = int(min(max(256, (count - have) / max(frac, 0.02) * 1.3), 400_000))
            cand = lo + rng.random((n, 3)) * ext if centre is None else centre + rng.normal(size=(n, 3)) * sigma
            m = inside(grid, cand, scale)
            frac = max(float(m.mean()), 1e-3)
            found.append(cand[m])
            have += int(m.sum())
        return np.concatenate(found)[:count] if found else np.zeros((0, 3))
    return sample


def _surface_points(geo, rng, count):
    tris, _ = fan_triangulate(geo.topology)
    P = geo.P.astype(np.float64)
    T = P[geo.topology.vertex_point[tris]]
    area = 0.5 * np.linalg.norm(np.cross(T[:, 1] - T[:, 0], T[:, 2] - T[:, 0]), axis=1)
    if not len(T) or area.sum() <= 0 or count <= 0:
        return np.zeros((0, 3))
    t = np.minimum(np.searchsorted(np.cumsum(area), rng.random(count) * area.sum(), side="right"), len(T) - 1)
    r1, r2 = rng.random(count), rng.random(count)
    flip = r1 + r2 > 1
    r1, r2 = np.where(flip, 1 - r1, r1), np.where(flip, 1 - r2, r2)
    return T[t, 0] + (T[t, 1] - T[t, 0]) * r1[:, None] + (T[t, 2] - T[t, 0]) * r2[:, None]


def _thin(pts, distance):
    """Greedy blue-noise thinning: drop a point when an earlier kept one is within ``distance``."""
    if distance <= 0 or len(pts) < 2:
        return np.ones(len(pts), bool)
    keep = np.ones(len(pts), bool)
    i, j, _ = spatial.pairs_within(pts, distance)
    for a, b in sorted(zip(i.tolist(), j.tolist())):
        if keep[a] and keep[b]:
            keep[b] = False
    return keep


@node("voronoi_fracture_points", inputs=["geo", Input("points", optional=True)], category="model", params=[
    Int("count", 20, min=1, soft_max=500, doc="Number of points to scatter (the extra input's points come on top)"),
    Int("seed", 0),
    Float("surface_fraction", 0.0, min=0.0, max=1.0, doc="Share of the points placed on the surface instead of inside: more, smaller "
                                                         "pieces at the outside, as in a shattering shell"),
    Float("min_distance", 0.0, min=0.0, soft_max=1.0, doc="Keep points at least this far apart (0: off)"),
    Int("relax", 0, min=0, max=50, doc="Lloyd relaxation passes on the interior points: pieces become more even in size"),
    Float("cluster_amount", 0.0, min=0.0, max=1.0, doc="Share of the interior points gathered round random cluster centres, so "
                                                       "the shape breaks into fine pieces in a few places"),
    Int("clusters", 3, min=1, max=64, doc="How many cluster centres"),
    Float("cluster_radius", 0.25, min=1e-6, soft_max=2.0, doc="Spread of each cluster"),
    Str("piece_attrib", "piece", doc="Point int attribute numbering the points (empty: none)"),
    *POINTS_OUT.params(),
])
def voronoi_fracture_points(geo, points, count, seed, surface_fraction, min_distance, relax, cluster_amount, clusters,
                            cluster_radius, piece_attrib, **og):
    """Fracture points for ``voronoi_fracture`` (Houdini's Voronoi Fracture Points): scattered inside a closed mesh, with
    optional surface points, thinning, relaxation, clustering and points from a second input. Connect the result to
    the fracture's second input."""
    B.require_polygons(geo, "voronoi_fracture_points")
    if geo.nprims == 0:
        raise ValueError("voronoi_fracture_points needs a closed mesh to scatter inside")
    rng = np.random.default_rng([int(seed) & 0xFFFFFFFF, 29])
    scale = float(np.ptp(geo.P, axis=0).max())
    n_surf = int(round(count * surface_fraction))
    n_cluster = int(round((count - n_surf) * cluster_amount))
    n_inner = count - n_surf - n_cluster
    if n_inner + n_cluster and np.any(topo.edges(geo).count != 2):
        raise ValueError("voronoi_fracture_points needs a closed mesh to scatter inside (set surface_fraction to 1 for an open one)")
    sample = _shape_sampler(geo, rng, scale)
    inner = sample(n_inner)
    clustered = np.zeros((0, 3))
    if n_cluster:
        centres = sample(clusters)
        if len(centres):
            share = np.array_split(np.arange(n_cluster), len(centres))
            clustered = np.concatenate([sample(len(s), centre, cluster_radius * scale) for s, centre in zip(share, centres)])
    for _ in range(int(relax)):                              # Lloyd: move each point to the centre of its cell's samples
        if len(inner) < 2:
            break
        cloud = sample(max(2000, len(inner) * 60))
        owner, _d = spatial.nearest(inner, cloud)
        cnt = np.bincount(owner, minlength=len(inner))
        mean = np.stack([np.bincount(owner, weights=cloud[:, k], minlength=len(inner)) for k in range(3)], 1)
        inner = np.where(cnt[:, None] > 0, mean / np.maximum(cnt, 1)[:, None], inner)
    surface = _surface_points(geo, rng, n_surf)
    extra = points.P.astype(np.float64) if points is not None and points.npoints else np.zeros((0, 3))
    kinds = [("interior", inner), ("surface", surface), ("clustered", clustered), ("extra", extra)]
    P = np.concatenate([k[1] for k in kinds]) if sum(len(k[1]) for k in kinds) else np.zeros((0, 3))
    label = np.concatenate([np.full(len(k[1]), n, np.int8) for n, k in enumerate(kinds)]) if len(P) else np.zeros(0, np.int8)
    keep = _thin(P, min_distance)
    keep[label == 3] = True                                  # the user's own points always stay
    P, label = P[keep], label[keep]
    out = Geometry.from_arrays(P.astype(np.float32))
    if piece_attrib:
        out.point[piece_attrib] = np.arange(len(P), dtype=np.int32)
    for n, key in enumerate(("interior", "surface", "clustered", "extra")):
        out = POINTS_OUT.write(out, og, key, label == n)
    return out


# ----------------------------------------------------------------------------------------------- voronoi adjacency

ADJ_OUT = OutGroups(
    OutGroup("connected", "Connected Pieces", "connected", "point", doc="Piece centres that touch at least one other piece"),
    OutGroup("isolated", "Isolated Pieces", "isolated", "point", doc="Piece centres that touch no other piece"),
    OutGroup("strong", "Strong Links", "stronglinks", "prim", doc="Links whose contact area is at least the 'strong area' fraction of "
                                                                  "the biggest one"),
)


@node("voronoi_adjacency", inputs=["geo", Input("points", optional=True)], category="model", params=[
    Str("piece_attrib", "piece", doc="Primitive int attribute with each face's piece (empty: connected pieces)"),
    Float("max_gap", 0.0, min=0.0, soft_max=0.1, doc="Pieces whose faces are up to this far apart still count as touching (use the "
                                                    "fracture's gap here)"),
    Float("min_area", 0.0, min=0.0, doc="Ignore contacts smaller than this"),
    Float("strong_area", 0.5, min=0.0, max=1.0, doc="Share of the largest contact above which a link is strong"),
    *ADJ_OUT.params(),
])
def voronoi_adjacency(geo, points, piece_attrib, max_gap, min_area, strong_area, **og):
    """Which pieces touch (Houdini's Voronoi Adjacency): a point at each piece's centre (or at the matching point of the
    second input, numbered by the piece) and a two-point curve between pieces that share a cut face. Prim attributes: ``piece_a``,
    ``piece_b`` and ``area`` (the contact area); points carry ``piece`` and the number of ``neighbors``."""
    B.require_polygons(geo, "voronoi_adjacency")
    pieces = piece_ids(geo, piece_attrib)
    ids = np.unique(pieces)
    if points is not None and points.npoints:
        if int(ids.max(initial=-1)) >= points.npoints or int(ids.min(initial=0)) < 0:
            raise ValueError("the points input has fewer points than there are pieces")
        centres = points.P.astype(np.float64)[ids]
    else:
        centres = piece_centres(geo, pieces, ids)
    a, b, area, _faces = piece_pairs(geo, pieces, max_gap)
    keep = area >= max(min_area, 1e-300)
    a, b, area = a[keep], b[keep], area[keep]
    out, ends = link_geometry(centres, ids, a, b, {"piece_a": a.astype(np.int32), "piece_b": b.astype(np.int32),
                                                    "area": area.astype(np.float32)})
    degree = np.bincount(ends.reshape(-1), minlength=len(ids)) if len(ends) else np.zeros(len(ids), np.int64)
    out.point["neighbors"] = degree.astype(np.int32)
    out = ADJ_OUT.write(out, og, "connected", degree > 0)
    out = ADJ_OUT.write(out, og, "isolated", degree == 0)
    return ADJ_OUT.write(out, og, "strong", lambda: area >= strong_area * (area.max() if len(area) else 0))


# -------------------------------------------------------------------------------------------------- boolean fracture

BF_OUT = OutGroups(
    OutGroup("inside", "Inside Faces", "inside", "prim", default=True,
             doc="The faces the cutters created: the new inside surface of every piece"),
    OutGroup("outside", "Outside Faces", "outside", "prim", doc="The faces that came from the input's own surface"),
    OutGroup("cutter", "Cutter Pieces", "cutterpieces", "prim", doc="All faces of the pieces that were inside a cutter"),
    OutGroup("remainder", "Remainder Piece", "remainder", "prim", doc="All faces of what was left outside every cutter"),
    OutGroup("cutpoints", "Cut Points", "cutpoints", "point", doc="The points where the inside meets the outside"),
    OutGroup("cutedges", "Cut Edges", "cutedges", "edge", doc="The edges where the inside meets the outside"),
)
_FROM_B = "vfcutter"


def _split_connected(geo):
    """The connected pieces of ``geo`` as separate geometries (points not shared)."""
    from .group import keep_prims
    labels = piece_labels(geo)[1]
    return [keep_prims(geo, labels == k) for k in range(int(labels.max()) + 1)] if geo.nprims else []


def _bool(a, b, operation):
    """a (op) b with the faces that came from b marked; returns (geometry, bool per prim: from b)."""
    from .boolean import boolean
    r = boolean(a, b, operation, outgrp_from_a=False, outgrp_from_b=True, outgrp_from_b_name=_FROM_B,
                outgrp_seam=False, outgrp_seampoints=False)
    key = "group_" + _FROM_B
    mark = r.prim[key].astype(bool) if key in r.prim else np.zeros(r.nprims, bool)
    if key in r.prim:
        r = r.copy()
        del r.prim[key]
    return r, mark


@node("boolean_fracture", inputs=["geo", "cutters"], category="model", params=[
    Enum("keep", ["both", "inside", "outside"], doc="inside: only the pieces cut out by the cutters; outside: only what is left"),
    Enum("mode", ["each_cutter", "union"], doc="each_cutter: one piece per cutter (earlier cutters win where they overlap); "
                                               "union: one inside piece for all of them"),
    Str("piece_attrib", "piece", doc="Primitive int attribute numbering the connected pieces (empty: none)"),
    Str("cutter_attrib", "cutter", doc="Primitive int attribute: which cutter made the piece (-1: the remainder; empty: none)"),
    *BF_OUT.params(),
])
def boolean_fracture(geo, cutters, keep, mode, piece_attrib, cutter_attrib, **og):
    """Break a closed mesh with other closed meshes (Houdini's Boolean Fracture): every connected body of the cutters
    carves out a piece (the input intersected with it) and what no cutter touched is the remainder. The new surface of
    each piece (the cutter's) is the ``inside`` group; the input's own surface is ``outside``. Pieces are closed solids, ready for
    ``rbd_configure``."""
    for g, nm in ((geo, "input"), (cutters, "cutters")):
        B.require_polygons(g, f"boolean_fracture {nm}")
    if geo.nprims == 0 or cutters.nprims == 0:
        return geo
    bodies = _split_connected(cutters)
    if mode == "union":
        bodies = [cutters]
    base = geo.copy()
    base.prim["vf_inside"] = np.zeros(base.nprims, np.int32)
    rem, parts = base, []

    def inside_of(g, mark):
        return (g.prim["vf_inside"] != 0) | mark if "vf_inside" in g.prim else mark

    for k, body in enumerate(bodies):
        if rem.nprims == 0:
            break
        cut, mark = _bool(rem, body, "intersect")
        if cut.nprims:
            cut = cut.copy()
            ins = inside_of(cut, mark)
            cut.prim["vf_inside"] = ins.astype(np.int32)
            parts.append((cut, k, True))
        rem, mark = _bool(rem, body, "subtract")
        if rem.nprims:
            rem = rem.copy()
            rem.prim["vf_inside"] = inside_of(rem, mark).astype(np.int32)
    if rem.nprims:
        parts.append((rem, -1, False))
    out = []
    for g, k, is_cutter in parts:
        if (keep == "inside" and not is_cutter) or (keep == "outside" and is_cutter):
            continue
        ins = g.prim["vf_inside"].astype(bool)
        g = g.copy()
        del g.prim["vf_inside"]
        if cutter_attrib:
            g.prim[cutter_attrib] = np.full(g.nprims, k, np.int32)
        seam = cut_edge_mask_for(g, ins)
        g = BF_OUT.write(g, og, "inside", ins)
        g = BF_OUT.write(g, og, "outside", ~ins)
        g = BF_OUT.write(g, og, "cutter", np.full(g.nprims, is_cutter))
        g = BF_OUT.write(g, og, "remainder", np.full(g.nprims, not is_cutter))
        g = BF_OUT.write(g, og, "cutpoints", lambda g=g, seam=seam: _points_of_edges(g, seam))
        out.append(BF_OUT.write(g, og, "cutedges", seam))
    if not out:
        return B.make(np.zeros((0, 3), np.float32), np.zeros(0, np.int64), np.zeros(1, np.int64))
    from .merge import merge
    res = merge(out)
    if piece_attrib:
        res = res.copy()
        res.prim[piece_attrib] = piece_labels(res)[1].astype(np.int32)
    return res


def cut_edge_mask_for(geo, inside):
    from .fracture import cut_edge_mask
    return cut_edge_mask(geo, inside)


def _points_of_edges(geo, edge_mask):
    m = np.zeros(geo.npoints, bool)
    m[topo.edges(geo).points[np.asarray(edge_mask, bool)].ravel()] = True
    return m


# ---------------------------------------------------------------------------------------------- convex decomposition

def mesh_volume(geo):
    """Signed volume of a closed mesh (positive when faces point outwards)."""
    tris, _ = fan_triangulate(geo.topology)
    if not len(tris):
        return 0.0
    T = geo.P.astype(np.float64)[geo.topology.vertex_point[tris]]
    return float(np.einsum("ij,ij->i", T[:, 0], np.cross(T[:, 1], T[:, 2])).sum() / 6.0)


def hull_volume(geo):
    from .. import hull as hl
    P = geo.P.astype(np.float64)
    tri = hl.convex_hull(P)
    if not len(tri):
        return 0.0
    T = P[tri]
    return float(np.einsum("ij,ij->i", T[:, 0], np.cross(T[:, 1], T[:, 2])).sum() / 6.0)


def concave_volume(geo):
    """(volume, how much of the convex hull is not filled by the mesh)."""
    v = mesh_volume(geo)
    return v, max(hull_volume(geo) - v, 0.0)


def split_planes(piece, limit):
    """Candidate cutting planes, best first: the planes of the faces on each reflex (concave) edge, which is how a
    polyhedron is taken apart into convex parts. Falls back to the three axes through the centre."""
    from .bevel import twins
    c, n, area = prim_geometry(piece)
    size = float(np.ptp(piece.P, axis=0).max())
    pv = topo.prim_of_vertex(piece)
    tw = twins(piece)
    he = np.flatnonzero((tw >= 0))
    he = he[he < tw[he]]
    p, q = pv[he], pv[tw[he]]
    out_p = np.einsum("ij,ij->i", c[q] - c[p], n[p])            # the neighbour sits in front of this face: a reflex edge
    out_q = np.einsum("ij,ij->i", c[p] - c[q], n[q])
    reflex = (out_p > 1e-6 * size) | (out_q > 1e-6 * size)
    score = np.zeros(piece.nprims)
    np.add.at(score, p[reflex], np.maximum(out_p[reflex], 0) + 1e-12)
    np.add.at(score, q[reflex], np.maximum(out_q[reflex], 0) + 1e-12)
    score *= area
    planes, seen = [], set()
    for f in np.argsort(-score, kind="stable"):
        if score[f] <= 0 or len(planes) >= limit:
            break
        key = tuple(np.round(np.r_[n[f], n[f] @ c[f] / max(size, 1e-12)], 4))
        if key in seen:
            continue
        seen.add(key)
        planes.append((c[f], n[f]))
    mid = piece.P.astype(np.float64).mean(0)                    # always also the planes through the middle: they take a ring in half
    centred = [(mid, np.eye(3)[k]) for k in range(3)]
    return planes + centred


CD_OUT = OutGroups(
    OutGroup("inside", "Inside Faces", "inside", "prim", default=True, doc="The faces the cuts created"),
    OutGroup("outside", "Outside Faces", "outside", "prim", doc="The faces that came from the input's own surface"),
    OutGroup("cutpoints", "Cut Points", "cutpoints", "point", doc="The points where the inside meets the outside"),
    OutGroup("cutedges", "Cut Edges", "cutedges", "edge", doc="The edges where the inside meets the outside"),
)


def _half(piece, origin, normal, side, tol):
    """One side of a plane cut. A face lying in the plane belongs to the half it faces away from (its outward normal
    points out of that half), so it is dropped from the other one."""
    from .group import keep_prims
    from .model import clip
    h = clip(piece, origin, normal, side, outgrp_above=False)
    if h.nprims == 0:
        return h
    d = (h.P.astype(np.float64) - origin) @ normal
    flat = np.zeros(h.nprims, bool)
    off = np.abs(d)[h.topology.vertex_point] > tol
    flat = np.bincount(topo.prim_of_vertex(h), weights=off, minlength=h.nprims) == 0
    _, nrm, _ = prim_geometry(h)
    wrong = flat & ((nrm @ normal) * (1 if side == "above" else -1) > 0)       # 'above' keeps flat faces that face down
    return keep_prims(h, ~wrong) if wrong.any() else h


def _try_split(piece, plane, min_volume, eps):
    from .fracture import INSIDE_TMP, _cap_plane
    origin, normal = plane
    halves = []
    for side in ("above", "below"):
        h = _half(piece, origin, normal, side, eps)
        if h.nprims == 0:
            return None
        halves.append(_cap_plane(h, origin, normal, eps, INSIDE_TMP))
    out = []
    for h in halves:
        for comp in _split_connected(h):
            if mesh_volume(comp) <= 0 or np.any(topo.edges(comp).count != 2):   # the cut did not close properly
                return None
            out.append(comp)
    if min(mesh_volume(c) for c in out) < min_volume:
        return None
    return out


@node("convex_decomposition", inputs=["geo"], category="model", params=[
    Float("max_concavity", 0.02, min=0.0, max=1.0, doc="A piece is convex enough when its convex hull holds at most this share "
                                                       "more volume than it does"),
    Int("max_pieces", 32, min=1, max=1000, doc="Stop splitting at this many pieces"),
    Float("min_volume", 0.001, min=0.0, max=0.5, doc="No cut may leave a piece smaller than this share of the whole shape"),
    Int("search", 16, min=1, max=200, doc="Cutting planes tried for each split (more is slower and finds better cuts)"),
    Str("piece_attrib", "piece", doc="Primitive int attribute numbering the pieces (empty: none)"),
    *CD_OUT.params(),
])
def convex_decomposition(geo, max_concavity, max_pieces, min_volume, search, piece_attrib, **og):
    """Cut a closed concave mesh into nearly convex pieces (Houdini's Convex Decomposition, for simulation collision shapes).
    The piece with the most volume missing from its convex hull is cut next, along the plane of a face beside one of its
    reflex edges (the cut that removes the most concavity wins), until every piece is within ``max_concavity`` or there are
    ``max_pieces``. Each cut is capped, so the pieces are closed solids; the caps are the ``inside`` group."""
    from .fracture import INSIDE_TMP, cut_edge_mask
    from .group import keep_prims
    from .merge import merge
    B.require_polygons(geo, "convex_decomposition")
    if geo.nprims == 0:
        return geo
    if np.any(topo.edges(geo).count != 2):
        raise ValueError("convex_decomposition needs a closed mesh (every edge shared by two faces)")
    from .. import groups
    start = groups.write(geo, "prim", INSIDE_TMP, np.zeros(geo.nprims, bool))
    total = abs(mesh_volume(start))
    if total <= 0:
        raise ValueError("convex_decomposition: the mesh has no volume (faces pointing inwards?)")
    size = float(np.ptp(geo.P, axis=0).max())
    eps = 5e-7 * max(size, 1e-9) + 1e-7
    work = []                                                       # (missing volume, tiebreak, geometry)
    done = []
    counter = 0

    def add(g):
        nonlocal counter
        v, missing = concave_volume(g)
        counter += 1
        (work if missing / max(v, 1e-300) > max_concavity else done).append((missing, counter, g))
    for comp in _split_connected(start):
        add(comp)
    while work and len(work) + len(done) < max_pieces:
        work.sort(key=lambda w: (-w[0], w[1]))
        missing, _, piece = work.pop(0)
        best = None
        parent = missing / max(mesh_volume(piece), 1e-300)
        for plane in split_planes(piece, search):
            parts = _try_split(piece, plane, min_volume * total, eps)
            if parts is None:
                continue
            stats = [concave_volume(p) for p in parts]
            left = sum(m for _v, m in stats)
            worst = max(m / max(v, 1e-300) for v, m in stats)
            if best is None or (left, worst) < best[0]:
                best = ((left, worst), parts, worst)
            if left <= 1e-9 * total:
                break
        # a cut is worth making when it removes concavity, or at least leaves no part more concave than the whole (a ring
        # needs two cuts before it gets better)
        if best is None or (best[0][0] >= missing * (1 - 1e-6) and best[2] > parent * (1 + 1e-3)):
            done.append((missing, counter, piece))
            continue
        for p in best[1]:
            add(p)
    pieces = [g for _m, _c, g in sorted(work + done, key=lambda w: w[1])]
    out = []
    for i, p in enumerate(pieces):
        ins = groups.member(p, "prim", INSIDE_TMP).astype(bool)
        p = p.copy()
        del p.prim[groups.attr_name(INSIDE_TMP)]
        if piece_attrib:
            p.prim[piece_attrib] = np.full(p.nprims, i, np.int32)
        seam = cut_edge_mask(p, ins)
        p = CD_OUT.write(p, og, "inside", ins)
        p = CD_OUT.write(p, og, "outside", ~ins)
        p = CD_OUT.write(p, og, "cutpoints", lambda p=p, seam=seam: _points_of_edges(p, seam))
        out.append(CD_OUT.write(p, og, "cutedges", seam))
    return merge(out)


# ------------------------------------------------------------------------------------------------------------ assemble

ASM_OUT = OutGroups(
    OutGroup("unshared", "Split Points", "splitpoints", "point",
             doc="The points that were copied so that each piece has its own (every copy after the first)"),
    OutGroup("shared", "Shared Points", "sharedpoints", "point", doc="The points that two or more pieces used before they were split"),
)


@node("assemble", inputs=["geo"], category="model", params=[
    Enum("source", ["attribute", "connectivity", "groups"], doc="How pieces are told apart: an integer primitive attribute, the "
                                                                "connected parts, or each primitive group matching 'groups'"),
    Str("piece_attrib", "piece", doc="attribute: the primitive int attribute to read"),
    Str("groups", "*", doc="groups: group names or wildcards (each group is a piece; faces in none become a last piece)"),
    Str("output_attrib", "piece", doc="Primitive int attribute to write"),
    Bool("renumber", True, doc="Number the pieces 0, 1, 2... in order of first appearance"),
    Bool("unshare_points", True, doc="Give every piece its own points (copy the ones pieces share); simulation needs this"),
    Enum("pivot", ["centroid", "origin", "bounds_bottom"], doc="Where each piece's pivot is: written as the primitive vector 'pivot'"),
    Str("name_prefix", "piece", doc="Pieces are named prefix0, prefix1...: the list is the detail attribute 'piece_names'"),
    Bool("piece_groups", False, doc="Also make a primitive group per piece, named like the piece"),
    Bool("remove_source_groups", False, doc="groups: delete the groups the pieces came from"),
    *ASM_OUT.params(),
])
def assemble(geo, source, piece_attrib, groups, output_attrib, renumber, unshare_points, pivot, name_prefix,
             piece_groups, remove_source_groups, **og):
    """Turn pieces into a clean set of separate bodies (Houdini's Assemble): number them, name them, give each its own
    points and a pivot, ready for ``rbd_configure``. MiniHou has no packed primitives or string attributes, so instead
    of packing, each piece's faces carry the int ``piece`` and its vector ``pivot``, and the names are the detail
    attribute ``piece_names``."""
    from .. import groups as G
    B.require_polygons(geo, "assemble")
    if geo.nprims == 0:
        return geo
    if source == "attribute":
        ids = piece_ids(geo, piece_attrib)
        used_groups = []
    elif source == "connectivity":
        ids, used_groups = piece_ids(geo, ""), []
    else:
        import fnmatch
        names = []
        for pat in (groups or "*").split():                        # in the order they are written
            names += [n for n in G.names(geo, "prim") if fnmatch.fnmatchcase(n, pat) and n not in names]
        if not names:
            raise ValueError(f"no primitive group matches {groups!r}; have {G.names(geo, 'prim')}")
        ids = np.full(geo.nprims, -1, np.int64)
        for k, name in enumerate(names):
            m = G.member(geo, "prim", name).astype(bool)
            ids[m & (ids < 0)] = k                          # a face in several groups goes to the first
        if (ids < 0).any():
            ids[ids < 0] = len(names)
        used_groups = names
    if renumber:
        uniq, first_at, inv = np.unique(ids, return_index=True, return_inverse=True)
        rank = np.empty(len(uniq), np.int64)
        rank[np.argsort(first_at, kind="stable")] = np.arange(len(uniq))      # numbered in order of first appearance
        ids = rank[inv.reshape(-1)]
    ids32 = ids.astype(np.int32)
    count = int(ids.max()) + 1
    t = geo.topology
    out = geo
    shared_mask = copied_mask = None
    if unshare_points:
        vp = t.vertex_point.astype(np.int64)
        pv = topo.prim_of_vertex(geo)
        slot = ids[pv]
        npieces = int(ids.max()) + 1
        key = vp * (npieces + 1) + slot
        loose = np.setdiff1d(np.arange(geo.npoints), vp)
        keys = np.concatenate([key, loose * (npieces + 1) + npieces])
        uniq, inv = np.unique(keys, return_inverse=True)
        inv = inv.reshape(-1)
        src = uniq // (npieces + 1)
        copies = np.bincount(src, minlength=geo.npoints)
        first = np.r_[True, src[1:] != src[:-1]]
        point = B.gather(geo.point, src)
        out = B.make(point["P"], inv[:len(vp)], t.prim_offsets, point, dict(geo.vertex.items()), dict(geo.prim.items()),
                     geo.detail, t.closed)
        shared_mask = copies[src] > 1
        copied_mask = ~first
    out = out.copy()
    if output_attrib:
        out.prim[output_attrib] = ids32
    P = out.P.astype(np.float64)
    vp2 = out.topology.vertex_point.astype(np.int64)
    pv2 = topo.prim_of_vertex(out)
    cnt = np.maximum(np.bincount(ids[pv2], minlength=count), 1)
    if pivot == "origin":
        pv_arr = np.zeros((count, 3))
    else:
        pv_arr = np.stack([np.bincount(ids[pv2], weights=P[vp2, k], minlength=count) for k in range(3)], 1) / cnt[:, None]
        if pivot == "bounds_bottom":
            low = np.full(count, np.inf)
            np.minimum.at(low, ids[pv2], P[vp2, 2])
            pv_arr[:, 2] = np.where(np.isfinite(low), low, 0.0)
    out.prim["pivot"] = pv_arr[ids].astype(np.float32)
    out.detail = dict(out.detail)
    out.detail["piece_names"] = [f"{name_prefix}{k}" for k in range(count)]
    if piece_groups:
        if count > 1000:
            raise ValueError("piece_groups is limited to 1000 pieces")
        for k in range(count):
            out = G.write(out, "prim", f"{name_prefix}{k}", ids == k)
    if remove_source_groups:
        out = out.copy()
        for name in used_groups:
            if G.attr_name(name) in out.prim:
                del out.prim[G.attr_name(name)]
    if shared_mask is not None:
        out = ASM_OUT.write(out, og, "unshared", copied_mask)
        out = ASM_OUT.write(out, og, "shared", shared_mask)
    return out
