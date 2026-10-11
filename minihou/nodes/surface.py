"""Convex hull and dissolve (merging flat faces into n-gons)."""
import numpy as np

from .. import hull as hl
from .. import topo
from ..geometry import Geometry
from ..sdk import Float, Str, node
from . import _build as B
from ._util import normalize_rows
from .model import _prim_sel, compact_points


@node("convex_hull", inputs=["geo"], category="modify")
def convex_hull(geo):
    """The smallest convex shape holding all the input points: a closed triangle mesh facing outwards, on those of
    the points that are corners of the hull (they keep their attributes). Primitives are ignored. With fewer than four
    points, or points that are all in one plane, there is no volume and the points come back unchanged."""
    P = geo.P.astype(np.float64)
    tri = hl.convex_hull(P)
    if not len(tri):
        return Geometry.points_only(geo.P) if geo.nprims == 0 else B.make(
            geo.P, np.zeros(0, np.int32), np.zeros(1, np.int32), dict(geo.point.items()), detail=geo.detail)
    hull = B.make(geo.P, tri.reshape(-1).astype(np.int32), np.arange(len(tri) + 1, dtype=np.int32) * 3,
                  dict(geo.point.items()), detail=geo.detail)
    return compact_points(hull)


# -- dissolve ---------------------------------------------------------------------------------------------------
def _flat_groups(geo, sel, cos_tol):
    """Representative polygon per polygon: neighbours across a shared edge join one flat region when their normals
    agree (within the angle), flattest pairs first, and every region stays within the angle of its first polygon's
    plane (so a gently curved surface merges into patches, not into one slab)."""
    from .bevel import twins
    from .normal import prim_normals
    nprims = geo.nprims
    pn = normalize_rows(prim_normals(geo).astype(np.float64))
    pv = topo.prim_of_vertex(geo)
    tw = twins(geo)
    he = np.flatnonzero(tw >= 0)
    a, b = pv[he], pv[tw[he]]
    keep = (a < b) & sel[a] & sel[b]
    a, b = a[keep], b[keep]
    dots = np.einsum("ij,ij->i", pn[a], pn[b])
    ok = dots >= cos_tol
    a, b, dots = a[ok], b[ok], dots[ok]
    order = np.argsort(-dots, kind="stable")
    parent = np.arange(nprims)
    ref = pn                                                 # plane normal of each region: its seed polygon's

    def find(x):
        while parent[x] != x:
            parent[x] = parent[parent[x]]
            x = parent[x]
        return x

    for k in order.tolist():
        ra, rb = find(int(a[k])), find(int(b[k]))
        if ra == rb:
            continue
        if float(ref[ra] @ ref[rb]) < cos_tol:
            continue
        lo, hi = (ra, rb) if ra < rb else (rb, ra)
        parent[hi] = lo
    return np.array([find(i) for i in range(nprims)], np.int64)


@node("dissolve", inputs=["geo"], category="modify", params=[
    Str("group", "", doc="Primitives to merge (pattern; empty: all)"),
    Float("angle", 1.0, min=0.0, max=89.0, soft_max=30.0, doc="Largest angle in degrees between neighbouring faces "
                                                              "that still count as one flat region"),
])
def dissolve(geo, group, angle):
    """Merge neighbouring faces that lie in one plane (within the angle) into single n-gons, e.g. the triangles that
    a boolean, a triangulate or a convex hull leave on flat sides. Points stay where they are; the merged face takes
    the attributes of the lowest-numbered face of its region. Regions whose outline would pinch or enclose a hole are
    left as they are."""
    from .boolean import _merge_pieces
    B.require_polygons(geo, "dissolve")
    if geo.nprims < 2:
        return geo
    sel = _prim_sel(geo, group)
    rep = _flat_groups(geo, sel, float(np.cos(np.radians(angle))))
    if np.array_equal(rep, np.arange(geo.nprims)):
        return geo
    t = geo.topology
    sizes = t.prim_sizes().astype(np.int64)
    vp = t.vertex_point.astype(np.int64)
    cidx, msz, mrep = _merge_pieces(vp, sizes, rep, geo.P.astype(np.float64))
    order = np.argsort(mrep, kind="stable")                  # keep the faces in the order of their representatives
    starts = np.cumsum(msz) - msz
    cidx = np.concatenate([cidx[starts[o]:starts[o] + msz[o]] for o in order]) if len(order) else cidx
    msz, mrep = msz[order], mrep[order]
    return B.make(geo.P, vp[cidx], B.offsets_from_sizes(msz), dict(geo.point.items()),
                  {n: arr[cidx] for n, arr in geo.vertex.items()}, B.gather(geo.prim, mrep), geo.detail)
