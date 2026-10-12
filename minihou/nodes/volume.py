"""Volumes: voxel remeshing, points to surface, and SDF booleans (Houdini's VDB from Polygons / from Particles,
VDB Smooth, VDB Combine and Convert VDB, collapsed into mesh-in, mesh-out nodes).

The volumes are dense signed distance grids (:mod:`minihou.sdf`), meshed back with surface nets into watertight quad
meshes. Cost grows with (size / voxel_size)^3: start coarse.
"""
import fnmatch

import numpy as np

from .. import sdf, spatial
from ..sdk import Bool, Enum, Float, Int, Str, node
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
    Float("voxel_size", 0.05, min=1e-4, soft_max=1.0, doc="Detail of the new mesh; cost grows with 1/size^3"),
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
    Float("voxel_size", 0.05, min=1e-4, soft_max=1.0),
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
    Float("voxel_size", 0.05, min=1e-4, soft_max=1.0),
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
