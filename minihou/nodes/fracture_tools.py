"""The rest of Houdini's RBD fracture toolbox: Voronoi Fracture Points, Voronoi Adjacency and the contact search that
Glue Cluster builds on. (Voronoi Fracture and Voronoi Split are in :mod:`fracture`.)"""
import numpy as np

from .. import groups, spatial, topo
from ..geometry import Geometry
from ..outgroups import OutGroup, OutGroups
from ..sdk import Bool, Enum, Float, Input, Int, Str, node
from . import _build as B
from ._util import fan_triangulate, fbm3
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
    are at most ``max_gap`` apart along the normal and line up in the plane of the face: with no gap their centroids
    and areas coincide; across a gap (where the two caps are cut differently) the faces' discs have to overlap."""
    c, n, area = prim_geometry(geo)
    size = float(np.ptp(geo.P, axis=0).max()) if geo.npoints else 1.0
    tol = 1e-5 * max(size, 1e-9)
    if max_gap <= 0:                                  # coincident faces: their centroids coincide
        i, j, _ = spatial.pairs_within(c, tol * 5)
        slack = tol * 4
    else:                                             # across a gap the two caps are cut differently: faces touch when their discs overlap
        rad = np.sqrt(area / np.pi)
        i, j, _ = spatial.pairs_within(c, max_gap + 2 * float(rad.max(initial=0)) + tol)
    if not len(i):
        return np.zeros(0, np.int64), np.zeros(0, np.int64), np.zeros(0)
    d = c[j] - c[i]
    along = np.einsum("ij,ij->i", d, n[i])
    side = np.linalg.norm(d - along[:, None] * n[i], axis=1)
    ok = (pieces[i] != pieces[j]) & (np.einsum("ij,ij->i", n[i], n[j]) < -0.999)
    if max_gap <= 0:
        ok &= (np.abs(along) <= tol) & (side <= slack)
        ratio = np.minimum(area[i], area[j]) / np.maximum(np.maximum(area[i], area[j]), 1e-300)
        ok &= ratio >= 1 - 1e-3
    else:
        ok &= (np.abs(along) <= max_gap + tol) & (side <= rad[i] + rad[j])
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
    OutGroup("impact", "Impact Points", "impactpoints", "point", doc="impacts mode: points placed round an impact"),
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
    Int("count", 20, min=1, soft_max=500, doc="Number of points to scatter (the extra input's points come on top)", proxy="count"),
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
    Enum("source", ["scatter", "impacts"], doc="impacts: the second input holds impact points (optional float 'impulse' = how hard); "
                                               "the fracture points gather round them"),
    Float("impact_radius", 0.4, min=1e-6, soft_max=2.0, doc="impacts: spread of the points round each impact"),
    Float("impact_share", 0.7, min=0.0, max=1.0, doc="impacts: share of the points placed at impacts (the rest are scattered everywhere)"),
    Int("merge_clusters", 0, min=0, max=1000, doc="Group the points into this many clusters (by distance, a k-means style pass) in the int "
                                                  "point attribute 'cluster'; voronoi_fracture cluster_pieces merges their cells (0: -1 everywhere)"),
    *POINTS_OUT.params(),
])
def voronoi_fracture_points(geo, points, count, seed, surface_fraction, min_distance, relax, cluster_amount, clusters,
                            cluster_radius, piece_attrib, source="scatter", impact_radius=0.4, impact_share=0.7,
                            merge_clusters=0, **og):
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
    extra = points.P.astype(np.float64) if points is not None and points.npoints and source == "scatter" else np.zeros((0, 3))
    impacts = np.zeros((0, 3))
    impact_id = np.zeros(0, np.int32)
    if source == "impacts":
        if points is None or not points.npoints:
            raise ValueError("source = impacts needs the impact points on the second input")
        centres = points.P.astype(np.float64)
        weight = points.point["impulse"].astype(np.float64) if "impulse" in points.point else np.ones(len(centres))
        weight = np.maximum(weight, 0) + 1e-12
        n_imp = int(round(len(inner) * impact_share))
        share = np.floor(weight / weight.sum() * n_imp).astype(int)
        share[np.argsort(-weight)[:n_imp - int(share.sum())]] += 1          # hand out the remainder to the hardest hits
        got = [sample(int(c), centres[k], impact_radius * scale) if c else np.zeros((0, 3)) for k, c in enumerate(share)]
        impacts = np.concatenate(got) if got else np.zeros((0, 3))
        impact_id = np.concatenate([np.full(len(g), k, np.int32) for k, g in enumerate(got)]) if got else np.zeros(0, np.int32)
        inner = inner[:len(inner) - n_imp] if n_imp <= len(inner) else inner
    kinds = [("interior", inner), ("surface", surface), ("clustered", clustered), ("extra", extra), ("impact", impacts)]
    P = np.concatenate([k[1] for k in kinds]) if sum(len(k[1]) for k in kinds) else np.zeros((0, 3))
    label = np.concatenate([np.full(len(k[1]), n, np.int8) for n, k in enumerate(kinds)]) if len(P) else np.zeros(0, np.int8)
    which = np.concatenate([np.full(len(impacts), -1, np.int32)] * 0 + [impact_id]) if len(impacts) else np.zeros(0, np.int32)
    keep = _thin(P, min_distance)
    keep[label == 3] = True                                  # the user's own points always stay
    P, label = P[keep], label[keep]
    out = Geometry.from_arrays(P.astype(np.float32))
    if piece_attrib:
        out.point[piece_attrib] = np.arange(len(P), dtype=np.int32)
    if source == "impacts":
        full = np.full(len(keep), -1, np.int32)
        full[np.flatnonzero(np.concatenate([np.full(len(k[1]), n, np.int8) for n, k in enumerate(kinds)]) == 4)] = which if len(which) else 0
        out.point["impact"] = full[keep]
    cluster = np.full(len(P), -1, np.int32)
    if merge_clusters and len(P):
        k = min(int(merge_clusters), len(P))
        seeds = P[rng.choice(len(P), k, replace=False)].astype(np.float64)
        for _ in range(5):                                   # a few k-means passes
            owner, _d = spatial.nearest(seeds, P.astype(np.float64))
            for c in range(k):
                if (owner == c).any():
                    seeds[c] = P[owner == c].mean(0)
        cluster = spatial.nearest(seeds, P.astype(np.float64))[0].astype(np.int32)
    out.point["cluster"] = cluster
    for n, key in enumerate(("interior", "surface", "clustered", "extra", "impact")):
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


