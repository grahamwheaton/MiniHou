"""Example node library: a generator that builds new topology with numpy (no Python loop over faces)."""
import numpy as np

from minihou.geometry import Geometry
from minihou.sdk import Bool, Float, Int, node

# One unit box: 8 corners, 6 outward-facing quads (counter-clockwise seen from outside).
_BOX_P = np.array([[0, 0, 0], [1, 0, 0], [1, 1, 0], [0, 1, 0], [0, 0, 1], [1, 0, 1], [1, 1, 1], [0, 1, 1]], np.float32)
_BOX_F = np.array([[0, 3, 2, 1], [4, 5, 6, 7], [0, 1, 5, 4], [1, 2, 6, 5], [2, 3, 7, 6], [3, 0, 4, 7]], np.int32)


@node("stairs", category="source", params=[
    Int("steps", 12, min=1, max=10000, soft_max=100),
    Float("width", 1.2, min=0.0, doc="Across the stairs (X)"),
    Float("run", 0.28, min=0.0, doc="Depth of one step (Y)"),
    Float("rise", 0.18, min=0.0, doc="Height of one step (Z)"),
    Float("spiral", 0.0, soft_min=-90.0, soft_max=90.0,
          doc="Degrees each step turns around the Z axis; 0 = a straight flight along +Y"),
    Float("inner_radius", 0.3, min=0.0, doc="Spiral only: gap between the Z axis and the steps (room for a column)"),
    Bool("solid", True, doc="Steps reach down to the ground; off = floating treads"),
    Float("thickness", 0.06, min=0.0, doc="Tread thickness when not solid"),
])
def stairs(steps, width, run, rise, spiral, inner_radius, solid, thickness):
    """Solid steps, one closed box each. Straight: a flight along +Y. Spiral: steps fan around the Z axis."""
    i = np.arange(steps)
    size = np.array([width, run, rise], np.float32)
    P = np.repeat(_BOX_P[None], steps, 0) * size                        # (steps, 8, 3) unit boxes scaled
    if solid:
        P[:, :, 2] *= (i + 1)[:, None]                                  # step i reaches down to the ground
    else:
        P[:, :, 2] = (i + 1)[:, None] * rise - (1 - _BOX_P[None, :, 2]) * thickness   # a tread at the step's height
    if spiral:
        P[:, :, 0] += inner_radius                                      # radial along +X, centred on the X axis
        P[:, :, 1] -= run / 2
        a = np.radians(spiral) * i[:, None]
        x, y = P[:, :, 0].copy(), P[:, :, 1].copy()
        P[:, :, 0], P[:, :, 1] = np.cos(a) * x - np.sin(a) * y, np.sin(a) * x + np.cos(a) * y
    else:
        P[:, :, 0] -= width / 2
        P[:, :, 1] += (i * run)[:, None]
    verts = (_BOX_F[None] + (8 * i)[:, None, None]).reshape(-1)          # 6 quads per step
    geo = Geometry.from_arrays(P.reshape(-1, 3), verts, np.arange(0, verts.size + 1, 4))
    geo.prim["step"] = np.repeat(i, 6).astype(np.int32)                  # which step each face belongs to
    return geo
