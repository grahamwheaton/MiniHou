"""Example node library: deformers. Files in this folder are loaded (and hot-reloaded) by the engine as node types."""
import numpy as np

from minihou.sdk import Float, Vec3, node


@node("twist", inputs=["geo"], category="deform", params=[
    Float("angle", 90.0, soft_min=-720.0, soft_max=720.0, doc="Degrees of rotation from the bottom to the top"),
    Vec3("center", (0.0, 0.0, 0.0), doc="A point on the twist axis (the axis is vertical, through this point)"),
])
def twist(geo, angle, center):
    """Twist geometry around a vertical axis: each point rotates in proportion to its height."""
    out = geo.copy()
    if geo.npoints == 0:
        return out
    P = geo.P.astype(np.float64)
    z0, z1 = P[:, 2].min(), P[:, 2].max()
    t = (P[:, 2] - z0) / (z1 - z0) if z1 > z0 else np.zeros(len(P))
    a = np.radians(angle) * t
    c, s = np.cos(a), np.sin(a)
    x, y = P[:, 0] - center[0], P[:, 1] - center[1]
    P[:, 0] = center[0] + c * x - s * y
    P[:, 1] = center[1] + s * x + c * y
    out.point["P"] = P.astype(np.float32)
    if "N" in geo.point:                                  # rotate normals the same way
        N = geo.point["N"].astype(np.float64)
        out.point["N"] = np.stack([c * N[:, 0] - s * N[:, 1], s * N[:, 0] + c * N[:, 1], N[:, 2]], 1).astype(np.float32)
    return out
