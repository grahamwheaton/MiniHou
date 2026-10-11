"""Subdivision: Catmull-Clark (Houdini's Subdivide, OpenSubdiv-style rules) and linear splitting, fully vectorised.

Each level splits every n-gon into n quads around a new face point. Positions use the Catmull-Clark rules (open
boundaries keep their curve: boundary points use the 1/8-3/4-1/8 B-spline rule, boundary edges their midpoint,
and corners -- boundary points of a single face -- stay put, like OpenSubdiv's 'edge and corner' mode);
'linear' keeps the shape and only refines. Point attributes other than P are interpolated linearly (floats) or by
minimum (ints, so groups stay crisp); vertex attributes such as uv split per face; primitive attributes are inherited.
"""
import numpy as np

from .. import topo
from ..sdk import Enum, Int, node
from . import _build as B

MAX_FACES = 20_000_000


def subdivide_once(geo, smooth: bool):
    t = geo.topology
    sizes = t.prim_sizes()
    if np.any(sizes < 3):
        from .model import compact_points
        from .group import keep_prims
        geo = compact_points(keep_prims(geo, sizes >= 3))
        t = geo.topology
    npt, nf, nv = geo.npoints, t.nprims, t.nvertices
    vp = t.vertex_point.astype(np.int64)
    pv = topo.prim_of_vertex(geo)
    nxt, prv = topo.next_vertex(geo), topo.prev_vertex(geo)
    e = topo.edges(geo)
    ne = len(e.points)
    P = geo.P.astype(np.float64)

    face_pt = topo.segment_sum(P[vp], pv, nf) / np.maximum(t.prim_sizes(), 1)[:, None]
    mid = (P[e.points[:, 0]] + P[e.points[:, 1]]) * 0.5
    boundary_edge = e.count == 1
    if smooth:
        fsum = topo.segment_sum(face_pt[pv], e.of_vertex, ne)
        edge_pt = np.where(boundary_edge[:, None], mid, (P[e.points[:, 0]] + P[e.points[:, 1]] + fsum) /
                           np.maximum(2 + e.count, 1)[:, None])
        # vertex points
        valence = np.bincount(e.points.ravel(), minlength=npt).astype(np.float64)
        nfaces = np.bincount(vp, minlength=npt).astype(np.float64)
        Q = topo.segment_sum(face_pt[pv], vp, npt) / np.maximum(nfaces, 1)[:, None]
        R = topo.segment_sum(np.repeat(mid, 2, axis=0), e.points.ravel(), npt) / np.maximum(valence, 1)[:, None]
        n = np.maximum(valence, 1)[:, None]
        interior = (Q + 2 * R + (n - 3) * P) / n
        bcount = np.bincount(e.points[boundary_edge].ravel(), minlength=npt)
        bnb = topo.segment_sum(P[e.points[boundary_edge][:, ::-1].ravel()], e.points[boundary_edge].ravel(), npt)
        on_boundary = bcount > 0
        boundary_rule = 0.75 * P + bnb / 8
        vert_pt = np.where(on_boundary[:, None], np.where((bcount == 2)[:, None], boundary_rule, P), interior)
        vert_pt = np.where(((nfaces == 0) | (on_boundary & (nfaces == 1)))[:, None], P, vert_pt)   # corners stay sharp
        edge_pt = np.where((e.count > 2)[:, None], mid, edge_pt)       # non-manifold edges: stay put
    else:
        edge_pt, vert_pt = mid, P

    # new points: [original points | edge points | face points]
    pts_point = dict(geo.point.items())
    e0, e1 = e.points[:, 0], e.points[:, 1]
    pts_edge = {name: ((a[e0] + a[e1]) * 0.5).astype(a.dtype) if a.dtype.kind == "f" else np.minimum(a[e0], a[e1])
                for name, a in geo.point.items()}
    pts_face = B.mean_by(geo.point, vp, pv, nf)
    point = B.concat([pts_point, pts_edge, pts_face])
    newP = np.concatenate([vert_pt, edge_pt, face_pt]).astype(np.float32)
    # one quad per original vertex: (v, edge(v->next), face, edge(prev->v))
    q = np.stack([vp, npt + e.of_vertex, npt + ne + pv, npt + e.of_vertex[prv]], 1).reshape(-1)
    # vertex attributes: corner, edge midpoints within the face, face average
    vtx = {}
    for name, a in geo.vertex.items():
        if a.dtype.kind == "f":
            f_avg = (topo.segment_sum(a, pv, nf) / np.maximum(t.prim_sizes(), 1).reshape((-1,) + (1,) * (a.ndim - 1)))
            nxt_mid = (a + a[nxt]) / 2
            prv_mid = (a + a[prv]) / 2
            vtx[name] = np.stack([a, nxt_mid, f_avg[pv].astype(a.dtype), prv_mid], 1).reshape((-1,) + a.shape[1:]).astype(a.dtype)
        else:
            vtx[name] = np.repeat(a, 4, axis=0)
    prim = {name: np.repeat(a, t.prim_sizes(), axis=0) for name, a in geo.prim.items()}
    return B.make(newP, q, np.arange(0, 4 * nv + 1, 4), point, vtx, prim, geo.detail)


@node("subdivide", inputs=["geo"], category="modify", params=[
    Int("depth", 1, min=0, max=6, doc="Levels; each multiplies the face count by about 4"),
    Enum("algorithm", ["catmull_clark", "linear"], doc="catmull_clark smooths; linear only splits faces"),
])
def subdivide(geo, depth, algorithm):
    """Subdivide polygons into quads (Houdini's Subdivide). Catmull-Clark smooths towards a limit surface and keeps open
    boundaries as smooth curves; linear splits without moving anything. Faces with fewer than 3 points are dropped."""
    faces = geo.nvertices * 4 ** max(depth - 1, 0)
    if depth and faces > MAX_FACES:
        raise ValueError(f"depth {depth} would make about {faces:,} faces (limit {MAX_FACES:,}); lower the depth")
    B.require_polygons(geo, "subdivide")
    out = geo
    for _ in range(depth):
        out = subdivide_once(out, algorithm == "catmull_clark")
    return out