def sound_faces(geo):
    """True when every face is a simple, flat polygon with area: no repeated points, no sliver, no bow tie."""
    t = geo.topology
    _, nrm, area = prim_geometry(geo)
    size = float(np.ptp(geo.P, axis=0).max())
    if (area <= (1e-6 * size) ** 2).any():
        return False
    pv = topo.prim_of_vertex(geo)
    vp = t.vertex_point.astype(np.int64)
    pair = pv * (int(vp.max()) + 1) + vp
    if len(np.unique(pair)) != len(pair):
        return False
    P = geo.P.astype(np.float64)
    off = np.abs(np.einsum("ij,ij->i", P[vp] - P[vp[t.prim_offsets[:-1]][pv]], nrm[pv]))
    return not (off > 1e-4 * size).any()


def _half(piece, origin, normal, side, tol, clean=False):
    """One side of a plane cut. A face lying in the plane belongs to the half it faces away from (its outward normal
    points out of that half), so it is dropped from the other one."""
    from .group import keep_prims
    from .model import clip
    from .fracture import split_repeated_faces, weld_points
    h = clip(piece, origin, normal, side, outgrp_above=False)
    if h.nprims == 0:
        return h
    if clean:                                                       # a second try for awkward cuts: coincident points are one point
        h = B.drop_degenerate(split_repeated_faces(B.drop_degenerate(weld_points(h, tol * 4))))
    d = (h.P.astype(np.float64) - origin) @ normal
    flat = np.zeros(h.nprims, bool)
    off = np.abs(d)[h.topology.vertex_point] > tol
    flat = np.bincount(topo.prim_of_vertex(h), weights=off, minlength=h.nprims) == 0
    _, nrm, _ = prim_geometry(h)
    wrong = flat & ((nrm @ normal) * (1 if side == "above" else -1) > 0)       # 'above' keeps flat faces that face down
    return keep_prims(h, ~wrong) if wrong.any() else h


def _try_split(piece, plane, min_volume, eps):
    for clean in (False, True):
        out = _split_once(piece, plane, min_volume, eps, clean)
        if out is not None:
            return out
    return None


def _split_once(piece, plane, min_volume, eps, clean):
    from .fracture import INSIDE_TMP, _cap_plane
    origin, normal = plane
    halves = []
    for side in ("above", "below"):
        h = _half(piece, origin, normal, side, eps, clean)
        if h.nprims == 0:
            return None
        halves.append(_cap_plane(h, origin, normal, eps, INSIDE_TMP))
    out = []
    for h in halves:
        for comp in _split_connected(h):
            if mesh_volume(comp) <= 0 or np.any(topo.edges(comp).count != 2) or not sound_faces(comp):   # the cut did not close properly
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
    prior = np.zeros(geo.nprims, bool)
    name = og.get("outgrp_inside_name", "inside")
    if name and groups.attr_name(name) in geo.prim:                 # faces that are already inside (an earlier fracture) stay inside
        prior = groups.member(geo, "prim", name).astype(bool)
    start = groups.write(geo, "prim", INSIDE_TMP, prior)
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


# ------------------------------------------------------------------------------------------------------ glue cluster

