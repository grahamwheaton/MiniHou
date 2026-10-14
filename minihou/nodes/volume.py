"""Volumes: voxel remeshing, points to surface, and SDF booleans (Houdini's VDB from Polygons / from Particles,
VDB Smooth, VDB Combine and Convert VDB, collapsed into mesh-in, mesh-out nodes).

The volumes are dense signed distance grids (:mod:`minihou.sdf`), meshed back with surface nets into watertight quad
meshes. Cost grows with (size / voxel_size)^3: start coarse.
"""
import fnmatch

import numpy as np

from .. import sdf, spatial
from ..sdk import Bool, Enum, Float, Guide, Handle, Int, Str, Vec3, node
from . import _build as B
from .model import triangulate


def _tris(geo, what):
    B.require_polygons(geo, what)
    if geo.nprims == 0:
        return np.zeros((0, 3, 3))
    tv, _, keep = triangulate(geo, np.ones(geo.nprims, bool))
    t = geo.topology
    offs = t.prim_offsets.astype(np.int64)
    whole = np.flatnonzero(keep & (t.prim_sizes() == 3))
    tv = np.concatenate([tv, np.stack([offs[whole], offs[whole] + 1, offs[whole] + 2], 1)])
    return geo.P.astype(np.float64)[t.vertex_point[tv]]


def _mesh(vol, source=None, transfer=""):
    pts, quads = sdf.surface_nets(vol)
    out = B.make(pts.astype(np.float32), quads.reshape(-1), np.arange(0, quads.size + 1, 4))
    if source is not None and transfer and source.npoints and len(pts):
        pats = transfer.replace(",", " ").split()
        names = [n for n in source.point.names() if n != "P" and any(fnmatch.fnmatchcase(n, p) for p in pats)]
        if names:
            idx, _ = spatial.nearest(source.P, pts)
            for n in names:
                out.point[n] = source.point[n][idx]
    return out


def _check_h(voxel_size):
    if voxel_size <= 0:
        raise ValueError("voxel_size must be positive")


@node("remesh", inputs=["geo"], category="volume", params=[
    Float("voxel_size", 0.05, min=1e-4, soft_max=1.0, doc="Detail of the new mesh; cost grows with 1/size^3", proxy="step3"),
    Float("offset", 0.0, soft_min=-1.0, soft_max=1.0, doc="Grow (+) or shrink (-) the surface by this distance"),
    Int("smooth", 0, min=0, soft_max=20, doc="Smoothing passes on the distance field (rounds sharp detail)"),
    Str("transfer", "", doc="Point attributes to carry over from the nearest original point, e.g. 'Cd uv'"),
])
def remesh(geo, voxel_size, offset, smooth, transfer):
    """Voxel remesh a closed mesh into an even, watertight quad mesh (VDB from Polygons -> smooth -> convert):
    merges overlapping parts, closes small gaps, optional offset and smoothing. Needs closed meshes."""
    _check_h(voxel_size)
    band = max(3.0, abs(offset) / voxel_size + 2.0, smooth + 2.0)
    vol = sdf.from_mesh(_tris(geo, "remesh"), voxel_size, band_voxels=band, pad_voxels=int(np.ceil(band)) + 1)
    if offset:
        vol = sdf.Volume(vol.origin, vol.h, vol.values - np.float32(offset))
    if smooth:
        vol = sdf.blur(vol, smooth)
    return _mesh(vol, geo, transfer)


@node("points_to_surface", inputs=["points"], category="volume", params=[
    Float("radius", 0.2, min=1e-4, soft_max=2.0, doc="Sphere radius per point (times pscale if present)"),
    Float("blend", 1.0, min=0.0, soft_max=4.0, doc="How much neighbouring points melt together (metaballs)"),
    Float("voxel_size", 0.05, min=1e-4, soft_max=1.0, proxy="step3"),
    Int("smooth", 0, min=0, soft_max=20),
    Str("transfer", "Cd", doc="Point attributes to carry from the nearest point onto the surface"),
])
def points_to_surface(points, radius, blend, voxel_size, smooth, transfer):
    """A smooth surface around points (VDB from Particles / metaballs): blobs, liquids, organic shapes."""
    _check_h(voxel_size)
    if points.npoints == 0:
        return B.make(np.zeros((0, 3), np.float32), np.zeros(0, np.int64), [0])
    r = np.full(points.npoints, radius, np.float64)
    if "pscale" in points.point:
        r = r * points.point["pscale"].reshape(points.npoints, -1)[:, 0].astype(np.float64)
    vol = sdf.from_points(points.P, r, voxel_size, blend)
    if smooth:
        vol = sdf.blur(vol, smooth)
    return _mesh(vol, points, transfer)


