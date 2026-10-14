"""Topology-preserving miter offsets for simple planar face regions.

Distances are measured perpendicular to boundary edges. Offsets that require a
straight-skeleton topology event (collapse, splitting or collision) are rejected.
"""
import numpy as np
from .. import topo


def _cross(a, b):
    return a[0]*b[1]-a[1]*b[0]


def _validate_loops(loops, tolerance):
    segments = [(a, b) for loop in loops for a, b in zip(loop, np.roll(loop, -1, axis=0))]
    for i, (a, b) in enumerate(segments):
        if np.linalg.norm(b-a) <= tolerance:
            raise ValueError('Inset offset collapses a boundary edge; reduce the amount')
        for c, d in segments[i+1:]:
            if any(np.linalg.norm(x-y) <= tolerance for x in (a,b) for y in (c,d)):
                continue
            ab, cd = b-a, d-c
            denominator = _cross(ab, cd)
            if abs(denominator) <= tolerance*tolerance:
                if abs(_cross(c-a, ab)) <= tolerance*np.linalg.norm(ab):
                    lo, hi = sorted(((c-a)@ab/(ab@ab), (d-a)@ab/(ab@ab)))
                    if min(hi, 1)-max(lo, 0) > tolerance:
                        raise ValueError('Inset boundary overlaps itself; reduce the amount')
                continue
            t, u = _cross(c-a, cd)/denominator, _cross(c-a, ab)/denominator
            if -tolerance <= t <= 1+tolerance and -tolerance <= u <= 1+tolerance:
                raise ValueError('Inset boundary self-intersects; reduce the amount or split the region')


def offset_selected(geo, selected, amount, individual=False):
    """Return positions per source corner, leaving unselected corners untouched."""
    topology = geo.topology
    vp = topology.vertex_point
    offsets = topology.prim_offsets
    P = geo.P.astype(float)
    result = P[vp].copy()
    faces = list(map(int, np.flatnonzero(selected)))
    edges = topo.edges(geo)
    face_of = topo.prim_of_vertex(geo)
    next_vertex = topo.next_vertex(geo)
    adjacency = {f:set() for f in faces}
    if not individual:
        owners = {}
        for corner in np.flatnonzero(selected[face_of]):
            owners.setdefault(int(edges.of_vertex[corner]), []).append(int(face_of[corner]))
        for owners_at_edge in owners.values():
            if len(owners_at_edge)>2:
                raise ValueError('Inset requires a manifold selected region')
            if len(owners_at_edge)==2:
                a,b=owners_at_edge; adjacency[a].add(b); adjacency[b].add(a)
    pending=set(faces)
    while pending:
        seed=pending.pop(); region={seed}; todo=[seed]
        while todo:
            for neighbour in adjacency[todo.pop()]:
                if neighbour in pending:
                    pending.remove(neighbour); region.add(neighbour); todo.append(neighbour)
        corners=np.concatenate([np.arange(offsets[f], offsets[f+1]) for f in sorted(region)])
        points=P[vp[corners]]
        extent=max(float(np.ptp(points,axis=0).max()),1e-12)
        tolerance=max(extent*1e-7,1e-12)
        q=P[vp[offsets[seed]:offsets[seed+1]]]
        normal=np.cross(q-q[0],np.roll(q,-1,axis=0)-q[0]).sum(0)
        length=np.linalg.norm(normal)
        if length<=tolerance*tolerance:
            raise ValueError('Inset requires non-degenerate planar faces')
        normal/=length
        if np.max(np.abs((points-q[0])@normal))>tolerance:
            raise ValueError('Inset connected regions must be planar; use individual faces or split the selection')
        direction=q[1]-q[0]; direction/=max(np.linalg.norm(direction),1e-30)
        if np.linalg.norm(direction)<.5:
            raise ValueError('Inset boundary contains a zero-length edge')
        basis=np.stack([direction,np.cross(normal,direction)],axis=1)
        counts=np.bincount(edges.of_vertex[corners],minlength=len(edges.points))
        boundary=corners[counts[edges.of_vertex[corners]]==1]
        if not len(boundary):
            raise ValueError('Inset requires an open planar region boundary')
        successor={}
        for c in boundary:
            a,b=int(vp[c]),int(vp[next_vertex[c]])
            if a in successor:
                raise ValueError('Inset boundary branches at a vertex; split the region')
            successor[a]=b
        loops=[]; remaining=set(successor)
        while remaining:
            start=next(iter(remaining)); loop=[]; at=start
            while at in remaining:
                remaining.remove(at); loop.append(at); at=successor.get(at)
            if at!=start or len(loop)<3:
                raise ValueError('Inset requires closed, consistently wound boundary loops')
            loops.append(loop)
        old_loops=[(P[loop]-q[0])@basis for loop in loops]
        _validate_loops(old_loops,tolerance)
        new_loops=[]; moved={}
        for loop,xy in zip(loops,old_loops):
            vectors=np.roll(xy,-1,axis=0)-xy
            lengths=np.linalg.norm(vectors,axis=1)
            if np.any(lengths<=tolerance):
                raise ValueError('Inset boundary contains a zero-length edge')
            directions=vectors/lengths[:,None]
            inward=np.stack([-directions[:,1],directions[:,0]],axis=1)
            shifted=[]
            for i,point in enumerate(xy):
                a,b=inward[i-1],inward[i]
                cosine=float(a@b)
                if 1+cosine<1e-10:
                    raise ValueError('Inset boundary has an unstable reversing miter')
                displacement=amount*(a+b)/(1+cosine)
                if np.linalg.norm(displacement)>max(abs(amount)*1000,tolerance):
                    raise ValueError('Inset boundary miter is too sharp; bevel the corner first')
                shifted.append(point+displacement)
            shifted=np.asarray(shifted)
            # Boundary edges cannot reverse as an offset crosses its medial axis.
            if np.any(np.sum((np.roll(shifted,-1,axis=0)-shifted)*directions,axis=1)<=tolerance):
                raise ValueError('Inset offset collapses or reverses an edge; reduce the amount')
            new_loops.append(shifted)
            moved.update({point:q[0]+position@basis.T for point,position in zip(loop,shifted)})
        _validate_loops(new_loops,tolerance)
        for corner in corners:
            if int(vp[corner]) in moved:result[corner]=moved[int(vp[corner])]
        # Internal face edges remain part of the original topology: reject folds.
        for face in region:
            cc=np.arange(offsets[face],offsets[face+1])
            polygon=(result[cc]-q[0])@basis
            _validate_loops([polygon],tolerance)
            if sum(_cross(a,b) for a,b in zip(polygon,np.roll(polygon,-1,axis=0)))<=tolerance*tolerance:
                raise ValueError('Inset offset folds an interior face; reduce amount or dissolve interior edges')
    return result
