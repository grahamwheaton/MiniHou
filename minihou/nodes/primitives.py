"""Source nodes: box, sphere, circle, tube, torus; plus fuse (point welding)."""
import numpy as np

from ..geometry import Geometry, Topology
from ..outgroups import OutGroup, OutGroups
from ..sdk import Bool, Enum, Float, Guide, Handle, Input, Int, Vec3, node
from . import _build as B
from ._util import euler_xyz_matrix


@node("point", category="source", params=[
    Vec3("center", (0.0, 0.0, 0.0), label="Position", doc="Position of the single point in model coordinates"),
], handles=[Handle("center", axis=axis, at="@center", comp=i, tool="move") for i, axis in enumerate("xyz")])
def point_node(center=(0.0, 0.0, 0.0)):
    """Create one editable point, with no edges or faces. Useful as a copy target or graph seed."""
    return Geometry.from_arrays(np.asarray([center], dtype=np.float32))


def fuse_points(geo: Geometry, distance: float, counts_out: list = None) -> Geometry:
    """Weld points closer than ``distance`` (grid-snapped: points in the same cell of size ``distance`` merge).
    The first point of each cluster is kept with its attributes; primitives are re-pointed. Degenerate repeated
    vertices inside a primitive are left as they are. ``counts_out`` (a list) receives, per surviving point, how many
    points it stands for."""
    if geo.npoints == 0:
        if counts_out is not None:
            counts_out.append(np.zeros(0, np.int64))
        return geo
    d = max(float(distance), 1e-12)
    key = np.floor(geo.P.astype(np.float64) / d + 0.5).astype(np.int64)
    _, first, inverse, cluster = np.unique(key, axis=0, return_index=True, return_inverse=True, return_counts=True)
    inverse = inverse.reshape(-1)
    order = np.argsort(first)                       # keep original point order of the survivors
    rank = np.empty_like(order)
    rank[order] = np.arange(order.shape[0])
    remap = rank[inverse]
    keep_idx = first[order]
    if counts_out is not None:
        counts_out.append(cluster[order])
    out = Geometry(keep_idx.shape[0], Topology(remap[geo.topology.vertex_point].astype(np.int32),
                                                geo.topology.prim_offsets, geo.topology.closed))
    for name, a in geo.point.items():
        out.point[name] = a[keep_idx]
    for name, a in geo.vertex.items():
        out.vertex[name] = a
    for name, a in geo.prim.items():
        out.prim[name] = a
    out.detail = dict(geo.detail)
    return out


FUSE_OUT = OutGroups(
    OutGroup("fused", "Fused Points", "fusedpoints", "point", doc="Points that stand for two or more welded points"),
    OutGroup("unique", "Unique Points", "uniquepoints", "point", doc="Points that were not welded to anything"),
)


@node("fuse", inputs=["geo", Input("targets", optional=True, doc="Snap targets: points of the first input within the distance move onto the nearest point here")],
      category="modify", params=[Float("distance", 0.001, min=0.0, soft_max=1.0), *FUSE_OUT.params()], version=2)
def fuse(geo, targets=None, *, distance, **og):
    """Weld points that lie within ``distance`` of each other (snapped to a grid of that size). With a second input
    the first input's points are only snapped onto the nearest point of the second (within ``distance``): the
    second input is not modified, and the first input's points are not welded to each other."""
    if targets is not None:
        from .. import spatial
        out = geo.copy()
        if geo.npoints and targets.npoints:
            idx, dist = spatial.nearest(targets.P, geo.P)
            hit = dist <= max(float(distance), 1e-12)
            P = geo.P.copy()
            P[hit] = targets.P[idx[hit]]
            out.point["P"] = P
        else:
            hit = np.zeros(geo.npoints, bool)
        out = FUSE_OUT.write(out, og, "fused", lambda: hit)
        return FUSE_OUT.write(out, og, "unique", lambda: ~hit)
    counts = []
    out = fuse_points(geo, distance, counts)
    c = counts[0] if counts else np.zeros(0, np.int64)
    out = FUSE_OUT.write(out, og, "fused", lambda: c > 1)
    return FUSE_OUT.write(out, og, "unique", lambda: c == 1)


def _face_grid(n):
    """Quads of an n x n cell grid over (n+1)^2 points, row-major, CCW seen from +Z."""
    ii, jj = np.meshgrid(np.arange(n), np.arange(n))
    b = (jj * (n + 1) + ii).ravel()
    return np.stack([b, b + 1, b + n + 2, b + n + 1], axis=1)