GLUE_PIECE_OUT = OutGroups(
    OutGroup("glued", "Glued Pieces", "glued", "prim", doc="pieces output: faces of pieces that are glued to at least one other"),
    OutGroup("loose", "Loose Pieces", "loose", "prim", doc="pieces output: faces of pieces that touch nothing"),
    OutGroup("interface", "Interface Faces", "interface", "prim", doc="pieces output: the faces that lie against another piece"),
)
GLUE_LINK_OUT = OutGroups(
    OutGroup("internal", "Internal Links", "internal", "prim", doc="constraints output: links inside one cluster (rigid, never break)"),
    OutGroup("external", "External Links", "external", "prim", doc="constraints output: links between clusters (these can break)"),
    OutGroup("weak", "Weak Links", "weaklinks", "prim", doc="constraints output: links weaker than the 'weak' share of the average strength"),
)


def cluster_pieces(ids, a, b, area, target):
    """Cluster number per piece (``ids`` are the sorted piece numbers): pieces joined by the largest contacts merge first
    until ``target`` clusters are left (0 or at least the piece count: every piece is its own)."""
    n = len(ids)
    parent = np.arange(n)

    def find(x):
        while parent[x] != x:
            parent[x] = parent[parent[x]]
            x = parent[x]
        return x
    clusters = n
    size = np.ones(n, np.int64)
    if 0 < target < n:
        cap = int(np.ceil(1.5 * n / target))                     # first keep the clusters about even in size, then merge on
        for limit in (cap, n):
            for k in np.argsort(-area, kind="stable"):
                if clusters <= target:
                    break
                ra, rb = find(int(np.searchsorted(ids, a[k]))), find(int(np.searchsorted(ids, b[k])))
                if ra != rb and size[ra] + size[rb] <= limit:
                    lo, hi = min(ra, rb), max(ra, rb)
                    parent[hi] = lo
                    size[lo] += size[hi]
                    clusters -= 1
    root = np.array([find(i) for i in range(n)])
    _, label = np.unique(root, return_inverse=True)              # roots are the lowest member: numbered by lowest piece
    return label.reshape(-1).astype(np.int64)


def glue_links(geo, pieces, ids, max_gap, min_area, strength, variation, seed, min_share=0.0):
    """Contacts between pieces as glue: (a, b, area, strength in newtons)."""
    a, b, area, _faces = piece_pairs(geo, pieces, max_gap)
    keep = area >= max(min_area, min_share * float(area.max(initial=0.0)), 1e-300)
    a, b, area = a[keep], b[keep], area[keep]
    noise = np.random.default_rng([int(seed) & 0xFFFFFFFF, 41]).random(len(area)) * 2 - 1
    force = strength * area * np.maximum(1 + variation * noise, 0.01)
    return a, b, area, force


@node("glue_cluster", inputs=["geo"], category="model", params=[
    Str("piece_attrib", "piece", doc="Primitive int attribute with each face's piece (empty: connected pieces)"),
    Enum("output", ["pieces", "constraints"], doc="pieces: the input with a 'cluster' attribute; constraints: the glue links, for the "
                                                  "solver's third input"),
    Float("strength", 4.0e6, min=0.0, soft_max=1e8, doc="Newtons per square metre of contact: a link breaks when the force on "
                                                          "it passes strength x contact area (0: never breaks)"),
    Float("strength_variation", 0.2, min=0.0, max=1.0, doc="Random spread of each link's strength (0.2: within 20%)"),
    Int("seed", 0),
    Float("max_gap", 0.0, min=0.0, soft_max=0.1, doc="Pieces whose faces are up to this far apart still count as touching"),
    Float("min_area", 0.0, min=0.0, doc="No glue where the contact is smaller than this"),
    Float("min_share", 0.0, min=0.0, max=1.0, doc="No glue where the contact is smaller than this share of the largest one: "
                                                  "slivers would otherwise snap at once"),
    Int("clusters", 0, min=0, soft_max=100, doc="Merge the most strongly stuck pieces into this many rigid clusters (0: no clustering)"),
    Float("weak", 0.5, min=0.0, max=1.0, doc="A link is weak when it is below this share of the average strength"),
    *GLUE_PIECE_OUT.params(),
    *GLUE_LINK_OUT.params(),
])
def glue_cluster(geo, piece_attrib, output, strength, strength_variation, seed, max_gap, min_area, min_share, clusters, weak, **og):
    """Glue touching pieces together (Houdini's Glue Cluster). Pieces that share a cut face are linked; ``clusters`` can
    merge the most firmly stuck ones into rigid groups (the int ``cluster``: configure it with ``rbd_configure`` ``compound``
    on). ``output = constraints`` gives the remaining links as lines between piece centres with the attributes the solver
    reads: ``rbd_a`` / ``rbd_b`` (body ids: clusters when clustering, else pieces), ``strength`` (newtons) and ``length``."""
    B.require_polygons(geo, "glue_cluster")
    pieces = piece_ids(geo, piece_attrib)
    ids = np.unique(pieces)
    a, b, area, force = glue_links(geo, pieces, ids, max_gap, min_area, strength, strength_variation, seed, min_share)
    label = cluster_pieces(ids, a, b, area, clusters)
    slot = {int(p): k for k, p in enumerate(ids)}
    body = label if clusters > 0 else ids
    if output == "pieces":
        out = geo.copy()
        out.prim["cluster"] = body[np.searchsorted(ids, pieces)].astype(np.int32)
        linked = np.zeros(len(ids), bool)
        if len(a):
            linked[[slot[int(x)] for x in a]] = True
            linked[[slot[int(x)] for x in b]] = True
        c, _n, _area = prim_geometry(geo)
        i, j, _ = contacts(geo, pieces, max_gap)
        face = np.zeros(geo.nprims, bool)
        keep = np.isin(pieces[i], a) | np.isin(pieces[i], b)
        face[i[keep]] = True
        face[j[keep]] = True
        per_prim = linked[np.searchsorted(ids, pieces)]
        out = GLUE_PIECE_OUT.write(out, og, "glued", per_prim)
        out = GLUE_PIECE_OUT.write(out, og, "loose", ~per_prim)
        return GLUE_PIECE_OUT.write(out, og, "interface", face)
    centres = piece_centres(geo, pieces, ids)
    ba = np.array([body[slot[int(x)]] for x in a], np.int64)
    bb = np.array([body[slot[int(x)]] for x in b], np.int64)
    attrs = {"piece_a": a.astype(np.int32), "piece_b": b.astype(np.int32), "rbd_a": ba.astype(np.int32), "rbd_b": bb.astype(np.int32),
             "area": area.astype(np.float32), "strength": (force if strength > 0 else np.zeros_like(force)).astype(np.float32),
             "length": np.sqrt(area).astype(np.float32)}
    out, _ends = link_geometry(centres, ids, a, b, attrs)
    out.point["cluster"] = label[np.arange(len(ids))].astype(np.int32)
    internal = (ba == bb) if clusters > 0 else np.zeros(len(a), bool)
    out = GLUE_LINK_OUT.write(out, og, "internal", internal)
    out = GLUE_LINK_OUT.write(out, og, "external", ~internal)
    mean = float(force[~internal].mean()) if (~internal).any() else 0.0
    return GLUE_LINK_OUT.write(out, og, "weak", lambda: force < weak * mean)


