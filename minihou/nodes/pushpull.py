"""Bounded solid push/pull and named inset/outset tools."""
import numpy as np
from .. import groups
from ..sdk import node, Str, Float, Enum
from . import _build as B
from .model import polyextrude
from .bevel import twins
from .boolean import boolean, _triangles
from .normal import prim_normals


def _solid(g, label):
    B.require_polygons(g, label)
    if g.nprims and (np.any(twins(g) < 0) or not np.isfinite(g.P).all()):
        raise ValueError(label + " requires a closed, consistently wound manifold solid")
    for face in range(g.nprims):
        t=g.topology; q=g.P[t.vertex_point[t.prim_offsets[face]:t.prim_offsets[face+1]]].astype(float)
        if len(q)>256:
            raise ValueError(label + " supports polygons with at most 256 corners; triangulate first")
        n=np.cross(q-q.mean(0),np.roll(q,-1,axis=0)-q.mean(0)).sum(0)
        axis=int(np.argmax(np.abs(n))); xy=np.delete(q,axis,axis=1)
        cross=lambda a,b: a[0]*b[1]-a[1]*b[0]
        for i in range(len(q)):
            a,b=xy[i],xy[(i+1)%len(q)]
            for j in range(i+2,len(q)-(1 if i==0 else 0)):
                c,d=xy[j],xy[(j+1)%len(q)]
                if cross(b-a,c-a)*cross(b-a,d-a)<0 and cross(d-c,a-c)*cross(d-c,b-c)<0:
                    angle=abs(cross(b-a,d-c))/max(np.linalg.norm(b-a)*np.linalg.norm(d-c),1e-30)
                    if angle>1e-3:
                        raise ValueError(label + " contains a self-crossing polygon")
    if g.nprims:
        triangles, _, _ = _triangles(g)
        p = g.P[triangles].astype(float)
        volume = np.einsum("ij,ij->i", p[:,0], np.cross(p[:,1],p[:,2])).sum()/6
        if volume <= 0:
            raise ValueError(label + " requires outward-facing solid geometry; reverse inward faces first")


@node("pushpull", inputs=["geo"], category="model", params=[
    Str("group", "", doc="Exactly one convex planar face; inset a face first to create a smaller opening"),
    Float("distance", .25, soft_min=-2, soft_max=2, doc="Positive adds solid; negative removes solid, including through the opposite wall"),
])
def pushpull(geo, group, distance):
    """Push/pull a single planar convex face with solid union/subtraction. Closed meshes only.
    Unlike ordinary extrusion, an inward pull removes intersected solid and can punch through.
    Unsupported or non-manifold results fail clearly rather than returning broken geometry."""
    if geo.nprims > 20000 or geo.npoints > 50000:
        raise ValueError("Push/Pull supports up to 20,000 faces / 50,000 points; simplify the solid first")
    _solid(geo, "Push/Pull")
    selected = groups.mask(geo, "prim", group)
    ids = np.flatnonzero(selected) if selected is not None else np.arange(geo.nprims)
    if len(ids) != 1:
        raise ValueError("Push/Pull: select exactly one planar convex face (use Inset for a smaller opening)")
    face = int(ids[0]); t = geo.topology
    indices = t.vertex_point[t.prim_offsets[face]:t.prim_offsets[face+1]]
    points = geo.P[indices].astype(float)
    n = prim_normals(geo)[face].astype(float); n /= max(np.linalg.norm(n),1e-30)
    extent = max(float(np.ptp(geo.P,axis=0).max()),1e-8)
    tolerance = extent * 1e-6
    if len(points)<3 or np.linalg.norm(n)<.5 or np.max(np.abs((points-points[0])@n))>tolerance:
        raise ValueError("Push/Pull: selected face must be non-degenerate and planar")
    edges = np.roll(points,-1,axis=0)-points
    if np.any(np.cross(edges,np.roll(edges,-1,axis=0))@n < -tolerance*tolerance):
        raise ValueError("Push/Pull: concave faces are not supported; split into convex faces first")
    if abs(distance) <= tolerance:
        return geo.copy()
    overlap=tolerance*10
    low, high = (-overlap,distance) if distance>0 else (distance,overlap)
    count=len(points)
    positions=np.concatenate([points+n*low,points+n*high])
    faces=[list(reversed(range(count))),list(range(count,2*count))]
    faces += [[i,(i+1)%count,(i+1)%count+count,i+count] for i in range(count)]
    cutter=B.make(positions,np.concatenate(faces),B.offsets_from_sizes([len(f) for f in faces]),
                  prim={k:np.repeat(v[face:face+1],len(faces),axis=0) for k,v in geo.prim.items()},detail=geo.detail)
    errors=[]
    # Bounded changes of solve frame avoid coincident-edge degeneracies in the
    # existing boolean kernel. Every candidate must pass the same solid checks.
    for angle in (0., .37, .91, 1.57):
        try:
            u=edges[0]/np.linalg.norm(edges[0]); v=np.cross(n,u)
            u,v=u*np.cos(angle)+v*np.sin(angle),v*np.cos(angle)-u*np.sin(angle)
            basis=np.stack([u,v,n],axis=1); origin=points.mean(0)
            local=geo.copy(); tool=cutter.copy()
            local.point["P"]=((geo.P-origin)@basis).astype(np.float32)
            tool.point["P"]=((cutter.P-origin)@basis).astype(np.float32)
            result=boolean(local,tool,"union" if distance>0 else "subtract")
            result.point["P"]=(result.P@basis.T+origin).astype(np.float32)
            _solid(result,"Push/Pull result")
            result.validate()
            break
        except (ValueError, RuntimeError) as exc:
            errors.append(str(exc))
    else:
        raise ValueError("Push/Pull could not produce a watertight solid; adjust the distance or split the face. " + errors[-1])

    return result


@node("inset", inputs=["geo"], category="model", params=[Str("group", ""), Float("amount", .1, min=0, soft_max=1), Enum("mode", ["connected","individual"])])
def inset(geo, group, amount, mode):
    """Inset selected faces without height; amount is the perpendicular distance from the original boundary."""
    return polyextrude(geo,group,0.,amount,mode,False)


@node("outset", inputs=["geo"], category="model", params=[Str("group", ""), Float("amount", .1, min=0, soft_max=1), Enum("mode", ["connected","individual"])])
def outset(geo, group, amount, mode):
    """Offset selected planar boundaries outward, without height. Does not resolve collisions with neighbours."""
    return polyextrude(geo,group,0.,-amount,mode,False)
