import numpy as np

from .. import groups
from ..sdk import Bool, Float, Int, Str, Vec3, node
from ._util import euler_xyz_matrix, normalize_rows


@node("transform", inputs=["geo"], category="modify", params=[
    Vec3("translate", (0.0, 0.0, 0.0)),
    Vec3("rotate", (0.0, 0.0, 0.0), doc="Euler XYZ in degrees (X applied first)"),
    Vec3("scale", (1.0, 1.0, 1.0)),
    Float("uniform_scale", 1.0, doc="Multiplies scale"),
    Vec3("pivot", (0.0, 0.0, 0.0)),
    Str("group", "", doc="Only move these points: group names, ^exclusions, numbers/ranges or '@P.z > 0' (a primitive "
                         "group moves its points). Empty: all"),
])
def transform(geo, translate, rotate, scale, uniform_scale, pivot, group=""):
    """Scale, then rotate, then translate about ``pivot``. Also rotates ``N`` if present."""
    s = np.asarray(scale, np.float64) * uniform_scale
    R = euler_xyz_matrix(rotate)
    pv = np.asarray(pivot, np.float64)
    M = (R * s[None, :])                       # R @ diag(s): scale first, then rotate
    out = geo.copy()
    P = geo.P.astype(np.float32, copy=False)
    out.point["P"] = (P - pv.astype(np.float32)) @ M.T.astype(np.float32) + (pv + np.asarray(translate)).astype(np.float32)
    if "N" in geo.point:
        s_safe = np.where(np.abs(s) < 1e-12, 1.0, s)
        out.point["N"] = normalize_rows((geo.point["N"] / s_safe.astype(np.float32)) @ R.T.astype(np.float32))
    return groups.restrict(geo, out, "point", group)


@node("copy_transform", inputs=["geo"], category="combine", params=[
    Int("copies", 2, min=1, max=100000, soft_max=100, doc="Total number of copies, the original included"),
    Vec3("translate", (1.0, 0.0, 0.0), doc="Applied once more to each copy than to the one before"),
    Vec3("rotate", (0.0, 0.0, 0.0), doc="Euler XYZ degrees per copy"),
    Float("scale", 1.0, doc="Uniform scale per copy"),
    Vec3("pivot", (0.0, 0.0, 0.0)),
])
def copy_transform(geo, copies, translate, rotate, scale, pivot):
    """Copies with an accumulating transform (Houdini's Copy and Transform): stairs, spirals, arrays. Each copy's
    points get the int ``copynum``."""
    from .merge import merge
    R = euler_xyz_matrix(rotate) * float(scale)
    pv, t = np.asarray(pivot, np.float64), np.asarray(translate, np.float64)
    M, off = np.eye(3), np.zeros(3)
    parts = []
    P0 = geo.P.astype(np.float64)
    N0 = geo.point["N"].astype(np.float64) if "N" in geo.point else None
    for i in range(int(copies)):
        g = geo.copy()
        g.point["P"] = ((P0 - pv) @ M.T + pv + off).astype(np.float32)
        if N0 is not None:
            g.point["N"] = normalize_rows(N0 @ M.T).astype(np.float32)
        g.point["copynum"] = np.full(geo.npoints, i, np.int32)
        parts.append(g)
        M, off = R @ M, R @ off + t                          # next = T(R(previous))
    return merge(parts)


@node("peak", inputs=["geo"], category="deform", params=[
    Float("distance", 0.1, soft_min=-1.0, soft_max=1.0),
    Str("group", "", doc="Only these points"),
])
def peak(geo, distance, group):
    """Move points along their normals (Houdini's Peak): inflate, shrink or offset a surface. Uses N if present,
    else area-weighted point normals."""
    from .normal import normal
    N = geo.point["N"] if "N" in geo.point else normal(geo, "point").point["N"]
    out = geo.copy()
    out.point["P"] = (geo.P + N.astype(np.float32) * np.float32(distance)).astype(np.float32)
    return groups.restrict(geo, out, "point", group)


@node("bound", inputs=["geo"], category="generate", params=[
    Vec3("padding", (0.0, 0.0, 0.0), min=0.0, doc="Grow the box by this much on each side"),
    Bool("oriented", False, doc="Fit the box to the points' principal axes instead of the world axes"),
])
def bound(geo, padding, oriented):
    """A closed box around the input (Houdini's Bound); detail attributes ``bound_min``/``bound_max``/``bound_size``
    (in the box's own axes when oriented)."""
    from .primitives import box
    P = geo.P.astype(np.float64)
    if len(P) == 0:
        return box((0.0, 0.0, 0.0), (0.0, 0.0, 0.0), 1)
    axes = np.eye(3)
    if oriented and len(P) >= 3:
        _, _, vt = np.linalg.svd(P - P.mean(0), full_matrices=False)
        axes = vt if np.linalg.det(vt) > 0 else vt * np.array([[1.0], [1.0], [-1.0]])
    Q = P @ axes.T
    lo, hi = Q.min(0) - np.asarray(padding), Q.max(0) + np.asarray(padding)
    b = box(tuple(hi - lo), (0.0, 0.0, 0.0), 1)
    b.point["P"] = ((b.P.astype(np.float64) + (lo + hi) / 2.0) @ axes).astype(np.float32)
    b.detail["bound_min"], b.detail["bound_max"] = lo.tolist(), hi.tolist()
    b.detail["bound_size"] = (hi - lo).tolist()
    return b