@node("box", inputs=[Input("geo", optional=True, doc="Optional geometry to bound")], category="source", params=[
    Enum("primitive_type", ["polygons", "points"], label="Primitive Type"),
    Enum("connectivity", ["quadrilaterals", "triangles"], label="Connectivity"),
    Bool("consolidate", True, label="Consolidate Corner Points"),
    Vec3("size", (1.0, 1.0, 1.0), min=0.0),
    Vec3("center", (0.0, 0.0, 0.0)),
    Vec3("rotate", (0.0, 0.0, 0.0)),
    Float("uniform_scale", 1.0, min=0.0),
    Int("divisions", 1, min=1, max=1000, soft_max=50, proxy="res2", doc="Legacy uniform cell divisions; used when axis divisions are zero"),
    Vec3("axis_divisions", (0,0,0), min=0, max=1000, label="Axis Divisions", doc="Cells per axis; zero uses Divisions. Integer values required."),
    Bool("add_vertex_normals", False),
])
def box_node(geo=None, size=(1,1,1), center=(0,0,0), divisions=1, rotate=(0,0,0), uniform_scale=1,
        axis_divisions=(0,0,0), consolidate=True, connectivity="quadrilaterals", primitive_type="polygons", add_vertex_normals=False):
    """Polygon box or surface points; optional input fits an axis-aligned bounding box before rotation."""
    if any(v != int(v) for v in axis_divisions):
        raise ValueError("Axis divisions must be integers")
    dims=[int(v) or divisions for v in axis_divisions]
    if sum(dims[i]*dims[j] for i,j in [(0,1),(1,2),(0,2)]) > 1000000:
        raise ValueError("Box exceeds one million face cells")
    size=np.asarray(size,np.float64);center=np.asarray(center,np.float64)
    if geo is not None and geo.npoints:
        lo,hi=geo.P.min(0),geo.P.max(0)
        size=size*(hi-lo);center=center+(hi+lo)/2
    points=[];polys=[];offset=0
    for u,v,w,sign in [(0,1,2,1),(1,0,2,-1),(1,2,0,1),(2,1,0,-1),(2,0,1,1),(0,2,1,-1)]:
        nu,nv=dims[u],dims[v]
        uu,vv=np.meshgrid(np.linspace(-.5,.5,nu+1),np.linspace(-.5,.5,nv+1))
        ps=np.zeros(((nu+1)*(nv+1),3));ps[:,u]=uu.ravel();ps[:,v]=vv.ravel();ps[:,w]=sign*.5
        ii,jj=np.meshgrid(np.arange(nu),np.arange(nv));q=(jj*(nu+1)+ii).ravel()+offset
        quads=np.stack([q,q+1,q+nu+2,q+nu+1],axis=1)
        points.append(ps);polys.extend(quads);offset+=len(ps)
    P=np.concatenate(points)*size*uniform_scale
    P=P@euler_xyz_matrix(rotate).T+center
    polygons=np.asarray(polys)
    if connectivity=="triangles":polygons=np.concatenate([polygons[:,[0,1,2]],polygons[:,[0,2,3]]])
    g=Geometry.from_arrays(P.astype(np.float32),polygons.ravel(),np.arange(len(polygons)+1)*polygons.shape[1])
    if consolidate:g=fuse_points(g,1e-6*max(1.,float(np.max(size)*uniform_scale)))
    if primitive_type=="points":return Geometry.points_only(g.P)
    if add_vertex_normals:
        from .normal import normal
        g=normal(g,"vertex",60.)
    return g


