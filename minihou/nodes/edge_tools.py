"""Edge groups and conservative polygon bridge/dissolve operations."""
import numpy as np
from .. import edges, groups, topo
from ..geometry import Geometry, Topology
from ..sdk import Bool, Enum, Float, Int, Str, node
from . import _build as B


@node('crease', inputs=['geo'], category='model', params=[
    Enum('domain', ['edges', 'points']),
    Str('group', '', doc='Edge selection or point group; empty selects all'),
    Float('sharpness', 2, min=0, max=10, doc='Semi-sharp subdivision levels; zero clears selected creases'),
])
def crease(geo, domain, group, sharpness):
    """Set edge creaseweight or point cornerweight for Subdivide; sharpness decays by one per level."""
    if domain == 'edges':
        data = edges.values(geo, edges.CREASE)
        data[edges.mask(geo, group)] = sharpness
        return edges.write(geo, edges.CREASE, data)
    sel = groups.mask(geo, 'point', group)
    data = np.array(geo.point.get('cornerweight', np.zeros(geo.npoints)), dtype=np.float32, copy=True)
    if data.shape != (geo.npoints,):
        raise ValueError('cornerweight must be a scalar point attribute')
    data[np.ones(geo.npoints, bool) if sel is None else sel] = sharpness
    out = geo.copy()
    out.point['cornerweight'] = data
    return out


@node('edge_group', inputs=['geo'], category='group', params=[
    Str('name', 'edges'),
    Enum('method', ['pattern', 'boundary', 'points', 'primitives']),
    Str('pattern', '*', doc='Edges: ids/ranges, p0-1 pairs, names, wildcards, ^exclusions. Other methods: point/primitive group pattern.'),
    Enum('combine', ['replace', 'union', 'intersect', 'subtract']),
])
def edge_group(geo, name, method, pattern, combine):
    """Create a derived edge group; stored as edge_group_<name> on outgoing corners for protocol compatibility."""
    groups.attr_name(name)
    e = topo.edges(geo)
    if method == 'pattern':
        m = edges.mask(geo, pattern)
    elif method == 'boundary':
        m = e.count == 1
    else:
        domain = 'point' if method == 'points' else 'prim'
        sel = groups.mask(geo, domain, pattern)
        sel = np.ones(geo.npoints if domain == 'point' else geo.nprims, bool) if sel is None else sel
        if domain == 'point':
            m = sel[e.points].all(axis=1)
        else:
            m = np.zeros(len(e.points), bool)
            valid = (e.of_vertex >= 0) & sel[topo.prim_of_vertex(geo)]
            m[e.of_vertex[valid]] = True
    key = edges.PREFIX + name
    old = edges.values(geo, key) != 0
    if combine == 'union':
        m |= old
    elif combine == 'intersect':
        m &= old
    elif combine == 'subtract':
        m = old & ~m
    return edges.write(geo, key, m.astype(np.int32))


@node('edge_group_promote', inputs=['geo'], category='group', params=[
    Str('pattern', '*', doc='Derived edge selection'), Str('name', 'edge_points'),
    Enum('to', ['points', 'primitives']),
])
def edge_group_promote(geo, pattern, name, to):
    """Copy an edge selection to incident points or primitives (any selected edge). Source edge groups remain."""
    e = topo.edges(geo)
    m = edges.mask(geo, pattern)
    if to == 'points':
        sel = np.zeros(geo.npoints, bool)
        sel[e.points[m].ravel()] = True
    else:
        sel = np.zeros(geo.nprims, bool)
        valid = e.of_vertex >= 0
        sel[topo.prim_of_vertex(geo)[valid][m[e.of_vertex[valid]]]] = True
    return groups.write(geo, 'point' if to == 'points' else 'prim', name, sel)


def _rebuild(geo, corners, sources, closed=None):
    """Build from original corner rows, retaining all point/vertex/primitive data."""
    ix = np.asarray([v for face in corners for v in face], np.int64)
    out = B.make(geo.P, geo.topology.vertex_point[ix], B.offsets_from_sizes([len(f) for f in corners]),
                 dict(geo.point.items()), B.gather(geo.vertex, ix),
                 B.gather(geo.prim, np.asarray(sources, np.int64)), geo.detail, closed)
    return edges.remap(geo, out)


