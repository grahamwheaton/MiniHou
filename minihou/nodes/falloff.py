"""Falloff: a 0..1 weight per point or primitive from a shape (the fields of Blender's Higgsas falloff nodes, as one node)."""
import numpy as np

from .. import groups, topo
from ..raycast import inside
from ..sdk import Bool, Enum, Float, Guide, Handle, Input, Int, Str, Vec3, node
from ._util import normalize_rows
from .ray import target_grid

SHAPES = ["sphere", "directional", "radial", "wave", "index", "mesh"]


def _smooth(t):
    t = np.clip(t, 0.0, 1.0)
    return t * t * (3.0 - 2.0 * t)


@node("falloff", inputs=["geo", Input("shape_mesh", optional=True, doc="The mesh for shape 'mesh' (weight by distance to it / inside it)")],
      category="attribute", params=[
    Str("name", "mask", doc="The float attribute to write"),
    Enum("domain", ["points", "primitives"], doc="primitives use the face centre and the primitive number"),
    Enum("shape", SHAPES, doc="sphere: 1 at the centre, 0 at the radius; directional: 0 to 1 along a direction; radial: 0 to 1 once round an axis; "
                              "wave: a pulse travelling along a direction; index: by point (or primitive) number; mesh: by distance to the mesh on input 2"),
    Vec3("center", (0.0, 0.0, 0.0), doc="Sphere centre; start of the ramp for directional; the axis point for radial and wave"),
    Vec3("direction", (0.0, 0.0, 1.0), doc="Ramp / axis / travel direction (directional, radial, wave)"),
    Float("size", 1.0, min=1e-9, soft_max=10.0, doc="sphere: radius; directional: length of the ramp; wave: width of the pulse; mesh: distance over which the weight fades"),
    Float("blend", 1.0, min=0.0, max=1.0, doc="sphere and mesh: the share of the size over which the weight fades (0: hard edge, 1: from the middle)"),
    Float("offset", 0.0, soft_min=-10.0, soft_max=10.0, doc="directional: slide the ramp along the direction; radial: start angle in degrees; wave: position of the front; index: first index"),
    Float("narrowness", 1.0, min=0.01, soft_max=8.0, doc="wave: sharpness of the pulse (higher is narrower)"),
    Int("every", 0, min=0, soft_max=100, doc="index: repeat the ramp every N elements (0: once)"),
    Float("range", 10.0, min=1e-9, soft_max=1000.0, doc="index: how many elements the ramp takes"),
    Bool("inside", False, doc="mesh: points inside a closed mesh get 1"),
    Bool("smooth", True, doc="Ease the ramp (smoothstep)"),
    Bool("mirror", False, doc="directional, radial and index: a ramp that rises and falls again (symmetric about its middle)"),
    Bool("flip", False, doc="1 - weight"),
    Float("min", 0.0, soft_min=-1.0, soft_max=1.0, doc="The weight where the falloff is 0 (the output range is min..max)"),
    Float("max", 1.0, soft_min=-1.0, soft_max=2.0, doc="The weight where the falloff is 1"),
    Str("group", "", doc="Only these elements (the rest get min)"),
], handles=[Handle("size", axis="x", at="@center", reach=True, min=1e-6, when="shape=sphere")],
   guides=[Guide("sphere", at="@center", size="@size", when="shape=sphere", id="sphere", label="falloff radius"),
           Guide("line", at="@center", axis="@direction", size="@size", when="shape=directional", id="ramp", label="ramp"),
           Guide("plane", at="@center", axis="@direction", when="shape=directional", id="start", label="ramp start"),
           Guide("line", at="@center", axis="@direction", when="shape=radial", id="axis", label="axis"),
           Guide("line", at="@center", axis="@direction", when="shape=wave", id="travel", label="wave direction")])
def falloff(geo, shape_mesh, name, domain, shape, center, direction, size, blend, offset, narrowness, every, range, inside, smooth, mirror,
            flip, min, max, group):
    """Write a 0..1 weight (remapped to min..max) from a shape: for masks, soft selections, deformer strengths and colours. The
    result is a normal float attribute that wrangles, soft_transform-like nodes, attribute_transfer and Blender all read."""
    dom = "point" if domain == "points" else "prim"
    groups.attr_name(name)
    count = geo.npoints if dom == "point" else geo.nprims
    if dom == "point":
        P = geo.P.astype(np.float64)
    else:
        P = topo.prim_centroids(geo).astype(np.float64) if geo.nprims else np.zeros((0, 3))
    idx = np.arange(count, dtype=np.float64)
    c = np.asarray(center, np.float64)
    d = np.asarray(direction, np.float64)
    if shape in ("directional", "radial", "wave") and float(np.linalg.norm(d)) < 1e-12:
        raise ValueError("falloff: the direction is zero")
    d = d / max_norm(d)
    rel = P - c
    if shape == "sphere":
        dist = np.linalg.norm(rel, axis=1)
        w = (size - dist) / (size * blend) if blend > 0 else (dist < size).astype(np.float64)
        w = np.clip(w, 0.0, 1.0)
    elif shape == "directional":
        x = rel @ d - offset
        t = np.abs(x) / size if mirror else x / size
        w = 1.0 - np.clip(t, 0.0, 1.0) if mirror else np.clip(t, 0.0, 1.0)
        mirror = False
    elif shape == "radial":
        a, b = _basis(d)
        ang = np.degrees(np.arctan2(rel @ b, rel @ a)) - offset
        t = (ang % 360.0) / 360.0
        w = (1.0 - np.abs(2.0 * t - 1.0)) if mirror else t
        mirror = False
    elif shape == "wave":
        u = (rel @ d - offset) / size
        w = np.clip(1.0 - np.abs(u), 0.0, 1.0) ** narrowness
        smooth_wave = smooth
        w = _smooth(w) if smooth_wave else w
        smooth = False
    elif shape == "index":
        i = idx - offset
        if every:
            i = i % every
        t = i / range
        w = (1.0 - np.abs(2.0 * np.clip(t, 0.0, 1.0) - 1.0)) if mirror else np.clip(t, 0.0, 1.0)
        mirror = False
    else:
        if shape_mesh is None or not shape_mesh.nprims:
            raise ValueError("falloff: connect the mesh to the second input for shape 'mesh'")
        grid = target_grid(shape_mesh)[0]
        _, dist, _ = grid.closest(P)
        w = np.clip((size - dist) / (size * blend), 0.0, 1.0) if blend > 0 else (dist < size).astype(np.float64)
        if inside:
            ext = float(np.ptp(shape_mesh.P, axis=0).max()) if shape_mesh.npoints else 1.0
            w = np.where(_inside(grid, P, ext), 1.0, w)
    if smooth and shape != "wave":
        w = _smooth(w)
    if flip:
        w = 1.0 - w
    value = min + w * (max - min)
    sel = groups.mask(geo, dom, group)
    if sel is not None:
        value = np.where(sel, value, min)
    out = geo.copy()
    out.table(dom)[name] = value.astype(np.float32)
    return out


def max_norm(v):
    return float(np.linalg.norm(v))


def _basis(axis):
    """Two unit vectors perpendicular to the axis and to each other (the angle is measured from the first)."""
    ref = np.array([1.0, 0.0, 0.0]) if abs(axis[0]) < 0.9 else np.array([0.0, 1.0, 0.0])
    a = ref - axis * (ref @ axis)
    a /= np.linalg.norm(a)
    return a, np.cross(axis, a)


def _inside(grid, P, ext):
    return inside(grid, P, ext)
