"""Geometry from Blender: a mesh object streamed live over the link (Houdini's Object Merge)."""
import numpy as np

from ..inputs import INPUTS
from ..sdk import Bool, Enum, Str, node

# MiniHou's own objects in Blender: reading them back would feed the output into itself.
OWN_OBJECTS = ("MiniHou_Output", "MiniHou_Proxy")


@node("blender_object", category="source", key_extra=lambda p: INPUTS.token(p["object"]), params=[
    Str("object", "", doc="Name of the Blender object to read: a mesh, a curve (Bezier, NURBS, poly, hair curves), a point cloud, text, "
                          "or the output of Geometry Nodes, which can hold all of those at once"),
    Enum("space", ["world", "object"], doc="world: where the object is in the scene; object: its own local coordinates"),
    Bool("join_edges", True, doc="Join the loose edges of a mesh into long curves (real curves and point clouds already come whole)"),
])
def blender_object(object, space, join_edges=True):
    """A Blender object's evaluated mesh (modifiers applied), live: edit it in Blender and the graph recooks.
    Native data arrives as attributes: the render UV map as `uv` and other UV maps by name (vertices), the render color
    attribute as `Cd` (+ `Alpha` if not opaque), `material_index` and `sharp_face` (primitives), vertex groups as point
    floats named after the group, custom normals as vertex `N`, and every other attribute under its own name. Loose
    edges (an edge-only mesh) become curves.

    Curves and point clouds come in as what they are: each curve is an open primitive (cyclic ones closed), sampled exactly as Blender
    draws it (Bezier handles, NURBS, Catmull-Rom, poly), with `radius` and `tilt` as point attributes and curve attributes as primitive
    attributes; a point cloud is points with `radius` and its attributes. An object whose evaluated geometry holds several kinds (a
    mesh, curves and points from one Geometry Nodes tree) arrives as one geometry with the parts side by side."""
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
    if join_edges and geo.topology.has_open and "curves" not in (data.info or {}).get("components", []):
        from .curves import polypath
        geo = polypath(geo)
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


@node("modifier_input", category="source")
def modifier_input():
    """Mesh entering this MiniHou modifier at its stack position, in object coordinates.

    The Rust editor may store an ordered ``modifier_inputs`` interface on this node.
    Each entry has a stable id, Blender socket name and type, and a default; a
    node parameter can refer to it with ``{"modifier_input":{"node":"input","id":"..."}}``.

    Supplied by an independent modifier instance. This node cannot cook in the legacy proxy session.
    """
    raise ValueError("modifier_input requires a MiniHou modifier instance and its preceding stack geometry")


@node("object_input", category="source", params=[Str("binding", "", doc="Modifier object input name; blank uses this node's name")])
def object_input(binding):
    """Import an evaluated Blender mesh from this modifier's own object picker, in source-local coordinates."""
    raise ValueError("Choose an object in this modifier's Object input picker in Blender")