@node("sdf_boolean", inputs=["a", "b"], category="volume", params=[
    Enum("operation", ["union", "subtract", "intersect"], doc="subtract: a minus b"),
    Float("voxel_size", 0.05, min=1e-4, soft_max=1.0, proxy="step3"),
    Float("smooth", 0.0, min=0.0, soft_max=1.0, doc="Fillet radius where the shapes meet (0 = sharp)"),
    Bool("transfer_attribs", True, doc="Carry point attributes from the nearest input point (Cd, uv...)"),
])
def sdf_boolean(a, b, operation, voxel_size, smooth, transfer_attribs):
    """Boolean of two closed meshes through distance fields: robust (no fragile intersection math), watertight, with
    optional smooth fillets; edges are as sharp as the voxel size allows."""
    _check_h(voxel_size)
    ta, tb = _tris(a, "sdf_boolean"), _tris(b, "sdf_boolean")
    allp = np.concatenate([ta.reshape(-1, 3), tb.reshape(-1, 3)])
    if not len(allp):
        return B.make(np.zeros((0, 3), np.float32), np.zeros(0, np.int64), [0])
    band = max(3.0, smooth / voxel_size + 2.0)
    for tris in (ta, tb):                                    # fail at once, not after the first mesh has been done
        if sdf.mesh_work(tris, voxel_size, band) > sdf.MAX_MESH_WORK:
            raise ValueError("this voxel size (with this smooth) needs too much work for the mesh: use a larger voxel size "
                             "or a smaller smooth")
    lo, dims = sdf.grid_for(allp.min(0), allp.max(0), voxel_size, (band + 2) * voxel_size)
    va = sdf.from_mesh(ta, voxel_size, band, grid=(lo, dims)).values
    vb = sdf.from_mesh(tb, voxel_size, band, grid=(lo, dims)).values
    k = float(smooth)
    if operation == "union":
        v = sdf.smooth_min(va, vb, k)
    elif operation == "intersect":
        v = -sdf.smooth_min(-va, -vb, k)
    else:
        v = -sdf.smooth_min(-va, vb, k)
    vol = sdf.Volume(lo, voxel_size, v.astype(np.float32))
    if not transfer_attribs:
        return _mesh(vol)
    from .merge import merge
    both = merge([a, b]) if a.npoints and b.npoints else (a if a.npoints else b)
    return _mesh(vol, both, "*")


# -- analytic shapes ---------------------------------------------------------------------------------------------
SDF_SHAPES = ["sphere", "box", "cylinder", "capsule", "ellipsoid", "torus", "round_cone", "prism"]


def _length(v):
    return np.sqrt((v * v).sum(-1))


def _box_distance(p, half):
    q = np.abs(p) - half
    return _length(np.maximum(q, 0.0)) + np.minimum(q.max(-1), 0.0)


def _extrude_distance(d2, z, half_depth):
    w = np.stack([d2, np.abs(z) - half_depth], -1)
    return np.minimum(w.max(-1), 0.0) + _length(np.maximum(w, 0.0))


def _round_cone_distance(p, a, b, r1, r2):
    """Inigo Quilez's exact round cone between spheres (a, r1) and (b, r2)."""
    ba = b - a
    l2 = float(ba @ ba)
    rr = r1 - r2
    a2 = l2 - rr * rr
    il2 = 1.0 / l2
    pa = p - a
    y = pa @ ba
    z = y - l2
    w = pa * l2 - np.outer(y, ba)
    x2 = (w * w).sum(-1)
    y2, z2 = y * y * l2, z * z * l2
    k = np.sign(rr) * rr * rr * x2
    return np.where(np.sign(z) * a2 * z2 > k, np.sqrt(x2 + z2) * il2 - r2,
                    np.where(np.sign(y) * a2 * y2 < k, np.sqrt(x2 + y2) * il2 - r1, (np.sqrt(x2 * a2 * il2) + y * rr) * il2 - r1))


def _prism_distance(p, sides, radius, depth):
    an = np.pi / sides
    bn = np.mod(np.arctan2(p[..., 0], p[..., 1]), 2.0 * an) - an
    ln = np.hypot(p[..., 0], p[..., 1])
    q = np.stack([ln * np.cos(bn), ln * np.abs(np.sin(bn))], -1) - radius * np.array([np.cos(an), np.sin(an)])
    q[..., 1] += np.clip(-q[..., 1], 0.0, radius * np.sin(an))
    d2 = _length(q) * np.sign(q[..., 0])
    return _extrude_distance(d2, p[..., 2], depth / 2.0)


