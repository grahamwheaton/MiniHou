"""View-projected interactive Knife with perspective-correct attribute interpolation."""
import json
import numpy as np
from ..sdk import node, Str, Bool, Float
from .. import edges, topo
from ..boolean import earclip
from . import _build as B

EPS=1e-8

def cross(a,b): return float(a[0]*b[1]-a[1]*b[0])

def bary(p,tri):
    a,b,c=tri;d=cross(b-a,c-a)
    if abs(d)<1e-14:return None
    v=cross(p-a,c-a)/d;w=cross(b-a,p-a)/d
    return np.array([1-v-w,v,w])

def clip(a,b,tri):
    u,v=bary(a,tri),bary(b,tri)
    if u is None:return None
    lo,hi=0.,1.
    for x,y in zip(u,v):
        d=y-x
        if abs(d)<EPS:
            if x < -EPS:return None
        elif d>0:lo=max(lo,-x/d)
        else:hi=min(hi,-x/d)
    if hi-lo<1e-8:return None
    return a+(b-a)*lo,a+(b-a)*hi

def cut_triangle(tri,a,b):
    """Split a convex triangle by a finite segment, including dangling interior ends."""
    hit=clip(a,b,np.array(tri))
    if hit is None:return [tri]
    a,b=hit
    if np.linalg.norm(a-b)<EPS:return [tri]
    boundary=list(tri)
    inside=[]
    for p in (a,b):
        if any(np.linalg.norm(p-q)<EPS for q in boundary):continue
        for i,q in enumerate(boundary):
            r=boundary[(i+1)%len(boundary)];d=r-q;t=np.dot(p-q,d)/(np.dot(d,d) or 1)
            if -EPS<=t<=1+EPS and np.linalg.norm(p-(q+t*d))<EPS:
                boundary.insert(i+1,p);break
        else:inside.append(p)
    if inside:
        p=inside[0];made=[[p,q,boundary[(i+1)%len(boundary)]] for i,q in enumerate(boundary)]
        if len(inside)>1:
            return [t for f in made for t in cut_triangle(f,a,b)]
        return [t for t in made if abs(cross(t[1]-t[0],t[2]-t[0]))>EPS]
    ia=min(range(len(boundary)),key=lambda i:np.linalg.norm(boundary[i]-a));ib=min(range(len(boundary)),key=lambda i:np.linalg.norm(boundary[i]-b))
    ia,ib=sorted((ia,ib))
    if ib-ia in (1,len(boundary)-1):polys=[boundary]
    else:polys=[boundary[ia:ib+1],boundary[ib:]+boundary[:ia+1]]
    result=[]
    for poly in polys:
        if len(poly)<3:continue
        xy=np.array(poly);sign=sum(cross(xy[i],xy[(i+1)%len(xy)]) for i in range(len(xy)));ids=list(range(len(poly)))
        if sum(cross(xy[i],xy[(i+1)%len(xy)]) for i in ids)<0:ids.reverse()
        for t in earclip(ids,xy):
            out=[xy[i] for i in t]
            if sign<0:out.reverse()
            if abs(cross(out[1]-out[0],out[2]-out[0])) > EPS*max(np.linalg.norm(out[1]-out[0]),np.linalg.norm(out[2]-out[0])):
                result.append(out)
    return result