# --------------------------------------------------------------------------------------------- rbd material fracture

MAX_MATERIAL_PIECES = 2000          # pieces rbd_material_fracture will make in all (levels multiply: 26 x 64 x 64 took over a minute)

# density kg/m3, friction, restitution, glue strength N/m2 (tuned so a wall stands under its own weight and breaks under a
# hard hit; the order is realistic, the numbers are not measured)
MATERIALS = {
    "concrete": (2400.0, 0.70, 0.05, 1.2e7),
    "brick": (1900.0, 0.60, 0.05, 6.0e6),
    "stone": (2700.0, 0.70, 0.05, 2.0e7),
    "wood": (700.0, 0.50, 0.20, 3.0e7),
    "glass": (2500.0, 0.40, 0.10, 6.0e6),
    "ceramic": (2300.0, 0.45, 0.15, 8.0e6),
    "ice": (920.0, 0.05, 0.05, 4.0e6),
    "metal": (7800.0, 0.35, 0.20, 8.0e8),
}
MAT_OUT = OutGroups(
    OutGroup("inside", "Inside Faces", "inside", "prim", default=True, doc="The new faces made by the fracture"),
    OutGroup("outside", "Outside Faces", "outside", "prim", doc="The faces that came from the input's own surface"),
    OutGroup("cutpoints", "Cut Points", "cutpoints", "point", doc="Points on the cuts"),
    OutGroup("cutedges", "Cut Edges", "cutedges", "edge", doc="The outline of every cut face (with interior_detail: where the inside meets "
                                                              "the outside)"),
)


