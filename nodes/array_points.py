"""Small, bounded target lattices used by the bundled array systems."""
import numpy as np
from minihou.geometry import Geometry
from minihou.sdk import Bool, Float, Guide, Handle, Input, Int, Vec3, node

LIMIT = 10000


def _move_handles(param):
    return [Handle(param, axis=axis, comp=i, at="@" + param, label=param + " " + axis.upper())
            for i, axis in enumerate("xyz")]


def _source_size(source):
    if source is None or not source.npoints:
        raise ValueError("Relative spacing needs nonempty source geometry")
    return np.ptp(source.P.astype(np.float64), axis=0)



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
], handles=_move_handles("center") + [
    Handle("radius", axis="x", at="@center", reach=True, min=0, label="Circle radius"),
    Handle("start_angle", kind="ring", axis="z", at="@center", scale=180/np.pi, label="Start angle")
], guides=[Guide("circle", at="@center", axis="z", size="@radius", label="Array ring")])
def circle_points(count, radius, center, start_angle, rotate_copies):
    """Evenly spaced ring targets without a duplicate endpoint; Z is up."""
    a = np.radians(start_angle) + np.arange(count) * (2 * np.pi / count)
    P = np.column_stack((radius * np.cos(a), radius * np.sin(a), np.zeros(count))) + center
    out = _points(P)
    if rotate_copies:
        out.point["rot"] = np.column_stack((np.zeros(count), np.zeros(count), a)).astype(np.float32)
    return out


@node("array_grid_points", inputs=[Input("source", optional=True)], category="source", params=[
    Int("columns", 5, min=1, max=LIMIT, soft_max=100),
    Int("rows", 5, min=1, max=LIMIT, soft_max=100),
    Float("spacing_x", 1., doc="Signed distance between columns"),
    Float("spacing_y", 1., doc="Signed distance between rows"),
    Vec3("origin", (0, 0, 0)),
    Bool("relative_spacing", False, doc="Spacing X/Y become factors of source bounding-box dimensions"),
], handles=_move_handles("origin") + [
    Handle("spacing_x", axis="x", at="@origin", reach=True, when="relative_spacing=False", label="Column spacing"),
    Handle("spacing_y", axis="y", at="@origin", reach=True, when="relative_spacing=False", label="Row spacing")
])
def grid_points(source, columns, rows, spacing_x, spacing_y, origin, relative_spacing=False):
    """XY lattice from origin; row-major order. At most 10000 total targets."""
    if columns * rows > LIMIT:
        raise ValueError("Grid Array is limited to 10000 total copies; reduce rows or columns")
    if relative_spacing:
        size = _source_size(source)
        spacing_x, spacing_y = spacing_x * size[0], spacing_y * size[1]
    x, y = np.meshgrid(np.arange(columns) * spacing_x, np.arange(rows) * spacing_y)
    return _points(np.column_stack((x.ravel(), y.ravel(), np.zeros(columns * rows))) + origin)


@node("array_linear_points", inputs=[Input("source", optional=True)], category="source", params=[
    Int("count", 5, min=1, max=LIMIT, soft_max=100),
    Vec3("offset", (1, 0, 0), doc="XYZ step in offset mode; direction in distance mode"),
    Vec3("origin", (0, 0, 0)),
    Bool("use_distance", False, doc="Use Distance along Offset direction instead of XYZ offset"),
    Bool("relative_spacing", False, doc="Use Relative Offset times source bounds; overrides Use Distance"),
    Bool("use_end_point", False, doc="Evenly span Origin to End; overrides other spacing toggles"),
    Float("distance", 1., min=0., doc="Distance per copy along Offset direction"),
    Vec3("relative_offset", (1, 0, 0), doc="Per-axis multiples of source bounding-box size"),
    Vec3("end", (4, 0, 0), doc="Last target in endpoints mode; count includes origin and end"),
], handles=_move_handles("origin") + [
    Handle("offset", axis=axis, comp=i, at="@origin", reach=True, when="use_distance=False&relative_spacing=False&use_end_point=False", label="Offset " + axis.upper())
    for i, axis in enumerate("xyz")
] + [Handle("distance", axis="@offset", at="@origin", reach=True, min=0,
            when="use_distance&relative_spacing=False&use_end_point=False", label="Copy distance")]
  + [Handle("end", axis=axis, comp=i, at="@end", when="use_end_point", label="End " + axis.upper())
     for i, axis in enumerate("xyz")])
def linear_points(source, count, offset, origin, use_distance=False, relative_spacing=False, use_end_point=False, distance=1., relative_offset=(1,0,0), end=(4,0,0)):
    """Offset, measured distance, source-size factor or start/end targets; first target at origin."""
    if use_end_point:
        return _points(np.asarray(origin) + np.linspace(0.,1.,count)[:,None]*(np.asarray(end)-origin))
    step = np.asarray(offset, np.float64)
    if relative_spacing:
        step = _source_size(source) * relative_offset
    elif use_distance:
        length = np.linalg.norm(step)
        if length < 1e-12:
            raise ValueError("Distance spacing needs a nonzero Offset direction")
        step = step / length * distance
    return _points(np.arange(count)[:, None] * step + origin)


@node("array_between_points", category="source", params=[
    Int("count", 5, min=1, max=LIMIT, soft_max=100),
    Vec3("start", (0, 0, 0)), Vec3("end", (4, 0, 0)),
], handles=_move_handles("start") + _move_handles("end"))
def between_points(count, start, end):
    """Even targets including both endpoints; count=1 places one copy at start."""
    t = np.linspace(0., 1., count)[:, None]
    return _points(np.asarray(start) + t * (np.asarray(end) - start))