def _oriented_chain(geo, path, closed, reverse_boundary):
    """Orient a chain with or against its unique existing polygon boundary."""
    vp, nxt = geo.topology.vertex_point, topo.next_vertex(geo)
    polygon = geo.topology.closed_mask()[topo.prim_of_vertex(geo)]
    signs = []
    for a, b in zip(path, path[1:] + (path[:1] if closed else [])):
        hits = np.flatnonzero(polygon & (((vp == a) & (vp[nxt] == b)) | ((vp == b) & (vp[nxt] == a))))
        if len(hits) > 1:
            raise ValueError('bridge requires boundary edges, not polygon interior edges')
        if len(hits):
            signs.append(vp[hits[0]] == a)
    if signs and not all(s == signs[0] for s in signs):
        raise ValueError('boundary winding is inconsistent')
    if signs and signs[0] == reverse_boundary:
        path = path[::-1]
    return path, bool(signs)


@node('polybridge', inputs=['geo'], category='model', params=[
    Str('source', '', doc='One boundary chain/loop: edge group, ids or pA-B pairs'),
    Str('destination', '', doc='Disjoint chain/loop with the same number of points'),
    Int('segments', 1, min=1, max=256),
    Int('twist', 0, doc='Destination loop cyclic point shift'),
    Bool('reverse', False, doc='Reverse destination correspondence (standalone profiles only)'),
])
def polybridge(geo, source, destination, segments, twist, reverse):
    """Linear quad bridge between equal-size boundary chains/loops. No unequal resampling or intersection repair."""
    if not source.strip() or not destination.strip():
        raise ValueError('specify both source and destination edge selections')
    a, ca = edges.chain(geo, edges.mask(geo, source))
    b, cb = edges.chain(geo, edges.mask(geo, destination))
    if ca != cb or len(a) != len(b) or set(a) & set(b):
        raise ValueError('bridge needs disjoint profiles of equal point count and matching open/closed state')
    a, _ = _oriented_chain(geo, a, ca, True)
    b, oriented = _oriented_chain(geo, b, cb, False)
    if reverse:
        if oriented:
            raise ValueError('reverse would conflict with polygon boundary winding')
        b = b[::-1]
    if twist and not ca:
        raise ValueError('twist requires closed loops')
    if ca:
        b = list(np.roll(b, -twist))
    n = len(a)
    if n * segments > 1_000_000:
        raise ValueError('bridge limit: one million profile samples')
    # Pick the corner belonging to the selected profile edge, avoiding unrelated UV seams.
    vp = geo.topology.vertex_point
    e = topo.edges(geo)
    def corners(path, pattern):
        chosen = edges.mask(geo, pattern)
        valid = e.of_vertex >= 0
        hit = np.zeros(geo.nvertices, bool)
        hit[valid] = chosen[e.of_vertex[valid]]
        hit |= hit[topo.prev_vertex(geo)] & geo.topology.closed_mask()[topo.prim_of_vertex(geo)]
        return np.array([np.flatnonzero((vp == p) & hit)[0] if np.any((vp == p) & hit)
                         else np.flatnonzero(vp == p)[0] for p in path])
    va, vb = corners(a, source), corners(b, destination)
    point_parts = [dict(geo.point.items())]
    rings = [a]
    for k in range(1, segments):
        rings.append(list(range(geo.npoints + (k-1)*n, geo.npoints + k*n)))
        point_parts.append(B.lerp(geo.point, np.array(a), np.array(b), np.full(n, k/segments)))
    rings.append(b)
    point = B.concat(point_parts)
    newfaces, ia, ib, tt, ps = [], [], [], [], []
    pv = topo.prim_of_vertex(geo)
    for k in range(segments):
        for j in range(n if ca else n-1):
            h = (j+1) % n
            newfaces.append([rings[k][j], rings[k][h], rings[k+1][h], rings[k+1][j]])
            ia.extend([va[j], va[h], va[h], va[j]])
            ib.extend([vb[j], vb[h], vb[h], vb[j]])
            tt.extend([k/segments, k/segments, (k+1)/segments, (k+1)/segments])
            ps.append(pv[va[j]])
    # Reject collapsed bridge faces rather than silently introducing zero area geometry.
    for f in newfaces:
        q = point['P'][f].astype(float)
        if np.linalg.norm(np.cross(q[1]-q[0], q[2]-q[0])) + np.linalg.norm(np.cross(q[2]-q[0], q[3]-q[0])) < 1e-12:
            raise ValueError('bridge contains a collapsed face')
    t = Topology.from_polys(geo.topology.polys()+newfaces,
                           np.r_[geo.topology.closed_mask(), np.ones(len(newfaces), bool)])
    vtx = B.concat([dict(geo.vertex.items()), B.lerp(geo.vertex, np.array(ia), np.array(ib), np.array(tt))])
    prim = B.concat([dict(geo.prim.items()), B.gather(geo.prim, np.array(ps))])
    out = B.make(point['P'], t.vertex_point, t.prim_offsets, point, vtx, prim, geo.detail, t.closed)
    return edges.remap(geo, out)


