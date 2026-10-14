"""Small, bounded target lattices used by the bundled array systems."""
import numpy as np
from minihou.geometry import Geometry
from minihou.sdk import Bool, Float, Int, Vec3, node

LIMIT = 10000


def _points(P):
    P = np.asarray(P, np.float32)
    if not np.isfinite(P).all():
        raise ValueError("Array positions must be finite")
    out = Geometry.points_only(P)
    out.point["array_index"] = np.arange(len(P), dtype=np.int32)
    return out


@node("array_circle_points", category="source", params=[
    Int("count", 12, min=1, max=LIMIT, soft_max=100),
    Float("radius", 2., min=0.), Vec3("center", (0, 0, 0)),
    Float("start_angle", 0., doc="Degrees from +X in the XY plane"),
    Bool("rotate_copies", False, doc="Turn each copy around Z to follow its radial angle"),
])
def circle_points(count, radius, center, start_angle, rotate_copies):
    """Evenly spaced ring targets without a duplicate endpoint; Z is up."""
    a = np.radians(start_angle) + np.arange(count) * (2 * np.pi / count)
    P = np.column_stack((radius * np.cos(a), radius * np.sin(a), np.zeros(count))) + center
    out = _points(P)
    if rotate_copies:
        out.point["rot"] = np.column_stack((np.zeros(count), np.zeros(count), a)).astype(np.float32)
    return out


@node("array_grid_points", category="source", params=[
    Int("columns", 5, min=1, max=LIMIT, soft_max=100),
    Int("rows", 5, min=1, max=LIMIT, soft_max=100),
    Float("spacing_x", 1., doc="Signed distance between columns"),
    Float("spacing_y", 1., doc="Signed distance between rows"),
    Vec3("origin", (0, 0, 0)),
])
def grid_points(columns, rows, spacing_x, spacing_y, origin):
    """XY lattice from origin; row-major order. At most 10000 total targets."""
    if columns * rows > LIMIT:
        raise ValueError("Grid Array is limited to 10000 total copies; reduce rows or columns")
    x, y = np.meshgrid(np.arange(columns) * spacing_x, np.arange(rows) * spacing_y)
    return _points(np.column_stack((x.ravel(), y.ravel(), np.zeros(columns * rows))) + origin)


@node("array_linear_points", category="source", params=[
    Int("count", 5, min=1, max=LIMIT, soft_max=100),
    Vec3("offset", (1, 0, 0), doc="XYZ translation per copy"),
    Vec3("origin", (0, 0, 0)),
])
def linear_points(count, offset, origin):
    """Constant XYZ offset; the first target is at origin."""
    return _points(np.arange(count)[:, None] * np.asarray(offset) + origin)


@node("array_between_points", category="source", params=[
    Int("count", 5, min=1, max=LIMIT, soft_max=100),
    Vec3("start", (0, 0, 0)), Vec3("end", (4, 0, 0)),
])
def between_points(count, start, end):
    """Even targets including both endpoints; count=1 places one copy at start."""
    t = np.linspace(0., 1., count)[:, None]
    return _points(np.asarray(start) + t * (np.asarray(end) - start))
