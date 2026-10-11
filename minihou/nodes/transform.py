import numpy as np

from .. import groups
from ..sdk import Float, Str, Vec3, node
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