@node("sphere", category="source", params=[
    Float("radius", 1.0, min=0.0),
    Vec3("center", (0.0, 0.0, 0.0)),
    Vec3("rotate", (0,0,0)), Vec3("scale", (1,1,1)), Float("uniform_scale", 1., min=0),
    Int("rows", 16, min=2, max=4096, soft_max=200, doc="latitude bands", proxy="res2"),
    Int("columns", 32, min=3, max=4096, soft_max=400, doc="longitude segments", proxy="res2"),
])
def sphere(radius, center, rows, columns, rotate=(0,0,0), scale=(1,1,1), uniform_scale=1):
    """UV sphere: triangle fans at the poles, quads between; outward-facing, closed."""
    lat = np.linspace(0, np.pi, rows + 1)[1:-1]                 # ring latitudes (from +Z pole down)
    lon = np.linspace(0, 2 * np.pi, columns, endpoint=False)
    sl, cl = np.sin(lat)[:, None], np.cos(lat)[:, None]
    ring = np.stack([sl * np.cos(lon)[None, :], sl * np.sin(lon)[None, :], np.broadcast_to(cl, (rows - 1, columns))], -1)
    P = np.concatenate([[[0, 0, 1]], ring.reshape(-1, 3), [[0, 0, -1]]]) * radius + np.asarray(center)
    top, bottom = 0, 1 + (rows - 1) * columns
    c = np.arange(columns)
    cn = (c + 1) % columns
    polys_tri_top = np.stack([np.full(columns, top), 1 + c, 1 + cn], 1)
    r0 = 1 + (rows - 2) * columns
    polys_tri_bot = np.stack([np.full(columns, bottom), r0 + cn, r0 + c], 1)
    quads = []
    for r in range(rows - 2):
        a, b = 1 + r * columns, 1 + (r + 1) * columns
        quads.append(np.stack([a + c, b + c, b + cn, a + cn], 1))
    quads = np.concatenate(quads) if quads else np.zeros((0, 4), int)
    vp = np.concatenate([polys_tri_top.ravel(), quads.ravel(), polys_tri_bot.ravel()])
    offs = np.concatenate([np.arange(columns + 1) * 3,
                           columns * 3 + np.arange(1, quads.shape[0] + 1) * 4,
                           columns * 3 + quads.shape[0] * 4 + np.arange(1, columns + 1) * 3])
    return source_transform(Geometry.from_arrays(P.astype(np.float32), vp, offs), center, rotate, scale, uniform_scale)


@node("circle", category="source", params=[
    Float("radius", 1.0, min=0.0),
    Vec3("center", (0.0, 0.0, 0.0)),
    Vec3("rotate", (0.0, 0.0, 0.0), doc="Euler XYZ degrees; the circle lies in XY facing +Z before rotation"),
    Int("divisions", 32, min=3, max=100000, proxy="count"),
], handles=[Handle("radius", axis="x", at="@center", reach=True, min=0.0)])

def circle(radius, center, rotate, divisions):
    """A single filled n-gon."""
    t = np.linspace(0, 2 * np.pi, divisions, endpoint=False)
    P = np.stack([np.cos(t), np.sin(t), np.zeros_like(t)], 1) * radius
    P = P @ euler_xyz_matrix(rotate).T + np.asarray(center)
    return Geometry.from_arrays(P.astype(np.float32), np.arange(divisions), [0, divisions])


def _ring_quads(rows, columns, wrap_rows):
    """Quads over a (rows+1 or rows) x columns lattice of points (row-major), wrapping around the columns and, for a
    torus, the rows; plus their vertex uvs (u around the columns, v along the rows, seams get u = 1 / v = 1)."""
    nr = rows if wrap_rows else rows + 1
    r, c = np.meshgrid(np.arange(rows), np.arange(columns), indexing="ij")
    r, c = r.ravel(), c.ravel()
    r1, c1 = (r + 1) % nr, (c + 1) % columns
    quads = np.stack([r * columns + c, r * columns + c1, r1 * columns + c1, r1 * columns + c], 1)
    u = np.stack([c, c + 1, c + 1, c], 1) / columns
    v = np.stack([r, r, r + 1, r + 1], 1) / rows
    return quads, np.stack([u, v], -1).reshape(-1, 2)


@node("tube", category="source", params=[
    Float("radius_bottom", 1.0, min=0.0, soft_max=10.0),
    Float("radius_top", 1.0, min=0.0, soft_max=10.0, doc="0 makes a cone"),
    Float("height", 2.0, min=0.0, soft_max=20.0),
    Vec3("center", (0.0, 0.0, 0.0)),
    Vec3("rotate", (0,0,0)), Vec3("scale", (1,1,1)), Float("uniform_scale", 1., min=0),
    Int("rows", 1, min=1, max=4096, soft_max=100, doc="segments along the height", proxy="res2"),
    Int("columns", 24, min=3, max=4096, soft_max=200, doc="segments around", proxy="res2"),
    Bool("caps", True, doc="Close the ends with n-gons (a zero radius ends in a point instead)"),
], handles=[Handle("radius_bottom", axis="x", at="bottom", reach=True, min=0.0),
            Handle("radius_top", axis="x", at="top", reach=True, min=0.0),
            Handle("height", axis="z", at="top", scale=2.0, min=0.0)])         # the tube grows both ways from its centre
