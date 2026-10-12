"""Polygon lofting and conservative planar profile offsets."""
import numpy as np
from .. import edges, groups
from ..geometry import Topology
from ..outgroups import OutGroup, OutGroups
from ..sdk import Bool, Enum, Float, Str, node
from . import _build as B
from .edge_tools import _rebuild


@node('skin_profiles', inputs=['geo'], category='model', params=[
    Str('group', '', doc='Profiles in primitive-number order; equal point counts and matching closure required'),
    Bool('keep_profiles', False),
    Bool('close_loft', False, doc='Connect the last profile back to the first (at least three profiles)'),
])
def skin(geo, group, keep_profiles, close_loft):
    """Loft quads through multiple ordered polygon/polyline profiles. Correspondence is by corner index, without resampling."""
    sel = groups.mask(geo, 'prim', group)
    sel = np.ones(geo.nprims, bool) if sel is None else sel
    ids = np.flatnonzero(sel)
    if len(ids) < (3 if close_loft else 2):
        raise ValueError('skin requires at least two profiles (three for close_loft)')
    t = geo.topology
    sizes, closed = t.prim_sizes()[ids], t.closed_mask()[ids]
    if np.any(sizes != sizes[0]) or np.any(closed != closed[0]) or sizes[0] < (3 if closed[0] else 2):
        raise ValueError('skin needs equal-count profiles with matching open/closed state')
    if sum(sizes) > 1_000_000:
        raise ValueError('skin limit: one million profile corners')
    vs = [list(range(t.prim_offsets[p], t.prim_offsets[p+1])) for p in ids]
    for v in vs:
        if len(set(t.vertex_point[v])) != len(v):
            raise ValueError('skin profiles must not repeat points')
    kept = np.flatnonzero(~sel | keep_profiles)
    faces = [list(range(t.prim_offsets[p],t.prim_offsets[p+1])) for p in kept]
    sources = kept.tolist()
    flags = t.closed_mask()[kept].tolist()
    for k in range(len(ids) if close_loft else len(ids)-1):
        a, b = vs[k], vs[(k+1)%len(ids)]
        for j in range(len(a) if closed[0] else len(a)-1):
            h = (j+1)%len(a)
            f = [a[j],a[h],b[h],b[j]]
            q = geo.P[t.vertex_point[f]].astype(float)
            if len(set(t.vertex_point[f])) < 4 or np.linalg.norm(np.cross(q[1]-q[0],q[2]-q[0])) + np.linalg.norm(np.cross(q[2]-q[0],q[3]-q[0])) < 1e-12:
                raise ValueError('skin would create a collapsed quad; check profile order/correspondence')
            faces.append(f)
            sources.append(int(ids[k]))
            flags.append(True)
    return _rebuild(geo, faces, sources, np.array(flags,bool))


def _offset(P, distance, tolerance, miter_limit):
    """Miter intersection of adjacent offset lines for a simple convex planar loop."""
    P = np.asarray(P,float)
    if len(P) < 3 or not np.isfinite(P).all():
        raise ValueError('offset requires at least three finite points')
    center = P.mean(axis=0)
    _, singular, axes = np.linalg.svd(P-center, full_matrices=False)
    scale = max(float(np.ptp(P,axis=0).max()),1e-12)
    eps = max(tolerance*scale,1e-12)
    if singular[1] < eps or np.max(np.abs((P-center)@axes[2])) > eps:
        raise ValueError('offset profile is degenerate or nonplanar')
    xy = (P-center)@axes[:2].T
    d = np.roll(xy,-1,axis=0)-xy
    lengths = np.linalg.norm(d,axis=1)
    if np.any(lengths < eps):
        raise ValueError('offset profile contains repeated/zero-length edges')
    d /= lengths[:,None]
    cross = lambda a,b: a[...,0]*b[...,1]-a[...,1]*b[...,0]
    area = np.sum(cross(xy,np.roll(xy,-1,axis=0)))/2
    sign = 1 if area > 0 else -1
    turns = cross(np.roll(d,1,axis=0),d)*sign
    if np.any(turns <= tolerance):
        raise ValueError('initial polyexpand2d supports strictly convex loops without collinear corners')
    # Every vertex must lie inside every supporting edge: also rejects star/self-crossing loops.
    for i in range(len(xy)):
        if np.any(cross(d[i],xy-xy[i])*sign < -eps):
            raise ValueError('offset profile must be simple and convex')
    normal = sign*np.stack([d[:,1],-d[:,0]],axis=1)
    prev_d, prev_n = np.roll(d,1,axis=0),np.roll(normal,1,axis=0)
    delta = distance*(normal-prev_n)
    along = cross(delta,d)/cross(prev_d,d)
    result = xy + distance*prev_n + along[:,None]*prev_d
    if distance and np.max(np.linalg.norm(result-xy,axis=1)) > abs(distance)*miter_limit + eps:
        raise ValueError('offset exceeds miter_limit; reduce distance or use a less acute profile')
    rd = np.roll(result,-1,axis=0)-result
    if np.any(np.sum(rd*d,axis=1) <= eps):
        raise ValueError('inward offset collapses or inverts the profile')
    return center + result@axes[:2]