def tessellate(geo, mask, n):
    """Cut the faces in ``mask`` into triangles and each triangle into n x n smaller ones, sharing the new points along shared
    edges. Returns ``(geometry, bool per primitive: a new triangle, bool per point: lies on the border of the tessellated
    area or is used by a face that was not tessellated)``. Point and corner attributes are interpolated."""
    from .boolean import _blend
    t = geo.topology
    tv, tp = fan_triangulate(t)
    sel = np.asarray(mask, bool)[tp]
    tv, tp = tv[sel], tp[sel]
    T = len(tv)
    if T == 0 or n < 1:
        return geo, np.zeros(geo.nprims, bool), np.ones(geo.npoints, bool)
    vp = t.vertex_point.astype(np.int64)
    cp = vp[tv]                                                     # corner points of each triangle (T, 3)
    lat = [(i, j) for i in range(n + 1) for j in range(n + 1 - i)]
    at = {ij: m for m, ij in enumerate(lat)}
    W = np.array([[(n - i - j) / n, i / n, j / n] for i, j in lat])
    K = np.array([[n - i - j, i, j] for i, j in lat])
    keys = np.zeros((T, len(lat), 4), np.int64)
    for m in range(len(lat)):
        zero = [c for c in range(3) if K[m, c] == 0]
        if len(zero) == 2:                                          # a corner of the triangle: its own point
            keys[:, m, 1] = cp[:, [c for c in range(3) if c not in zero][0]]
        elif len(zero) == 1:                                        # on an edge: shared with the neighbour by (low point, high point, steps)
            cu, cv = [c for c in range(3) if c not in zero]
            pu, pv = cp[:, cu], cp[:, cv]
            keys[:, m] = np.stack([np.ones(T, np.int64), np.minimum(pu, pv), np.maximum(pu, pv),
                                   np.where(pu > pv, K[m, cu], K[m, cv])], 1)
        else:                                                       # inside the triangle
            keys[:, m] = np.stack([np.full(T, 2), np.arange(T), np.full(T, m), np.zeros(T, np.int64)], 1)
    flat = keys.reshape(-1, 4)
    corner = (np.abs(W).max(axis=1) == 1.0)[None, :].repeat(T, 0).reshape(-1)       # lattice points that are corners
    uniq, first, inv = np.unique(flat[~corner], axis=0, return_index=True, return_inverse=True)
    inv = inv.reshape(-1)
    new_index = np.flatnonzero(~corner)[first]                      # (tri * lattice) slot that first made each new point
    npts = geo.npoints
    pid = np.empty(len(flat), np.int64)
    pid[corner] = flat[corner, 1]
    pid[~corner] = npts + inv
    pid = pid.reshape(T, len(lat))
    tri_of, lat_of = new_index // len(lat), new_index % len(lat)
    newP = np.einsum("mc,mck->mk", W[lat_of], geo.P.astype(np.float64)[cp[tri_of]])
    newpts = _blend(geo.point, cp[tri_of], W[lat_of])
    newpts["P"] = newP.astype(np.float32)
    point = B.concat([dict(geo.point.items()), newpts])
    tpl = []
    for i in range(n):
        for j in range(n - i):
            tpl.append((at[(i, j)], at[(i + 1, j)], at[(i, j + 1)]))
            if i + j < n - 1:
                tpl.append((at[(i + 1, j)], at[(i + 1, j + 1)], at[(i, j + 1)]))
    tpl = np.array(tpl)
    ntp = len(tpl)
    tri_pts = pid[:, tpl].reshape(T * ntp, 3)
    rows = np.repeat(tv, ntp, axis=0)                               # the parent's three corner rows, per small triangle
    wts = np.tile(W[tpl.reshape(-1)], (T, 1))
    vtx = _blend(geo.vertex, np.repeat(rows, 3, axis=0), wts)
    pv = topo.prim_of_vertex(geo)
    keepv = ~np.asarray(mask, bool)[pv]
    keepp = ~np.asarray(mask, bool)
    sizes = t.prim_sizes()
    # the border of the tessellated area: edges used by only one of its triangles
    side = np.concatenate([np.sort(cp[:, [0, 1]], axis=1), np.sort(cp[:, [1, 2]], axis=1), np.sort(cp[:, [2, 0]], axis=1)])
    big = max(npts, 1)
    skey = side[:, 0] * big + side[:, 1]
    ukey, cnt = np.unique(skey, return_counts=True)
    border = ukey[cnt == 1]
    # the new points on border edges are also inserted into the untouched faces beside them: no T-junctions
    flat_pid = pid.reshape(-1)
    edge_new = (flat[:, 0] == 1) & ~corner
    on_edge = {}
    if edge_new.any():
        ek = flat[edge_new]
        is_b = np.isin(ek[:, 1] * big + ek[:, 2], border)
        for (_, lo, hi, kk), q in zip(ek[is_b].tolist(), flat_pid[edge_new][is_b].tolist()):
            on_edge.setdefault((lo, hi), {})[kk] = q
    kept = np.flatnonzero(keepv)
    nxt = topo.next_vertex(geo)[kept]
    a_pt, b_pt = vp[kept], vp[nxt]
    ids = np.arange(len(kept))
    ra, rb, fr, pt, key = [kept], [kept], [np.zeros(len(kept))], [a_pt], [ids * n]
    if on_edge:
        ekey = np.minimum(a_pt, b_pt) * big + np.maximum(a_pt, b_pt)
        have = {lo * big + hi: (lo, hi) for lo, hi in on_edge}
        for c in np.flatnonzero(np.isin(ekey, list(have))).tolist():
            lo, hi = have[int(ekey[c])]
            steps = sorted(on_edge[(lo, hi)].items())
            forward = a_pt[c] == lo
            if not forward:
                steps = steps[::-1]
            for j, (kk, q) in enumerate(steps):
                ra.append(np.array([kept[c]]))
                rb.append(np.array([nxt[c]]))
                fr.append(np.array([kk / n if forward else 1 - kk / n]))
                pt.append(np.array([q]))
                key.append(np.array([c * n + j + 1]))
    ra, rb, fr, pt, key = (np.concatenate(x) for x in (ra, rb, fr, pt, key))
    order = np.argsort(key, kind="stable")
    ra, rb, fr, pt = ra[order], rb[order], fr[order], pt[order]
    kept_prim = pv[ra]
    kept_sizes = np.bincount(kept_prim, minlength=t.nprims)[keepp]
    kvert = B.lerp(geo.vertex, ra, rb, fr)
    vertex_point = np.concatenate([pt, tri_pts.reshape(-1)])
    offs = B.offsets_from_sizes(np.concatenate([kept_sizes, np.full(T * ntp, 3)]))
    vertex = B.concat([kvert, vtx])
    prim = B.concat([B.gather(geo.prim, np.flatnonzero(keepp)), B.gather(geo.prim, np.repeat(tp, ntp))])
    out = B.make(point["P"], vertex_point, offs, point, vertex, prim, geo.detail)
    tess = np.r_[np.zeros(int(keepp.sum()), bool), np.ones(T * ntp, bool)]
    # points that stay put: the border of the tessellated area, and anything an untouched face uses
    fixed = np.zeros(out.npoints, bool)
    fixed[vp[keepv]] = True
    fixed[side[np.isin(skey, border)].reshape(-1)] = True
    for pts_ in on_edge.values():
        fixed[list(pts_.values())] = True
    return out, tess, fixed


