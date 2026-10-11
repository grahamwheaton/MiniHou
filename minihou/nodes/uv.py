"""UV projection (Houdini's UV Project / UV Texture): planar, box (triplanar), cylindrical and spherical.

UVs are written per vertex (face corner), so seams can be split cleanly: a face that straddles the wrap-around of a
cylindrical or spherical projection gets continuous coordinates instead of a stretch across the whole texture. In
Blender the result is the UV map 'UVMap'.
"""
import numpy as np

from .. import topo
from ..sdk import Enum, Float, Str, Vec3, node
from .normal import prim_normals
from ._util import euler_xyz_matrix, normalize_rows


def _local(P, center, rotate, scale):
    R = euler_xyz_matrix(rotate)
    s = np.where(np.abs(np.asarray(scale, np.float64)) < 1e-12, 1.0, np.asarray(scale, np.float64))
    return ((P - np.asarray(center, np.float64)) @ R) / s          # into the projection's frame (R^T applied)


def _fix_wrap(u, pv, nprims, on_axis):
    """Faces crossing the u = 0/1 seam: shift their small-u corners by +1 so the face is continuous. Corners on the
    projection axis (a sphere's poles) have no direction: they take the mean u of the face's other corners."""
    off = ~on_axis
    lo = np.full(nprims, np.inf)
    hi = np.full(nprims, -np.inf)
    np.minimum.at(lo, pv[off], u[off])
    np.maximum.at(hi, pv[off], u[off])
    wraps = (hi - lo) > 0.5
    u = np.where(wraps[pv] & (u < 0.5), u + 1.0, u)
    if on_axis.any():
        cnt = np.bincount(pv[off], minlength=nprims)
        mean = np.bincount(pv[off], weights=u[off], minlength=nprims) / np.maximum(cnt, 1)
        u = np.where(on_axis, mean[pv], u)
    return u


@node("uv_project", inputs=["geo"], category="attribute", params=[
    Enum("method", ["box", "planar", "cylindrical", "spherical"],
         doc="box: each face projected along its dominant axis (triplanar); planar: along the frame's Z; "
             "cylindrical / spherical: around the frame's Z axis"),
    Vec3("center", (0.0, 0.0, 0.0)),
    Vec3("rotate", (0.0, 0.0, 0.0), doc="Orientation of the projection frame, Euler XYZ degrees"),
    Vec3("scale", (1.0, 1.0, 1.0), doc="Size of one UV tile in the frame (box/planar); height (cylindrical)"),
    Float("rotate_uv", 0.0, soft_min=-180.0, soft_max=180.0, doc="Rotate the UVs about (0.5, 0.5), degrees"),
    Str("group", "", doc="Only these primitives (others keep their uv)"),
])
def uv_project(geo, method, center, rotate, scale, rotate_uv, group):
    """Project texture coordinates onto the geometry (written as vertex uv, Blender's UV map)."""
    from .. import groups
    t = geo.topology
    vp = t.vertex_point.astype(np.int64)
    pv = topo.prim_of_vertex(geo)
    L = _local(geo.P.astype(np.float64)[vp], center, rotate, scale)
    if method == "planar":
        uv = L[:, :2] + 0.5
    elif method == "box":
        R = euler_xyz_matrix(rotate)
        n = normalize_rows(prim_normals(geo)).astype(np.float64) @ R      # face normals in the frame
        axis = np.argmax(np.abs(n), axis=1)[pv]
        sign = np.sign(n[pv, axis])
        sign = np.where(sign == 0, 1.0, sign)
        # u/v axes per dominant axis, mirrored so textures read correctly from each side
        u = np.choose(axis, [L[:, 1] * sign, -L[:, 0] * sign, L[:, 0] * sign])
        v = np.choose(axis, [L[:, 2], L[:, 2], L[:, 1]])
        uv = np.stack([u, v], 1) + 0.5
    elif method == "cylindrical":
        u = (np.arctan2(L[:, 1], L[:, 0]) / (2 * np.pi)) % 1.0
        axis = np.hypot(L[:, 0], L[:, 1]) < 1e-9 * max(1.0, float(np.abs(L).max(initial=0)))
        uv = np.stack([_fix_wrap(u, pv, t.nprims, axis), L[:, 2] + 0.5], 1)
    else:
        r = np.maximum(np.linalg.norm(L, axis=1), 1e-12)
        u = (np.arctan2(L[:, 1], L[:, 0]) / (2 * np.pi)) % 1.0
        v = np.arccos(np.clip(L[:, 2] / r, -1, 1)) / np.pi
        axis = np.hypot(L[:, 0], L[:, 1]) < 1e-6 * r
        uv = np.stack([_fix_wrap(u, pv, t.nprims, axis), 1.0 - v], 1)
    if rotate_uv:
        a = np.radians(rotate_uv)
        c, s = np.cos(a), np.sin(a)
        d = uv - 0.5
        uv = np.stack([d[:, 0] * c - d[:, 1] * s, d[:, 0] * s + d[:, 1] * c], 1) + 0.5
    uv = uv.astype(np.float32)
    sel = groups.mask(geo, "prim", group)
    out = geo.copy()
    if sel is not None:
        old = geo.vertex.get("uv")
        if old is None and "uv" in geo.point and geo.point["uv"].shape[1:] == (2,):
            old = geo.point["uv"][vp]
        if old is None or old.shape != uv.shape:
            old = np.zeros_like(uv)
        uv = np.where(sel[pv][:, None], uv, old).astype(np.float32)
    for other in ("point", "prim"):
        if "uv" in out.table(other):
            del out.table(other)["uv"]
    out.vertex["uv"] = uv
    return out