EXPAND_OUT = OutGroups(
    OutGroup('inside', 'Inside Group', 'inside', 'prim', doc='The offset loops that lie inside the input loop'),
    OutGroup('outside', 'Outside Group', 'outside', 'prim', doc='The offset loops that lie outside the input loop'),
)
EXPAND_ATTRS = (('edgedist', 'Edge Distance Attribute', 'edgedist'), ('edgespeed', 'Edge Speed Attribute', 'edgespeed'))


def _attribute_pairs(items):
    """Houdini-style attribute outputs: a checkbox and a name, drawn like the Output Groups section."""
    out = []
    for key, label, name in items:
        out.append(Bool('outattr_' + key, False, label=label, doc=f'Write the point attribute {name!r} on the offset points'))
        out.append(Str('outattr_' + key + '_name', name, label=label + ' name'))
    for p in out:
        object.__setattr__(p, 'ui', 'outgroup' if p.kind == 'bool' else 'outgroup_name')
    return out


@node('polyexpand2d', inputs=['geo'], category='model', params=[
    Float('distance', 0.1, doc='Positive expands, negative insets; winding-independent. With mode inside_outside it is the offset '
                               'amount (its absolute value)'),
    Str('group', '', doc='Closed convex planar profile primitives'),
    Enum('mode', ['single', 'inside_outside'], doc='single: one offset loop, outwards for a positive distance and inwards for a negative '
                                                  'one. inside_outside: the outside loop and the inside loop, as Houdini outputs them'),
    Bool('output_inside', True, doc='inside_outside: make the inside loop'),
    Bool('output_outside', True, doc='inside_outside: make the outside loop'),
    Bool('keep_input', False, doc='Keep the input loops as well as the offset ones'),
    Float('tolerance', 1e-6, min=1e-10, max=0.01, doc='Planarity tolerance relative to profile extent'),
    Float('miter_limit', 10, min=1, doc='Maximum corner displacement / absolute offset'),
    *EXPAND_OUT.params(),
    *_attribute_pairs(EXPAND_ATTRS),
])
def polyexpand2d(geo, distance, group, tolerance, miter_limit, mode='single', output_inside=True, output_outside=True,
                 keep_input=False, **og):
    """Offset independent convex closed planar loops in their own planes. Reject concavity, holes/union requests and collapse."""
    sel = groups.mask(geo,'prim',group)
    sel = np.ones(geo.nprims,bool) if sel is None else sel
    if mode == 'single':
        jobs = [(float(distance), 'outside' if distance > 0 else 'inside')]
    else:
        jobs = ([(abs(float(distance)), 'outside')] if output_outside else []) + ([(-abs(float(distance)), 'inside')] if output_inside else [])
    if not sel.any() or distance == 0 or not jobs:
        return geo
    t = geo.topology
    if np.any(~t.closed_mask()[sel]):
        raise ValueError('initial polyexpand2d requires closed convex profiles')
    polys = t.polys()
    offs = t.prim_offsets.astype(np.int64)
    positions, source = [geo.P], list(range(geo.npoints))
    dist_attr, speed_attr = [np.zeros(geo.npoints)], [np.zeros(geo.npoints)]
    count = geo.npoints
    out_polys, src_prim, src_vert, kind = [], [], [], []              # per output primitive; kind 1 inside, 2 outside
    for p in range(geo.nprims):
        ids = polys[p]
        if not sel[p] or keep_input:
            out_polys.append(list(ids)), src_prim.append(p), kind.append(0)
            src_vert.extend(range(offs[p], offs[p + 1]))
        if not sel[p]:
            continue
        for d, which in jobs:
            moved = _offset(geo.P[ids],d,tolerance,miter_limit)
            positions.append(moved)
            source.extend(ids)
            dist_attr.append(np.full(len(ids), d))
            speed_attr.append(np.linalg.norm(moved - geo.P[ids].astype(np.float64), axis=1) / max(abs(d), 1e-30))
            out_polys.append(list(range(count,count+len(ids)))), src_prim.append(p), kind.append(1 if which == 'inside' else 2)
            src_vert.extend(range(offs[p], offs[p + 1]))
            count += len(ids)
    closed = t.closed_mask()[np.array(src_prim)]
    newtopo = Topology.from_polys(out_polys,closed)
    point = B.gather(geo.point,np.array(source))
    P = np.concatenate(positions).astype(np.float32)
    out = B.make(P,newtopo.vertex_point,newtopo.prim_offsets,point,B.gather(geo.vertex,np.array(src_vert)),
                 B.gather(geo.prim,np.array(src_prim)),geo.detail,newtopo.closed)
    out = edges.remap(geo,out,source)
    kind = np.array(kind)
    out = EXPAND_OUT.write(out, og, 'inside', lambda: kind == 1)
    out = EXPAND_OUT.write(out, og, 'outside', lambda: kind == 2)
    for key, arr in (('edgedist', np.concatenate(dist_attr)), ('edgespeed', np.concatenate(speed_attr))):
        if og.get('outattr_' + key) and str(og.get('outattr_' + key + '_name', key)).strip():
            out.point[str(og['outattr_' + key + '_name']).strip()] = arr.astype(np.float32)
    return out
