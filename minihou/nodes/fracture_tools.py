"""The rest of Houdini's RBD fracture toolbox: Voronoi Fracture Points, Voronoi Adjacency and the contact search that
Glue Cluster builds on. (Voronoi Fracture and Voronoi Split are in :mod:`fracture`.)"""
import numpy as np

from .. import spatial, topo
from ..geometry import Geometry
from ..outgroups import OutGroup, OutGroups
from ..sdk import Float, Input, Int, Str, node
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