def interior_detail(geo, inside, divisions, amplitude, frequency, seed):
    """Roughen the faces in ``inside``: tessellate them and push the new interior points about with vector noise. The noise is
    a function of position, so two pieces whose cut faces coincide stay matched to each other; the border does not move."""
    out, tess, fixed = tessellate(geo, inside, divisions)
    if not tess.any():
        return out
    pos = out.P.astype(np.float64) * float(frequency)
    shift = np.stack([fbm3(pos.astype(np.float32), int(seed) + 1013 * c, 3, 2.0, 0.5) for c in range(3)], 1) * float(amplitude)
    free = ~fixed
    P = out.P.astype(np.float64)
    P[free] += shift[free]
    out = out.copy()
    out.point["P"] = P.astype(np.float32)
    return out


@node("rbd_material_fracture", inputs=["geo"], category="physics", params=[
    Enum("material", [*MATERIALS, "custom"], doc="Density, friction, bounce and how firmly the pieces are glued (custom: the four numbers below)"),
    Float("density", 1000.0, min=1.0, soft_max=10000.0, doc="custom: kg per cubic metre"),
    Float("friction", 0.6, min=0.0, max=10.0, doc="custom: friction"),
    Float("restitution", 0.1, min=0.0, max=1.0, doc="custom: bounce"),
    Float("glue_strength", 1.0e7, min=0.0, soft_max=1e9, doc="custom: glue strength in newtons per square metre of contact"),
    Int("pieces", 24, min=2, max=2000, soft_max=200, doc="How many pieces to break the shape into (at the first level)"),
    Int("fracture_level", 1, min=1, max=4, doc="Fracture each piece again this many times in all (each pass breaks the previous pieces)"),
    Int("sub_pieces", 4, min=2, max=64, doc="Pieces each piece breaks into at the second level and beyond"),
    Int("seed", 0),
    Float("surface_fraction", 0.15, min=0.0, max=1.0, doc="Share of the fracture points on the surface (finer pieces at the outside)"),
    Int("relax", 2, min=0, max=20, doc="Relaxation passes: more even pieces"),
    Float("gap", 0.0, min=0.0, soft_max=0.05, doc="A gap between pieces (needs max_gap in the glue)"),
    Bool("convex_parts", False, doc="Split pieces that are not convex (a concave input) into convex parts, so the body still simulates"),
    Float("max_concavity", 0.05, min=0.0, max=1.0, doc="convex_parts: how concave a piece may stay"),
    Bool("interior_detail", False, doc="Roughen the new inside faces with noise (render geometry: such pieces are not convex, so "
                                       "they cannot go to the solver)"),
    Int("detail_divisions", 4, min=1, max=16, doc="interior_detail: each inside triangle is cut into this many squared pieces"),
    Float("detail_amplitude", 0.02, min=0.0, soft_max=0.5, doc="interior_detail: how far the noise moves points"),
    Float("detail_frequency", 4.0, min=0.0, soft_max=20.0, doc="interior_detail: noise frequency"),
    Bool("glue", True, doc="Glue the pieces together (output = constraints gives the links)"),
    Int("clusters", 0, min=0, soft_max=50, doc="Merge strongly stuck pieces into this many rigid clusters (needs glue; 0: none)"),
    Float("strength_scale", 1.0, min=0.0, soft_max=10.0, doc="Multiplies the material's glue strength"),
    Float("mass_scale", 1.0, min=0.0001, soft_max=10.0, doc="Multiplies the pieces' mass (units are metres and kilograms)"),
    Enum("output", ["pieces", "constraints"], doc="pieces: the fractured, configured bodies for the solver's first input; "
                                                  "constraints: the glue links for its third"),
    *MAT_OUT.params(),
])
def rbd_material_fracture(geo, material, density, friction, restitution, glue_strength, pieces, fracture_level, sub_pieces,
                          seed, surface_fraction, relax, gap, convex_parts, max_concavity, interior_detail, detail_divisions,
                          detail_amplitude, detail_frequency, glue, clusters, strength_scale, mass_scale, output, **og):
    """A one-node fracture for simulation (Houdini's RBD Material Fracture): scatter fracture points, cut the closed
    mesh with ``voronoi_fracture`` (again for each further ``fracture_level``), give each piece its own points, glue the
    pieces and set mass (density x volume), friction and bounce from the material. Feed ``output = pieces`` to
    ``rbd_solver`` and ``output = constraints`` to its third input. Pieces are convex when the input is; for a concave
    input switch on ``convex_parts`` (each concave piece becomes a compound body of convex parts)."""
    from .fracture import voronoi_fracture
    from .group import keep_prims
    from .merge import merge
    if material != "custom":
        density, friction, restitution, glue_strength = MATERIALS[material]
    total = int(pieces) * int(sub_pieces) ** max(int(fracture_level) - 1, 0)
    if total > MAX_MATERIAL_PIECES:
        raise ValueError(f"{pieces} pieces broken into {sub_pieces} more for {fracture_level - 1} further level(s) would make about "
                         f"{total:,} pieces (limit {MAX_MATERIAL_PIECES:,}); lower the pieces, sub_pieces or fracture_level")
    scatter_off = {"outgrp_" + k: False for k in ("interior", "surface", "clustered", "extra", "impact")}
    fracture_og = {k: v for k, v in og.items() if k.startswith("outgrp_")}
    inside_name = og.get("outgrp_inside_name", "inside")
    pts = voronoi_fracture_points(geo, None, pieces, seed, surface_fraction, 0.0, relax, 0.0, 3, 0.25, "piece", **scatter_off)
    frac = voronoi_fracture(geo, pts, "piece", "inside", gap, **fracture_og)
    split = assemble(frac, "attribute", "piece", "*", "piece", True, True, "centroid", "piece", False, False)
    for level in range(2, int(fracture_level) + 1):                  # each further level breaks every piece again
        ids = split.prim["piece"].astype(np.int64)
        parts, offset = [], 0
        for k in range(int(ids.max()) + 1):
            part = keep_prims(split, ids == k)
            sub = voronoi_fracture_points(part, None, sub_pieces, int(seed) + 31 * level + 7 * k, surface_fraction, 0.0, 1, 0.0, 3,
                                          0.25, "piece", **scatter_off)
            sub_pieces_geo = voronoi_fracture(part, sub, "piece", "inside", gap, **fracture_og)
            numbered = assemble(sub_pieces_geo, "attribute", "piece", "*", "piece", True, False, "centroid", "piece", False, False)
            numbered.prim["piece"] = numbered.prim["piece"] + np.int32(offset)       # unique across all the parts
            offset += int(numbered.prim["piece"].max()) - offset + 1
            parts.append(numbered)
        split = assemble(merge_pieces(parts), "attribute", "piece", "*", "piece", True, True, "centroid", "piece", False, False)
    ids = split.prim["piece"].astype(np.int64)
    count = int(ids.max()) + 1 if len(ids) else 0
    link_gap = gap * 1.2
    if output == "constraints":
        if not glue:
            raise ValueError("output = constraints needs glue switched on")
        return glue_cluster(split, "piece", "constraints", glue_strength * strength_scale, 0.25, seed, link_gap, 0.0, 0.05,
                            clusters, 0.5)
    out = split
    if glue and clusters > 0:
        out = glue_cluster(split, "piece", "pieces", glue_strength * strength_scale, 0.25, seed, link_gap, 0.0, 0.05, clusters, 0.5)
    volume = np.array([mesh_volume(part_of(split, ids, k)) for k in range(count)])
    mass = np.maximum(volume * density * mass_scale, 1e-6)
    compound = False
    if convex_parts:                                                # concave pieces become several convex parts of one body
        pieces_out = []
        for k in range(count):
            part = keep_prims(out, ids == k)
            if concave_volume(part)[1] / max(mesh_volume(part), 1e-300) > max_concavity:
                dec = convex_decomposition(part, max_concavity, 16, 0.001, 8, "part", **{key: val for key, val in fracture_og.items()})
                dec = dec.copy()
                for name in ("piece", "cluster"):
                    if name in part.prim:
                        dec.prim[name] = np.full(dec.nprims, part.prim[name][0], np.int32)
                pieces_out.append(dec)
                compound = True
            else:
                pieces_out.append(part)
        out = merge_pieces(pieces_out)
        ids = out.prim["piece"].astype(np.int64)
    if interior_detail:
        name = inside_name or "inside"
        if groups.attr_name(name) not in out.prim:
            raise ValueError("interior_detail needs the inside faces group switched on")
        inside = groups.member(out, "prim", name).astype(bool)
        out = interior_detail_pass(out, inside, detail_divisions, detail_amplitude, detail_frequency, seed, og)
        ids = out.prim["piece"].astype(np.int64)
    out = out.copy()
    body = out.prim["cluster"].astype(np.int64) if "cluster" in out.prim else ids
    if "cluster" in out.prim:                                # a cluster is one body: its mass is the sum of its pieces'
        piece_body = np.zeros(count, np.int64)
        piece_body[ids] = body
        _, inv = np.unique(piece_body, return_inverse=True)
        inv = inv.reshape(-1)
        mass_prim = np.bincount(inv, weights=mass)[inv][ids]
    else:
        mass_prim = mass[ids]
    out.prim["rbd_id"] = body.astype(np.int32)
    if "cluster" in out.prim or compound:
        out.prim["rbd_compound"] = np.ones(out.nprims, np.int32)
    if compound:
        out.prim["rbd_hull"] = np.ones(out.nprims, np.int32)       # parts are nearly convex: the solver collides with their hulls
    out.prim["rbd_active"] = np.ones(out.nprims, np.int32)
    out.prim["rbd_mass"] = mass_prim.astype(np.float32)
    for name, value in (("friction", friction), ("restitution", restitution), ("linear_damping", 0.05), ("angular_damping", 0.1)):
        out.prim["rbd_" + name] = np.full(out.nprims, value, np.float32)
    out.prim["rbd_v"] = np.zeros((out.nprims, 3), np.float32)
    out.prim["rbd_w"] = np.zeros((out.nprims, 3), np.float32)
    out.detail = dict(out.detail)
    out.detail["rbd_material"] = material
    return out


