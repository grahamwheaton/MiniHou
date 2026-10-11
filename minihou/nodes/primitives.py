"""Source nodes: box, sphere, circle; plus fuse (point welding)."""
import numpy as np

from ..geometry import Geometry, Topology
from ..sdk import Float, Int, Vec3, node
from ._util import euler_xyz_matrix


def fuse_points(geo: Geometry, distance: float) -> Geometry:
    """Weld points closer than ``distance`` (grid-snapped: points in the same cell of size ``distance`` merge).
    The first point of each cluster is kept with its attributes; primitives are re-pointed. Degenerate repeated
    vertices inside a primitive are left as they are."""
    if geo.npoints == 0:
        return geo
    d = max(float(distance), 1e-12)
    key = np.floor(geo.P.astype(np.float64) / d + 0.5).astype(np.int64)
    _, first, inverse = np.unique(key, axis=0, return_index=True, return_inverse=True)
    inverse = inverse.reshape(-1)
    order = np.argsort(first)                       # keep original point order of the survivors
    rank = np.empty_like(order)
    rank[order] = np.arange(order.shape[0])
    remap = rank[inverse]
    keep_idx = first[order]
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


@node("fuse", inputs=["geo"], category="modify", params=[Float("distance", 0.001, min=0.0, soft_max=1.0)])
def fuse(geo, distance):
    """Weld points that lie within ``distance`` of each other (snapped to a grid of that size)."""
    return fuse_points(geo, distance)


def _face_grid(n):
    """Quads of an n x n cell grid over (n+1)^2 points, row-major, CCW seen from +Z."""
    ii, jj = np.meshgrid(np.arange(n), np.arange(n))
    b = (jj * (n + 1) + ii).ravel()
    return np.stack([b, b + 1, b + n + 2, b + n + 1], axis=1)


@node("box", category="source", params=[
    Vec3("size", (1.0, 1.0, 1.0), min=0.0),
    Vec3("center", (0.0, 0.0, 0.0)),
    Int("divisions", 1, min=1, max=1000, soft_max=50),
])
def box(size, center, divisions):
    """Closed box of quads (outward-facing), each face split into ``divisions`` x ``divisions``."""
    n = divisions
    u = np.linspace(-0.5, 0.5, n + 1)
    uu, vv = np.meshgrid(u, u)
    uu, vv = uu.ravel(), vv.ravel()
    ones = np.full_like(uu, 0.5)
    # (P(u, v) for each face) chosen so (u, v) increasing is CCW seen from outside
    faces = [np.stack([uu, vv, ones], 1), np.stack([vv, uu, -ones], 1),      # +Z, -Z
             np.stack([ones, uu, vv], 1), np.stack([-ones, vv, uu], 1),      # +X, -X
             np.stack([vv, ones, uu], 1), np.stack([uu, -ones, vv], 1)]      # +Y, -Y
    quads = _face_grid(n)
    per = (n + 1) ** 2
    P = np.concatenate(faces) * np.asarray(size, np.float64) + np.asarray(center, np.float64)
    vp = np.concatenate([quads + k * per for k in range(6)]).ravel()
    g = Geometry.from_arrays(P.astype(np.float32), vp, np.arange(6 * n * n + 1) * 4)
    return fuse_points(g, 1e-6 * max(1.0, float(max(size))) if max(size) > 0 else 1e-9)


@node("sphere", category="source", params=[
    Float("radius", 1.0, min=0.0),
    Vec3("center", (0.0, 0.0, 0.0)),
    Int("rows", 16, min=2, max=4096, soft_max=200, doc="latitude bands"),
    Int("columns", 32, min=3, max=4096, soft_max=400, doc="longitude segments"),
])
def sphere(radius, center, rows, columns):
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
    return Geometry.from_arrays(P.astype(np.float32), vp, offs)


@node("circle", category="source", params=[
    Float("radius", 1.0, min=0.0),
    Vec3("center", (0.0, 0.0, 0.0)),
    Vec3("rotate", (0.0, 0.0, 0.0), doc="Euler XYZ degrees; the circle lies in XY facing +Z before rotation"),
    Int("divisions", 32, min=3, max=100000),
])
def circle(radius, center, rotate, divisions):
    """A single filled n-gon."""
    t = np.linspace(0, 2 * np.pi, divisions, endpoint=False)
    P = np.stack([np.cos(t), np.sin(t), np.zeros_like(t)], 1) * radius
    P = P @ euler_xyz_matrix(rotate).T + np.asarray(center)
    return Geometry.from_arrays(P.astype(np.float32), np.arange(divisions), [0, divisions])