def tube(radius_bottom, radius_top, height, center, rows, columns, caps, rotate=(0,0,0), scale=(1,1,1), uniform_scale=1):
    """Cylinder or cone along +Z (Houdini's Tube), outward-facing, with vertex uvs; closed when ``caps`` is on."""
    t = np.linspace(0.0, 1.0, rows + 1)
    rad = radius_bottom + (radius_top - radius_bottom) * t
    ang = np.linspace(0, 2 * np.pi, columns, endpoint=False)
    P = np.stack([rad[:, None] * np.cos(ang), rad[:, None] * np.sin(ang),
                  np.broadcast_to(((t - 0.5) * height)[:, None], (rows + 1, columns))], -1).reshape(-1, 3)
    quads, uv = _ring_quads(rows, columns, False)
    polys, uvs = list(quads), list(uv.reshape(-1, 4, 2))
    if caps:
        disc = np.stack([np.cos(ang), np.sin(ang)], 1) * 0.5 + 0.5
        polys.append(np.arange(columns)[::-1])                              # bottom faces -Z
        uvs.append(disc[::-1])
        polys.append(rows * columns + np.arange(columns))                   # top faces +Z
        uvs.append(disc)
    sizes = [len(p) for p in polys]
    g = Geometry.from_arrays((P + np.asarray(center)).astype(np.float32), np.concatenate(polys),
                             B.offsets_from_sizes(sizes))
    g.vertex["uv"] = np.concatenate(uvs).astype(np.float32)
    if min(radius_bottom, radius_top) <= 0.0:                              # cone: weld the tip ring into one point
        g = B.drop_degenerate(fuse_points(g, 1e-7 * max(1.0, radius_bottom, radius_top, height)))
    return source_transform(g, center, rotate, scale, uniform_scale)


@node("torus", category="source", params=[
    Float("radius", 1.0, min=0.0, soft_max=10.0, doc="From the centre to the middle of the tube"),
    Float("thickness", 0.25, min=0.0, soft_max=5.0, doc="Radius of the tube"),
    Vec3("center", (0.0, 0.0, 0.0)),
    Vec3("rotate", (0,0,0)), Vec3("scale", (1,1,1)), Float("uniform_scale", 1., min=0),
    Int("rows", 12, min=3, max=4096, soft_max=200, doc="segments around the tube", proxy="res2"),
    Int("columns", 32, min=3, max=4096, soft_max=400, doc="segments around the ring", proxy="res2"),
], handles=[Handle("radius", axis="x", at="@center", reach=True, min=0.0)])

def torus(radius, thickness, center, rows, columns, rotate=(0,0,0), scale=(1,1,1), uniform_scale=1):
    """Torus in the XY plane (Houdini's Torus), outward-facing quads with vertex uvs; closed."""
    phi = np.linspace(0, 2 * np.pi, rows, endpoint=False)[:, None]        # around the tube
    th = np.linspace(0, 2 * np.pi, columns, endpoint=False)[None, :]       # around the ring
    w = (radius + thickness * np.cos(phi)) * np.ones_like(th)
    P = np.stack([w * np.cos(th), w * np.sin(th), np.broadcast_to(thickness * np.sin(phi), w.shape)], -1)
    quads, uv = _ring_quads(rows, columns, True)
    g = Geometry.from_arrays((P.reshape(-1, 3) + np.asarray(center)).astype(np.float32), quads.ravel(),
                             np.arange(len(quads) + 1) * 4)
    g.vertex["uv"] = uv.astype(np.float32)
    return source_transform(g, center, rotate, scale, uniform_scale)


def source_transform(g, center, rotate, scale, uniform_scale):
    out=g.copy();c=np.asarray(center)
    out.point["P"]=(((g.P-c)*np.asarray(scale)*uniform_scale)@euler_xyz_matrix(rotate).T+c).astype(np.float32)
    return out


def box(size=(1,1,1), center=(0,0,0), divisions=1, **kwargs):
    """Keep the internal primitive helper's original positional calling convention."""
    return box_node(size=size, center=center, divisions=divisions, **kwargs)
