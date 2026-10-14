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
    geo = _named(data.geo, object, (data.info or {}).get("collection", ""))
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


def _named(geo, name: str, collection: str = ""):
    """``geo`` with the object's name (detail ``object_name``, and ``object_collection`` when known) so a scene_objects node can
    be fed the object itself."""
    if geo.detail.get("object_name") == name and geo.detail.get("object_collection", "") == collection:
        return geo
    out = geo.copy()
    out.detail["object_name"] = name
    if collection:
        out.detail["object_collection"] = collection
    return out


def _collection_token(p) -> str:
    c = INPUTS.get_collection(p["collection"])
    return INPUTS.collection_token(p["collection"]) + "".join(INPUTS.token(o) for o in (c.objects if c else ()))


@node("blender_collection", category="source", key_extra=_collection_token, params=[
    Str("collection", "", doc="Name of the Blender collection (its child collections are included)"),
    Enum("output", ["points", "geometry"], doc="points: one point per object, at its origin, named so scene_objects / instance can pick "
                                               "from them (attributes instance, rot, scale; detail instance_objects); geometry: the "
                                               "geometry of every object merged together in world space"),
    Str("pattern", "", doc="Only objects whose name matches this (space separated wildcards, ^ excludes); empty: all"),
])
def blender_collection(collection, output="points", pattern=""):
    """Every object of a Blender collection, live. As points (the default) it is a list of the collection's objects: wire it into
    scene_objects' ``source`` and each point duplicates one of them (cycle, random or first); as geometry it is all of them merged.
    The detail attributes ``collection_name`` and ``collection_objects`` carry the names."""
    from ..geometry import Geometry
    from .instance import matrix_to_euler_xyz
    if not collection:
        raise ValueError("set 'collection' to the name of a Blender collection")
    cdata = INPUTS.get_collection(collection)
    if cdata is None:
        raise ValueError(f"waiting for Blender collection {collection!r} (is Blender connected to this engine?)")
    if cdata.error:
        raise ValueError(f"Blender collection {collection!r}: {cdata.error}")
    names = [n for n in cdata.objects if n not in OWN_OBJECTS]
    if pattern.strip():
        import fnmatch
        pats = pattern.split()
        inc = [p for p in pats if not p.startswith("^")]
        exc = [p[1:] for p in pats if p.startswith("^")]
        names = [n for n in names if (not inc or any(fnmatch.fnmatchcase(n, p) for p in inc))
                 and not any(fnmatch.fnmatchcase(n, p) for p in exc)]
    mats = []
    datas = []
    for n in names:
        d = INPUTS.get(n)
        datas.append(d)
        mats.append(np.asarray(d.matrix, np.float64).reshape(4, 4) if d is not None and d.matrix is not None else np.eye(4))
    if output == "geometry":
        from .merge import merge
        parts = [blender_object(n, "world") for n, d in zip(names, datas) if d is not None and d.geo is not None]
        geo = merge(parts) if parts else Geometry()
    else:
        P = np.array([m[:3, 3] for m in mats], np.float64).reshape(-1, 3)
        geo = Geometry.from_arrays(P.astype(np.float32), np.zeros(0, np.int32), np.zeros(1, np.int32))
        n_ = len(names)
        if n_:
            scale = np.array([np.linalg.norm(m[:3, :3], axis=0) for m in mats], np.float32)
            R = np.array([m[:3, :3] / np.maximum(np.linalg.norm(m[:3, :3], axis=0), 1e-12) for m in mats])
            rot = matrix_to_euler_xyz(R).astype(np.float32)
            geo.point["instance"] = np.arange(1, n_ + 1, dtype=np.int32)
            geo.point["rot"] = rot
            geo.point["scale"] = scale
        geo.detail["instance_objects"] = list(names)
    geo.detail["collection_name"] = collection
    geo.detail["collection_objects"] = list(names)
    return geo


@node("modifier_input", category="source")
def modifier_input():
    """Mesh entering this MiniHou modifier at its stack position, in object coordinates.

    Supplied by an independent modifier instance. This node cannot cook in the legacy proxy session.
    """
    raise ValueError("modifier_input requires a MiniHou modifier instance and its preceding stack geometry")


@node("object_input", category="source", params=[Str("binding", "", doc="Modifier object input name; blank uses this node's name")])
def object_input(binding):
    """Import an evaluated Blender mesh from this modifier's own object picker, in source-local coordinates."""
    raise ValueError("Choose an object in this modifier's Object input picker in Blender")
