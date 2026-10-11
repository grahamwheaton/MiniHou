"""Subdivision: Catmull-Clark (Houdini's Subdivide, OpenSubdiv-style rules) and linear splitting, fully vectorised.

Each level splits every n-gon into n quads around a new face point. Positions use the Catmull-Clark rules (open
boundaries keep their curve: boundary points use the 1/8-3/4-1/8 B-spline rule, boundary edges their midpoint,
and corners -- boundary points of a single face -- stay put, like OpenSubdiv's 'edge and corner' mode);
'linear' keeps the shape and only refines. Point attributes other than P are interpolated linearly (floats) or by
minimum (ints, so groups stay crisp); vertex attributes such as uv split per face; primitive attributes are inherited.
"""
import numpy as np

from .. import edges, topo
from ..sdk import Bool, Enum, Int, node
from . import _build as B

MAX_FACES = 20_000_000


def subdivide_once(geo, smooth: bool, use_creases: bool = True):
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

    sharp = edges.values(geo, edges.CREASE)
    corner = np.asarray(geo.point.get('cornerweight', np.zeros(npt)), dtype=float)
    if corner.shape != (npt,) or not np.isfinite(corner).all() or np.any(corner < 0):
        raise ValueError('cornerweight must be a finite nonnegative scalar point attribute')
    if smooth and use_creases and (np.any(sharp) or np.any(corner)):
        strength = np.clip(sharp, 0, 1)
        edge_pt = edge_pt*(1-strength[:,None]) + mid*strength[:,None]
        # Semi-sharp crease rule: two sharp incident edges give a cubic boundary
        # point; three pin a corner. Fractional strengths blend those rules.
        incident = [[] for _ in range(npt)]
        for eid, (a,b) in enumerate(e.points):
            w = 1.0 if boundary_edge[eid] else strength[eid]
            if w > 0:
                incident[a].append((w,int(b)))
                incident[b].append((w,int(a)))
        for p, links in enumerate(incident):
            links.sort(reverse=True)
            if len(links) >= 2:
                w = links[1][0]
                crease_pos = .75*P[p]+.125*(P[links[0][1]]+P[links[1][1]])
                # Preserve the existing single-face boundary-corner convention.
                if not (on_boundary[p] and nfaces[p] == 1):
                    vert_pt[p] = (1-w)*vert_pt[p]+w*crease_pos
            w = max(min(corner[p],1), links[2][0] if len(links)>=3 else 0)
            vert_pt[p] = (1-w)*vert_pt[p]+w*P[p]

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
    out = B.make(newP, q, np.arange(0, 4 * nv + 1, 4), point, vtx, prim, geo.detail)
    edge_names = [name for name in geo.vertex if name.startswith(edges.PREFIX) or name == edges.CREASE]
    if 'cornerweight' in geo.point:
        out.point['cornerweight'] = np.r_[np.maximum(corner-1,0),np.zeros(ne+nf)].astype(np.float32)
    if not edge_names:
        return out
    # Only the two children of an original edge inherit edge attributes; face
    # spokes must not inherit generic corner interpolation.
    ne_out = topo.edges(out)
    children = {}
    for eid,(a,b) in enumerate(e.points):
        children[tuple(sorted((int(a), npt+eid)))] = eid
        children[tuple(sorted((int(b), npt+eid)))] = eid
    source = np.array([children.get(tuple(pair),-1) for pair in ne_out.points])
    valid = source >= 0
    for name in edge_names:
        old = edges.values(geo,name)
        if name == edges.CREASE:
            old = np.maximum(old-1,0)
        data = np.zeros(len(source),geo.vertex[name].dtype)
        data[valid] = old[source[valid]]
        out = edges.write(out,name,data)
    return out


@node("subdivide", inputs=["geo"], category="modify", version=2, params=[
    Int("depth", 1, min=0, max=6, doc="Levels; each multiplies the face count by about 4"),
    Enum("algorithm", ["catmull_clark", "linear"], doc="catmull_clark smooths; linear only splits faces"),
    Bool("use_creases", True, doc="Use outgoing-corner creaseweight and point cornerweight; sharpness decays each level"),
])
def subdivide(geo, depth, algorithm, use_creases=True):
    """Subdivide polygons into quads (Houdini's Subdivide). Catmull-Clark smooths towards a limit surface and keeps open
    boundaries as smooth curves; linear splits without moving anything. Faces with fewer than 3 points are dropped."""
    faces = geo.nvertices * 4 ** max(depth - 1, 0)
    if depth and faces > MAX_FACES:
        raise ValueError(f"depth {depth} would make about {faces:,} faces (limit {MAX_FACES:,}); lower the depth")
    B.require_polygons(geo, "subdivide")
    out = geo
    for _ in range(depth):
        out = subdivide_once(out, algorithm == "catmull_clark", use_creases)
    return out
