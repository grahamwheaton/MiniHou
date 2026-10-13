"""Portable spatial falloff, trilinear lattice and local affine point deformers."""
import itertools
import json
import numpy as np
from .. import groups, topo
from ..geometry import Geometry
from ..sdk import Enum, Float, Int, Str, Vec3, node
from ._util import euler_xyz_matrix, normalize_rows


def _apply(geo, mapping, group):
    """Apply a position map; transform existing N by its local inverse transpose."""
    P = geo.P.astype(np.float64)
    if not np.isfinite(P).all():
        raise ValueError('deformation requires finite positions')
    sel = groups.mask(geo,'point',group)
    sel = np.ones(geo.npoints,bool) if sel is None else sel
    if not sel.any():
        return geo
    out = geo.copy()
    Q = P.copy()
    Q[sel] = mapping(P[sel])
    if not np.isfinite(Q).all():
        raise ValueError('deformation produced nonfinite positions')
    out.point['P'] = Q.astype(np.float32)
    if any('N' in geo.table(d) for d in ('point','vertex','prim')):
        h = max(float(np.ptp(P,axis=0).max()),1.0)*1e-5
        jac = np.broadcast_to(np.eye(3),(geo.npoints,3,3)).copy()
        for k in range(3):
            offset = np.eye(3)[k]*h
            jac[sel,:,k] = (mapping(P[sel]+offset)-mapping(P[sel]-offset))/(2*h)
        for domain in ('point','vertex','prim'):
            table = geo.table(domain)
            if 'N' not in table:
                continue
            N = table['N']
            if N.shape != (table.size,3):
                raise ValueError('N must have three components')
            if domain == 'point':
                J, active = jac, sel
            elif domain == 'vertex':
                J, active = jac[geo.topology.vertex_point], sel[geo.topology.vertex_point]
            else:
                pv, vp = topo.prim_of_vertex(geo),geo.topology.vertex_point
                J = np.zeros((geo.nprims,3,3))
                np.add.at(J,pv,jac[vp])
                J /= np.maximum(geo.topology.prim_sizes(),1)[:,None,None]
                active = np.bincount(pv,weights=sel[vp],minlength=geo.nprims)>0
            result = N.copy()
            result[active] = normalize_rows(np.einsum('ni,nij->nj',N[active],np.linalg.pinv(J[active])))
            out.table(domain)['N'] = result
    return out


@node('soft_transform', inputs=['geo'], category='deform', params=[
    Vec3('center',(0,0,0),doc='Falloff center and transform pivot'),
    Float('radius',1,min=1e-8),
    Vec3('translate',(0,0,0)), Vec3('rotate',(0,0,0),doc='Euler XYZ degrees'), Vec3('scale',(1,1,1)),
    Enum('falloff',['smooth','linear']),
    Str('group','',doc='Only these points can move; distance is Euclidean from center'),
])
def soft_transform(geo, center, radius, translate, rotate, scale, falloff, group):
    """Transform with radial smoothstep/linear falloff. No geodesic distance or selection-centred multi-source capture."""
    center = np.asarray(center,float)
    M = euler_xyz_matrix(rotate)*np.asarray(scale)[None,:]
    def mapping(P):
        w = np.clip(1-np.linalg.norm(P-center,axis=1)/radius,0,1)
        if falloff == 'smooth':
            w = w*w*(3-2*w)
        target = (P-center)@M.T+center+translate
        return P+w[:,None]*(target-P)
    return _apply(geo,mapping,group)


@node('lattice_grid', category='source', params=[
    Vec3('center',(0,0,0)), Vec3('size',(2,2,2)),
    Int('columns',2,min=2,max=64), Int('rows',2,min=2,max=64), Int('layers',2,min=2,max=64),
    Str('offsets','[]',doc='Per-control XYZ displacement values edited by viewport handles. Changing resolution resets incompatible offsets.'),
])
def lattice_grid(center,size,columns,rows,layers,offsets="[]"):
    """Complete 3D control-point grid for Lattice: columns X, rows Y, layers Z; X varies fastest."""
    if np.any(np.asarray(size)<=0):
        raise ValueError('lattice_grid size must be positive in all three dimensions')
    axes = [np.linspace(c-s/2,c+s/2,n) for c,s,n in zip(center,size,(columns,rows,layers))]
    P = np.array([[x,y,z] for z in axes[2] for y in axes[1] for x in axes[0]],np.float32)
    delta = np.asarray(json.loads(offsets), dtype=float)
    if delta.size and (delta.ndim != 1 or not np.isfinite(delta).all()):
        raise ValueError('lattice offsets must be a finite flat XYZ array')
    if delta.size == P.size:
        P += delta.reshape(P.shape)
    out = Geometry(len(P))
    out.point['P'] = P
    out.detail['lattice_dimensions'] = [int(columns), int(rows), int(layers)]
    return out


