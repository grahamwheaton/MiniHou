"""Geometry from Blender: a mesh object streamed live over the link (Houdini's Object Merge)."""
import numpy as np

from ..inputs import INPUTS
from ..sdk import Enum, Str, node

# MiniHou's own objects in Blender: reading them back would feed the output into itself.
OWN_OBJECTS = ("MiniHou_Output", "MiniHou_Proxy")


@node("blender_object", category="source", key_extra=lambda p: INPUTS.token(p["object"]), params=[
    Str("object", "", doc="Name of the Blender object to read (mesh, or anything Blender can turn into a mesh)"),
    Enum("space", ["world", "object"], doc="world: where the object is in the scene; object: its own local coordinates"),
])
def blender_object(object, space):
    """A Blender object's evaluated mesh (modifiers applied), live: edit it in Blender and the graph recooks.
    Native data arrives as attributes: the render UV map as `uv` and other UV maps by name (vertices), the render color
    attribute as `Cd` (+ `Alpha` if not opaque), `material_index` and `sharp_face` (primitives), vertex groups as point
    floats named after the group, custom normals as vertex `N`, and every other attribute under its own name."""
    if not object:
        raise ValueError("set 'object' to the name of a Blender object")
    if object in OWN_OBJECTS:
        raise ValueError(f"{object!r} is MiniHou's own output; reading it would feed the result back into itself")
    data = INPUTS.get(object)
    if data is None:
        raise ValueError(f"waiting for Blender object {object!r} (is Blender connected to this engine?)")
    if data.geo is None:
        raise ValueError(f"Blender object {object!r}: {data.error}")
    geo = data.geo
    if space == "object" or data.matrix is None:
        return geo
    M = np.asarray(data.matrix, np.float64).reshape(4, 4)
    if np.allclose(M, np.eye(4)):
        return geo
    out = geo.copy()
    P = geo.P.astype(np.float64)
    out.point["P"] = (P @ M[:3, :3].T + M[:3, 3]).astype(np.float32)
    normal_m = np.linalg.inv(M[:3, :3]).T                       # normals transform by the inverse transpose
    for dom in ("point", "vertex", "prim"):
        table = geo.table(dom)
        if "N" in table and table["N"].ndim == 2 and table["N"].shape[1] == 3:
            n = table["N"].astype(np.float64) @ normal_m.T
            n /= np.maximum(np.linalg.norm(n, axis=1, keepdims=True), 1e-12)
            out.table(dom)["N"] = n.astype(np.float32)
    return out