@node('dissolve', inputs=['geo'], category='model', params=[
    Str('group', '', doc='Interior edges to remove; empty selection is a no-op'),
    Float('angle', 0.1, min=0, max=180, doc='Maximum angle between faces, degrees'),
    Bool('preserve_seams', True, doc='Reject merges across differing corner data or primitive attributes/materials'),
])
def dissolve(geo, group, angle, preserve_seams):
    """Merge selected manifold face regions into simple n-gons. Reject holes, branches, boundary edges and seams."""
    if not group.strip():
        return geo
    B.require_polygons(geo, 'dissolve')
    e = topo.edges(geo)
    chosen = edges.mask(geo, group)
    if np.any(e.count[chosen] != 2):
        raise ValueError('dissolve requires manifold interior edges')
    from .normal import prim_normals
    normals = prim_normals(geo).astype(float)
    lengths = np.linalg.norm(normals, axis=1)
    pv, vp, nxt = topo.prim_of_vertex(geo), geo.topology.vertex_point, topo.next_vertex(geo)
    parent = list(range(geo.nprims))
    def root(p):
        while parent[p] != p:
            parent[p] = parent[parent[p]]
            p = parent[p]
        return p
    for eid in np.flatnonzero(chosen):
        u, v = np.flatnonzero(e.of_vertex == eid)
        p, q = pv[u], pv[v]
        if p == q or vp[u] != vp[nxt[v]] or vp[v] != vp[nxt[u]]:
            raise ValueError('dissolve requires distinct consistently wound faces')
        if min(lengths[p], lengths[q]) < 1e-12 or np.dot(normals[p], normals[q])/(lengths[p]*lengths[q]) < np.cos(np.radians(angle))-1e-9:
            raise ValueError('dissolve face angle exceeds limit or face is degenerate')
        if preserve_seams:
            for name, a in geo.prim.items():
                if not name.startswith('group_') and not np.array_equal(a[p], a[q]):
                    raise ValueError(f'dissolve crosses primitive attribute seam: {name}')
            for name, a in geo.vertex.items():
                if name.startswith(edges.PREFIX) or name == edges.CREASE:
                    continue
                if not np.allclose(a[u], a[nxt[v]], atol=1e-6, rtol=0) or not np.allclose(a[v], a[nxt[u]], atol=1e-6, rtol=0):
                    raise ValueError(f'dissolve crosses corner attribute seam: {name}')
        parent[root(q)] = root(p)
    regions = {}
    for p in range(geo.nprims):
        regions.setdefault(root(p), []).append(p)
    faces, sources, members = [], [], []
    for region in sorted(regions.values(), key=min):
        vs = np.flatnonzero(np.isin(pv, region) & ~chosen[e.of_vertex])
        if len(region) == 1:
            face = vs.tolist()
        else:
            by_start = {}
            for v in vs:
                if int(vp[v]) in by_start:
                    raise ValueError('dissolve region has a hole, repeated corner or branching boundary')
                by_start[int(vp[v])] = int(v)
            face = []
            cur = int(vs[0]) if len(vs) else -1
            while cur >= 0 and cur not in face:
                face.append(cur)
                cur = by_start.get(int(vp[nxt[cur]]), -1)
            if len(face) < 3 or len(face) != len(vs) or cur != face[0]:
                raise ValueError('dissolve region must have one simple closed boundary')
        faces.append(face)
        sources.append(min(region))
        members.append(region)
    out = _rebuild(geo, faces, sources)
    for name, a in geo.prim.items():
        if name.startswith('group_'):
            out.prim[name] = np.array([np.max(a[r], axis=0) for r in members], dtype=a.dtype)
    return out
