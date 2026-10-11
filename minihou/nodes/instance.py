"""Instancing: points that become instances of Blender objects (Houdini's instance attribute / packed instancing).

The engine only decides *which* object goes on each point and how it is rotated and scaled; Blender does the
instancing in the Geometry Nodes proxy (Instance on Points), so a forest of a million trees costs a million points,
not a million copies of a tree. Point attributes written:

* ``instance`` -- 1-based index into the object list (0 = not an instance point, which is what merge zero-fills)
* ``rot``      -- Euler XYZ rotation in radians (Blender's default rotation order)
* ``scale``    -- per-axis scale

and the detail attribute ``instance_objects`` (the object names, in order).
"""
import numpy as np

from .. import groups
from ..geometry import rows
from ..sdk import Enum, Float, Int, Str, node
from ._util import rotation_from_z_to


def matrix_to_euler_xyz(R: np.ndarray) -> np.ndarray:
    """Euler XYZ angles (radians) of rotation matrices (k, 3, 3), with R = Rz @ Ry @ Rx (Blender's 'XYZ')."""
    cy = np.hypot(R[:, 0, 0], R[:, 1, 0])
    y = np.arctan2(-R[:, 2, 0], cy)                       # atan2 keeps precision near +-90 degrees (arcsin does not)
    ok = cy > 1e-9
    x = np.where(ok, np.arctan2(R[:, 2, 1], R[:, 2, 2]), np.arctan2(-R[:, 1, 2], R[:, 1, 1]))
    z = np.where(ok, np.arctan2(R[:, 1, 0], R[:, 0, 0]), 0.0)
    return np.stack([x, y, z], 1)


def euler_xyz_to_matrix(e: np.ndarray) -> np.ndarray:
    x, y, z = e[:, 0], e[:, 1], e[:, 2]
    cx, sx, cy, sy, cz, sz = np.cos(x), np.sin(x), np.cos(y), np.sin(y), np.cos(z), np.sin(z)
    R = np.empty((len(e), 3, 3))
    R[:, 0] = np.stack([cy * cz, sx * sy * cz - cx * sz, cx * sy * cz + sx * sz], 1)
    R[:, 1] = np.stack([cy * sz, sx * sy * sz + cx * cz, cx * sy * sz - sx * cz], 1)
    R[:, 2] = np.stack([-sy, sx * cy, cx * cy], 1)
    return R


@node("instance", inputs=["points"], category="generate", params=[
    Str("objects", "", doc="Blender objects to place, space separated (e.g. 'Tree Bush Rock'); they stay in your scene"),
    Enum("pick", ["random", "cycle", "attribute"], doc="Which object each point gets; attribute: from 'pick_attrib'"),
    Str("pick_attrib", "variant", doc="pick = attribute: an int point attribute, 0 = first object (wraps around)"),
    Int("seed", 0),
    Enum("align", ["normal", "up"], doc="normal: the object's +Z follows the point's N; up: objects stay upright"),
    Float("random_spin", 360.0, min=0.0, max=360.0, doc="Random rotation about the object's own +Z, in degrees"),
    Float("scale_min", 1.0, min=0.0, soft_max=4.0, doc="Random uniform scale range, times the point's pscale if any"),
    Float("scale_max", 1.0, min=0.0, soft_max=4.0),
    Str("group", "", doc="Only these points get instances"),
])
def instance(points, objects, pick, pick_attrib, seed, align, random_spin, scale_min, scale_max, group):
    """Turn points into instances of Blender objects: Blender's Geometry Nodes proxy places the real objects on them
    (Instance on Points), so huge counts stay cheap and the objects stay editable in Blender."""
    names = [n for n in objects.replace(",", " ").split() if n]
    if not names:
        raise ValueError("list the Blender objects to instance, e.g. 'Tree Rock'")
    n = points.npoints
    rng = np.random.default_rng(int(seed) & 0xFFFFFFFF)
    if pick == "random":
        idx = rng.integers(0, len(names), n)
    elif pick == "cycle":
        idx = np.arange(n) % len(names)
    else:
        a = points.point.get(pick_attrib)
        if a is None:
            raise ValueError(f"no point attribute {pick_attrib!r} to pick objects by")
        idx = rows(a)[:, 0].astype(np.int64) % len(names)
    spin = np.radians(rng.random(n) * random_spin)
    R_spin = np.zeros((n, 3, 3))
    R_spin[:, 0, 0] = R_spin[:, 1, 1] = np.cos(spin)
    R_spin[:, 0, 1], R_spin[:, 1, 0] = -np.sin(spin), np.sin(spin)
    R_spin[:, 2, 2] = 1.0
    if align == "normal" and "N" in points.point:
        N = points.point["N"].astype(np.float64)
        N = N / np.maximum(np.linalg.norm(N, axis=1, keepdims=True), 1e-30)   # float64: keeps R orthonormal
        R = rotation_from_z_to(N) @ R_spin
    else:
        R = R_spin
    s = scale_min + rng.random(n) * (scale_max - scale_min)
    if "pscale" in points.point:
        s = s * rows(points.point["pscale"])[:, 0].astype(np.float64)
    out = points.copy()
    sel = groups.mask(points, "point", group)
    inst = (idx + 1).astype(np.int32)
    if sel is not None:
        inst = np.where(sel, inst, 0).astype(np.int32)
    out.point["instance"] = inst
    out.point["rot"] = matrix_to_euler_xyz(R).astype(np.float32)
    out.point["scale"] = np.repeat(s[:, None], 3, 1).astype(np.float32)
    out.detail["instance_objects"] = names
    return out