def merge_pieces(parts):
    from .merge import merge
    return merge(parts) if len(parts) > 1 else parts[0]


def interior_detail_pass(out, inside, divisions, amplitude, frequency, seed, og):
    """:func:`interior_detail` plus the cut point / cut edge groups, rebuilt for the new faces."""
    from .fracture import cut_edge_mask
    out = interior_detail(out, inside, divisions, amplitude, frequency, seed)
    name = og.get("outgrp_inside_name", "inside")
    ins = groups.member(out, "prim", name).astype(bool)
    seam = cut_edge_mask(out, ins)
    out = MAT_OUT.write(out, og, "cutpoints", lambda: _points_of_edges(out, seam))
    return MAT_OUT.write(out, og, "cutedges", seam)


def part_of(geo, ids, k):
    from .group import keep_prims
    return keep_prims(geo, ids == k)


# ------------------------------------------------------------------------------------------- constraint properties

PROPS_OUT = OutGroups(OutGroup("changed", "Edited Links", "editedlinks", "prim", doc="The links this node set or scaled"))


@node("rbd_constraint_properties", inputs=["constraints"], category="physics", params=[
    Str("group", "", doc="Links to edit: group names, ranges or '@area > 0.1' (empty: all). 'internal' / 'external' from glue_cluster work here"),
    Enum("mode", ["set", "scale"], doc="set: replace the strength; scale: multiply what is there"),
    Float("strength", 1.0e6, min=0.0, soft_max=1e8, doc="The new strength (set) or the factor (scale)"),
    Bool("per_area", True, doc="set: newtons per square metre of the link's contact area (off: newtons for the whole link)"),
    Float("variation", 0.0, min=0.0, max=1.0, doc="Random spread of each link's result (0.2: within 20%)"),
    Int("seed", 0),
    Bool("unbreakable", False, doc="Strength 0 on these links: the solver never breaks them"),
    *PROPS_OUT.params(),
])
def rbd_constraint_properties(constraints, group, mode, strength, per_area, variation, seed, unbreakable, **og):
    """Edit the glue links of ``glue_cluster`` (Houdini's RBD Constraint Properties): set or scale the break strength of a
    group of links, with random variation, or make them unbreakable. Chain several to give different parts of a structure
    different strengths."""
    if constraints.nprims == 0:
        return constraints
    if "strength" not in constraints.prim:
        raise ValueError("these are not glue links: no 'strength' attribute (use glue_cluster output = constraints)")
    from .. import groups as G
    sel = G.mask(constraints, "prim", group)
    sel = np.ones(constraints.nprims, bool) if sel is None else sel
    cur = constraints.prim["strength"].astype(np.float64)
    noise = np.random.default_rng([int(seed) & 0xFFFFFFFF, 53]).random(constraints.nprims) * 2 - 1
    spread = np.maximum(1 + variation * noise, 0.01)
    if unbreakable:
        new = np.zeros(constraints.nprims)
    elif mode == "scale":
        new = cur * strength * spread
    else:
        base = strength * constraints.prim["area"].astype(np.float64) if per_area and "area" in constraints.prim else np.full(constraints.nprims, strength)
        new = base * spread
    out = constraints.copy()
    out.prim["strength"] = np.where(sel, new, cur).astype(np.float32)
    return PROPS_OUT.write(out, og, "changed", sel)