@node('knife',inputs=['geo'],category='model',params=[Str('cuts','[]',doc='View-projected cut chains with point positions and snap references.'),Bool('cut_through',False,doc='Cut hidden faces along the view projection.'),Float('snap_tolerance',10.,doc='Viewport snapping radius in pixels.')])
def knife(geo,cuts,cut_through=False,snap_tolerance=10.):
    B.require_polygons(geo,'Knife')
    segments=json.loads(cuts)
    if not segments:return geo
    if not isinstance(segments,list) or len(segments)>512:raise ValueError('Knife supports up to 512 segments per operation.')
    def screen(point,matrix):
        snap=point.get('snap') or {};ids=snap.get('element',[]);weights=snap.get('params',[])
        if snap.get('point_count')!=geo.npoints or not ids or len(ids)!=len(weights) or any(not isinstance(i,int) or i<0 or i>=geo.npoints for i in ids):return point['screen']
        q=sum((geo.P[i].astype(float)*w for i,w in zip(ids,weights)),np.zeros(3))
        m=np.asarray(matrix,float).reshape(4,4,order='F');clip=m@np.r_[q,1.]
        return (clip[:2]/clip[3]).tolist() if abs(clip[3])>EPS else point['screen']
    segments=[{'a':screen(a,chain['matrix']),'b':screen(b,chain['matrix']),'matrix':chain['matrix'],'through':chain.get('through',False)} for chain in segments for a,b in zip(chain['points'],chain['points'][1:])] if segments and 'points' in segments[0] else segments
    if cut_through:segments=[dict(s,through=True) for s in segments]
    P=geo.P.astype(float);vp=geo.topology.vertex_point;off=geo.topology.prim_offsets
    # Triangulate each polygon in its own stable plane, preserving original winding.
    original=[]
    for f in range(geo.nprims):
        corners=list(range(int(off[f]),int(off[f+1])));q=P[vp[corners]]
        normal=np.sum(np.cross(q,np.roll(q,-1,axis=0)),axis=0);drop=int(np.argmax(abs(normal)));xy=np.delete(q,drop,axis=1)
        ids=list(range(len(q)));reverse=sum(cross(xy[i],xy[(i+1)%len(q)]) for i in ids)<0
        if reverse:ids.reverse()
        for t in earclip(ids,xy):
            if reverse:t=t[::-1]
            original.append((f,np.array([corners[i] for i in t])))
    # Rows retain barycentric coordinates in their original triangle throughout all cuts.
    rows=[[(np.eye(3)[i]) for i in range(3)] for _ in original]
    rows=[[r] for r in rows];touched=set()
    for seg in segments:
        matrix=np.asarray(seg['matrix'],float).reshape(4,4,order='F');a=np.asarray(seg['a'],float);b=np.asarray(seg['b'],float)
        if a.shape!=(2,) or b.shape!=(2,) or not np.isfinite(matrix).all() or not np.isfinite([a,b]).all():raise ValueError('Invalid Knife projection.')
        projected=np.c_[P,np.ones(len(P))]@matrix.T
        w=projected[:,3];screen=projected[:,:2]/np.where(abs(w)>EPS,w,EPS)[:,None]
        depth=projected[:,2]/np.where(abs(w)>EPS,w,EPS)
        if np.linalg.norm(a-b)<EPS:continue
        selected=seg.get('faces');through=bool(seg.get('through',False))
        # Broad phase once per stroke. Occlusion checks only visit projected triangles
        # crossed by this stroke, not every polygon for every candidate face.
        candidates=[]
        low=np.minimum(a,b);high=np.maximum(a,b)
        for oi,(other,cs) in enumerate(original):
            pts=screen[vp[cs]]
            if np.any(w[vp[cs]]<=EPS) or np.any(pts.max(0)<low-EPS) or np.any(pts.min(0)>high+EPS):continue
            interval=clip(a,b,pts)
            if interval is not None:candidates.append((other,cs,interval))
        for ti,(face,corners) in enumerate(original):
            if selected is not None and face not in selected:continue
            ids=vp[corners]
            if np.any(w[ids]<=EPS):continue
            base=screen[ids];clipped=clip(a,b,base)
            if clipped is None:continue
            pieces=[clipped]
            if not through:
                # Partition wherever another face enters/leaves the stroke. Visibility
                # must not be decided by a single midpoint for a partially hidden face.
                delta=b-a;den=float(delta@delta);cuts=[0.,1.]
                for other,cs,interval in candidates:
                    if other==face:continue
                    if interval is not None:
                        cuts.extend(float((p-a)@delta/den) for p in interval)
                cuts=sorted(set(round(max(0.,min(1.,v)),10) for v in cuts));pieces=[]
                for lo,hi in zip(cuts,cuts[1:]):
                    hit=clip(a+delta*lo,a+delta*hi,base)
                    if hit is None:continue
                    mid=(hit[0]+hit[1])/2;z=float(bary(mid,base)@depth[ids]);occluded=False
                    for other,cs,_ in candidates:
                        if other==face:continue
                        bw=bary(mid,screen[vp[cs]])
                        if bw is not None and min(bw)>=-EPS and bw@depth[vp[cs]]<z-1e-7:occluded=True;break
                    if not occluded:pieces.append(hit)
            for start,end in pieces:
                made=[]
                for tri in rows[ti]:
                    q=np.array(tri)@P[ids];h=np.c_[q,np.ones(3)]@matrix.T;xy=h[:,:2]/h[:,3,None]
                    # Do not reproject/reinterpolate unaffected rows: snapped strokes
                    # often run along an existing edge and round trips create slivers.
                    if clip(start,end,xy) is None:
                        made.append(tri)
                        continue
                    parts=cut_triangle(list(xy),start,end)
                    # A stroke can coincide with the temporary tessellation diagonal.
                    # Still materialise that diagonal when it is not a polygon boundary.
                    hit=clip(start,end,xy)
                    if hit is not None:touched.add(face)
                    for part in parts:
                        weights=[]
                        for p in part:
                            bc=bary(p,xy)
                            if bc is None:
                                raise ValueError('Knife encountered a degenerate projected face; change the view or remove zero-area faces')
                            bc=bc/h[:,3];total=sum(bc)
                            if not np.isfinite(total) or abs(total)<1e-14:
                                raise ValueError('Knife intersection lies on the camera plane; change the view')
                            bc/=total;weights.append(bc@np.array(tri))
                        made.append(weights)
                rows[ti]=made
    if not touched:return geo
    # Key by original point weights, never by position: coincident disconnected shells stay disconnected.
    positions=list(P);pointweights=[{i:1.} for i in range(len(P))];keys={((i,1.),):i for i in range(len(P))};faces=[];sources=[];vertexweights=[]
    supports={tuple(row):[i] for i,row in enumerate(pointweights)}
    def point(cs,weights):
        mapping={}
        for c,v in zip(cs,weights):
            if abs(v)>1e-7:mapping[int(vp[c])]=mapping.get(int(vp[c]),0)+float(v)
        total=sum(mapping.values())
        mapping={i:v/total for i,v in mapping.items()}
        support=tuple(sorted(mapping))
        # Adjacent original triangles can compute the same snapped intersection
        # a few ulps apart. Compare weights within the same support (never weld
        # spatially coincident disconnected shells).
        for index in supports.get(support,[]):
            if max(abs(mapping[i]-pointweights[index][i]) for i in support)<2e-7:
                return index
        key=tuple(sorted((i,round(v,7)) for i,v in mapping.items()))
        if key not in keys:
            keys[key]=len(positions);supports.setdefault(support,[]).append(len(positions));positions.append(sum(P[i]*v for i,v in mapping.items()));pointweights.append(mapping)
        return keys[key]
    for ti,(face,cs) in enumerate(original):
        if face not in touched:continue
        for tri in rows[ti]:
            faces.append([point(cs,bw) for bw in tri]);sources.append(face);vertexweights.append([{int(c):float(v) for c,v in zip(cs,bw) if abs(v)>1e-7} for bw in tri])
    for face in range(geo.nprims):
        if face not in touched:
            cs=list(range(int(off[face]),int(off[face+1])));faces.append([int(vp[c]) for c in cs]);sources.append(face);vertexweights.append([{c:1.} for c in cs])
    # Stitch new points on every original edge into uncut neighbours (no T-junctions).
    edgepoints={}
    for i,weights in enumerate(pointweights):
        if len(weights)==2:edgepoints.setdefault(tuple(sorted(weights)),[]).append((i,weights))
    for fi,face in enumerate(faces):
        expanded=[];attrs=[]
        for j,a in enumerate(face):
            b=face[(j+1)%len(face)];expanded.append(a);attrs.append(vertexweights[fi][j])
            if a>=len(P) or b>=len(P):continue
            for idx,weights in sorted(edgepoints.get(tuple(sorted((a,b))),[]),key=lambda r:r[1].get(b,0)):
                t=weights.get(b,0);aw=vertexweights[fi][j];bw=vertexweights[fi][(j+1)%len(face)];mix={k:aw.get(k,0)*(1-t)+bw.get(k,0)*t for k in aw.keys()|bw.keys()};expanded.append(idx);attrs.append(mix)
        faces[fi]=expanded;vertexweights[fi]=attrs
    # Successive snapped strokes may end on an earlier cut edge. Stitch those
    # endpoints into every coplanar sibling, not just into original mesh edges.
    # Scope candidates to the original face so coincident shells stay separate.
    face_points={}
    for face,source in zip(faces,sources):face_points.setdefault(source,set()).update(face)
    positions_array=np.asarray(positions)
    tolerance=max(float(np.ptp(P,axis=0).max())*1e-8,1e-12)
    for fi,face in enumerate(faces):
        candidates=np.asarray(sorted(face_points[sources[fi]]),dtype=int)
        expanded=[];attrs=[]
        for j,a in enumerate(face):
            b=face[(j+1)%len(face)];expanded.append(a);attrs.append(vertexweights[fi][j])
            delta=positions_array[b]-positions_array[a];den=float(delta@delta)
            if den<=tolerance*tolerance:continue
            relative=positions_array[candidates]-positions_array[a]
            fractions=relative@delta/den
            on=(fractions>1e-8)&(fractions<1-1e-8)&(np.linalg.norm(relative-fractions[:,None]*delta,axis=1)<tolerance)
            for index in np.flatnonzero(on)[np.argsort(fractions[on])]:
                idx=int(candidates[index]);t=float(fractions[index])
                aw=vertexweights[fi][j];bw=vertexweights[fi][(j+1)%len(face)]
                expanded.append(idx);attrs.append({k:aw.get(k,0)*(1-t)+bw.get(k,0)*t for k in aw.keys()|bw.keys()})
        faces[fi]=expanded;vertexweights[fi]=attrs
    def protected(a,b):
        for seg in segments:
            matrix=np.asarray(seg['matrix']).reshape(4,4,order='F');q=np.c_[np.array([positions[a],positions[b]]),np.ones(2)]@matrix.T
            if np.any(q[:,3]<=EPS):continue
            q=q[:,:2]/q[:,3,None];u=np.asarray(seg['a']);v=np.asarray(seg['b']);d=v-u;den=d@d
            if den<EPS:continue
            if all(abs(cross(d,p-u))<1e-7 and -EPS<=((p-u)@d/den)<=1+EPS for p in q):return True
        return False
    # Dissolve triangulation-only diagonals back into polygon regions. A closed
    # interior loop retains necessary bridge edges because mesh faces cannot have holes.
    changed=True
    while changed:
        changed=False;owners={}
        for i,f in enumerate(faces):
            for a,b in zip(f,f[1:]+f[:1]):owners.setdefault(tuple(sorted((a,b))),[]).append(i)
        for (a,b),ii in owners.items():
            if len(ii)!=2:continue
            i,j=ii
            if sources[i]!=sources[j] or protected(a,b):continue
            boundary={};weights={};valid=True
            for k in (i,j):
                f=faces[k]
                for n,(u,v) in enumerate(zip(f,f[1:]+f[:1])):
                    weights[u]=vertexweights[k][n]
                    if set((u,v))==set((a,b)):continue
                    if u in boundary:valid=False;break
                    boundary[u]=v
                else:continue
                break
            if not valid or not boundary:continue
            start=next(iter(boundary));loop=[];at=start
            while at in boundary and at not in loop:loop.append(at);at=boundary[at]
            if at!=start or len(loop)!=len(boundary) or len(loop)<3:continue
            faces[i]=loop;vertexweights[i]=[weights[u] for u in loop]
            del faces[j];del sources[j];del vertexweights[j];changed=True;break
    def interpolate(table,weights):
        result={}
        for name,array in table.items():
            result[name]=np.asarray([sum(array[i]*v for i,v in row.items()) if array.dtype.kind=='f' else array[max(row,key=row.get)] for row in weights],dtype=array.dtype)
        return result
    pointattrs=interpolate(geo.point,pointweights)
    pointattrs['point_group_knife']=np.array([int(i>=len(P)) for i in range(len(pointweights))],np.int32)
    out=B.make(np.asarray(positions),np.array([i for f in faces for i in f]),B.offsets_from_sizes([len(f) for f in faces]),pointattrs,interpolate(geo.vertex,[r for f in vertexweights for r in f]),B.gather(geo.prim,np.array(sources)),geo.detail)
    # Preserve edge data on subdivided source edges, not only unchanged endpoint pairs.
    old=topo.edges(geo);new=topo.edges(out);lookup={tuple(pair):i for i,pair in enumerate(old.points)}
    origin=[]
    for a,b in new.points:
        support=tuple(sorted(pointweights[a].keys()|pointweights[b].keys()))
        origin.append(lookup.get(support,-1))
    origin=np.asarray(origin);valid=origin>=0
    for name in geo.vertex:
        if name.startswith(edges.PREFIX) or name==edges.CREASE:
            values=np.zeros(len(new.points),geo.vertex[name].dtype);values[valid]=edges.values(geo,name)[origin[valid]];out=edges.write(out,name,values)
    cut=np.array([int(origin[i]<0 and protected(a,b)) for i,(a,b) in enumerate(new.points)],np.int32)
    return edges.write(out,'edge_group_knife',cut)