def shape_distance(shape, p, radius, size, depth, minor_radius, radius2, sides, bevel):
    """Signed distance (negative inside) of a shape centred on the origin, axis +Z, at points p (..., 3)."""
    if shape == "sphere":
        return _length(p) - radius
    if shape == "box":
        half = np.maximum(np.asarray(size, np.float64) / 2.0 - bevel, 1e-9)
        return _box_distance(p, half) - bevel
    if shape == "cylinder":
        r, h = max(radius - bevel, 1e-9), max(depth / 2.0 - bevel, 1e-9)
        return _extrude_distance(np.hypot(p[..., 0], p[..., 1]) - r, p[..., 2], h) - bevel
    if shape == "capsule":
        z = np.clip(p[..., 2], -depth / 2.0, depth / 2.0)
        return _length(np.stack([p[..., 0], p[..., 1], p[..., 2] - z], -1)) - radius
    if shape == "ellipsoid":
        r = np.maximum(np.asarray(size, np.float64) / 2.0, 1e-9)
        k0 = _length(p / r)
        k1 = _length(p / (r * r))
        return np.where(k1 > 1e-12, k0 * (k0 - 1.0) / np.maximum(k1, 1e-12), -r.min())
    if shape == "torus":
        q = np.stack([np.hypot(p[..., 0], p[..., 1]) - radius, p[..., 2]], -1)
        return _length(q) - minor_radius
    if shape == "round_cone":
        flat = p.reshape(-1, 3)
        a, b = np.array([0.0, 0.0, -depth / 2.0]), np.array([0.0, 0.0, depth / 2.0])
        return _round_cone_distance(flat, a, b, radius, radius2).reshape(p.shape[:-1])
    return _prism_distance(p, int(sides), max(radius - bevel, 1e-9), depth) - bevel


@node("sdf_shape", category="volume", params=[
    Enum("shape", SDF_SHAPES, doc="A shape from its exact distance field, meshed with surface nets: all-quad, watertight, with rounding (bevel) for free"),
    Float("radius", 0.5, min=1e-4, soft_max=5.0, doc="sphere, cylinder, capsule, prism (corner radius), torus (ring radius), round_cone (bottom radius)"),
    Vec3("size", (1.0, 1.0, 1.0), min=1e-4, doc="box (full size) and ellipsoid (full diameters)"),
    Float("depth", 1.0, min=1e-4, soft_max=10.0, doc="cylinder, prism: height; capsule: length of the straight part; round_cone: distance between the two spheres' centres"),
    Float("minor_radius", 0.15, min=1e-4, soft_max=2.0, doc="torus: tube radius"),
    Float("radius2", 0.2, min=1e-4, soft_max=5.0, doc="round_cone: top radius"),
    Int("sides", 6, min=3, max=64, doc="prism: number of sides"),
    Float("bevel", 0.0, min=0.0, soft_max=1.0, doc="box, cylinder, prism: round the edges with this radius"),
    Vec3("center", (0.0, 0.0, 0.0)),
    Vec3("rotate", (0.0, 0.0, 0.0), doc="Euler XYZ degrees"),
    Float("voxel_size", 0.04, min=1e-4, soft_max=1.0, doc="Detail of the mesh; cost grows with 1/size^3", proxy="step3"),
    Int("smooth", 0, min=0, soft_max=20, doc="Smoothing passes on the field (rounds sharp detail)"),
], handles=[Handle("radius", axis="x", at="@center", reach=True, min=1e-4, when="shape!=box&shape!=ellipsoid")],
   guides=[Guide("bounds", id="bounds", label="bounds")])
def sdf_shape(shape, radius, size, depth, minor_radius, radius2, sides, bevel, center, rotate, voxel_size, smooth):
    """An analytic shape as a watertight quad mesh (Higgsas' SDF nodes): sphere, box, cylinder, capsule, ellipsoid, torus,
    round cone and n-sided prism, with rounded edges, a position and a rotation. Feed several through sdf_boolean for smooth
    unions and subtractions."""
    from ._util import euler_xyz_matrix
    _check_h(voxel_size)
    reach = {"sphere": radius, "box": float(np.linalg.norm(size)) / 2.0, "cylinder": float(np.hypot(radius, depth / 2.0)),
             "capsule": radius + depth / 2.0, "ellipsoid": float(np.max(size)) / 2.0, "torus": radius + minor_radius,
             "round_cone": max(radius, radius2) + depth / 2.0, "prism": float(np.hypot(radius, depth / 2.0))}[shape] + 2 * voxel_size
    c = np.asarray(center, np.float64)
    R = euler_xyz_matrix(rotate)
    lo, dims = sdf.grid_for(c - reach, c + reach, voxel_size, 2 * voxel_size)
    ax = [lo[i] + np.arange(dims[i]) * voxel_size for i in range(3)]
    p = np.stack(np.meshgrid(*ax, indexing="ij"), -1) - c
    local = p @ R                                                      # world offset -> the shape's own frame (R is a rotation: R^T p)
    d = shape_distance(shape, local, radius, size, depth, minor_radius, radius2, sides, bevel)
    vol = sdf.Volume(lo, float(voxel_size), d.astype(np.float32))
    if smooth:
        vol = sdf.blur(vol, smooth)
    return _mesh(vol)