def _controls(rest,deformed):
    if rest is None or deformed is None or not rest.npoints:
        raise ValueError('connect nonempty rest and deformed control inputs')
    if rest.npoints != deformed.npoints or rest.topology.digest != deformed.topology.digest:
        raise ValueError('rest and deformed controls must have identical topology and point correspondence')
    A,B = rest.P.astype(float),deformed.P.astype(float)
    if not np.isfinite(A).all() or not np.isfinite(B).all():
        raise ValueError('control positions must be finite')
    return A,B


@node('lattice', inputs=['geo','rest','deformed'], category='deform', params=[
    Str('group',''), Enum('outside',['clamp','unchanged'],doc='Outside rest bounds: clamp displacement lookup or leave points unchanged'),
])
def lattice(geo,rest,deformed,group,outside):
    """Trilinear displacement from a complete axis-aligned rectilinear rest grid. Deformed controls retain point order."""
    A,B = _controls(rest,deformed)
    axes = [np.unique(A[:,k]) for k in range(3)]
    dims = tuple(map(len,axes))
    if min(dims)<2 or np.prod(dims)!=len(A):
        raise ValueError('rest lattice must form a complete rectangular 3D grid aligned with X, Y and Z')
    coords = tuple(np.searchsorted(axes[k],A[:,k]) for k in range(3))
    flat = np.ravel_multi_index(coords,dims)
    if len(np.unique(flat))!=len(A):
        raise ValueError('rest lattice has duplicate or missing control points')
    delta = np.zeros(dims+(3,))
    delta[coords] = B-A
    def mapping(P):
        indices,weights = [],[]
        inside = np.ones(len(P),bool)
        for k,axis in enumerate(axes):
            q = np.clip(P[:,k],axis[0],axis[-1])
            idx = np.clip(np.searchsorted(axis,q,side='right')-1,0,len(axis)-2)
            indices.append(idx)
            weights.append((q-axis[idx])/(axis[idx+1]-axis[idx]))
            inside &= (P[:,k]>=axis[0]) & (P[:,k]<=axis[-1])
        D = np.zeros_like(P)
        for corner in itertools.product((0,1),repeat=3):
            w = np.ones(len(P))
            for k,c in enumerate(corner):
                w *= weights[k] if c else 1-weights[k]
            D += w[:,None]*delta[tuple(indices[k]+corner[k] for k in range(3))]
        if outside == 'unchanged':
            D[~inside] = 0
        return P+D
    return _apply(geo,mapping,group)


@node('point_deform', inputs=['geo','rest','deformed'], category='deform', params=[
    Float('radius',0,min=0,doc='0: nearest controls without a radius cutoff; otherwise uncaptured points stay unchanged'),
    Int('max_points',16,min=1,max=128,doc='Nearest control samples per point; at least four noncoplanar samples for full affine reproduction'),
    Str('group',''),
])
def point_deform(geo,rest,deformed,radius,max_points,group):
    """Weighted local affine displacement fit to corresponding control clouds. NumPy-only; no stored capture or piece matching."""
    A,B = _controls(rest,deformed)
    if geo.npoints*len(A)>50_000_000:
        raise ValueError('initial point_deform limit: 50 million query/control pairs; reduce the control cloud')
    size = max(float(np.ptp(A,axis=0).max()),1e-6)
    floor = (size*1e-4)**2
    delta = B-A
    def mapping(P):
        Q = P.copy()
        for i,p in enumerate(P):
            d2 = np.sum((A-p)**2,axis=1)
            ix = np.argsort(d2,kind='stable')[:max_points]
            if radius:
                ix = ix[d2[ix]<=radius*radius]
            if not len(ix):
                continue
            w = 1/(d2[ix]+floor)
            w /= w.sum()
            center = w@A[ix]
            mean = w@delta[ix]
            X = A[ix]-center
            Y = delta[ix]-mean
            # Minimum-norm displacement gradient: unconstrained directions keep identity.
            grad = np.linalg.pinv(X*np.sqrt(w[:,None]),rcond=1e-8)@(Y*np.sqrt(w[:,None]))
            Q[i] += mean+(p-center)@grad
        return Q
    return _apply(geo,mapping,group)
