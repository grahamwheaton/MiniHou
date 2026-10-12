"""Scene objects: results that become real Blender objects, not part of the one MiniHou mesh.

``scene_objects`` is a sink (like a named ``output``): it passes its input through, and the live link (and a Blender modifier's
cook) turns the points it receives into a **manifest** the add-on realises as ordinary scene objects: one linked duplicate of a
Blender object per point, placed with the point's position, ``rot`` and ``scale``. You can then give each object its own
modifiers, materials, constraints; they are found again by a stable id, so the next cook moves the same objects instead of
making new ones. See :func:`build_manifest` for the wire form.
"""
import numpy as np

from ..geometry import Geometry, rows
from ..sdk import Enum, Int, Str, node

MAX_OBJECTS_LIMIT = 10_000


@node("scene_objects", inputs=["geo"], category="output", params=[
    Enum("mode", ["instances"], doc="instances: one Blender object per point (a linked duplicate of the object the point names), "
                                    "placed with the point's position, rot and scale"),
    Str("prefix", "MiniHou", doc="Object names start with this (Prefix_0001, ...)"),
    Str("collection", "", doc="Blender collection the objects go in (created if missing; empty: the prefix)"),
    Str("source", "", doc="Blender object to duplicate for points that do not say which (no 'instance' attribute); "
                          "points from the instance node name their objects themselves"),
    Str("id_attrib", "id", doc="Integer point attribute that gives each object a stable identity; without it the point number "
                               "is used (so changing how many points there are can hand objects to other points)"),
    Int("max_objects", 2000, min=1, max=MAX_OBJECTS_LIMIT, doc="Safety limit: more points than this is refused, not truncated"),
])
def scene_objects(geo, mode, prefix, collection, source, id_attrib, max_objects):
    """Make Blender objects out of points. The geometry passes through unchanged; what happens in Blender is described by the
    manifest the link sends (see the manual). Objects are never deleted by a cook: ones that are no longer needed are hidden, and
    a Clean up button in the MiniHou panel removes them for good."""
    plan(geo, mode, source, id_attrib, max_objects)                     # validates: a bad setup is an error on this node
    return geo


def plan(geo: Geometry, mode: str, source: str, id_attrib: str, max_objects: int) -> dict:
    """Which points become objects, which Blender object each duplicates, their ids and transforms (columnar, JSON-ready)."""
    n = geo.npoints
    inst = geo.point.get("instance")
    names = geo.detail.get("instance_objects")
    src_names: list = []
    if inst is not None and inst.ndim == 1 and isinstance(names, (list, tuple)) and names:
        src_names = [str(x) for x in names]
        pick = np.flatnonzero((inst > 0) & (inst <= len(src_names)))
        src_index = inst[pick].astype(np.int64) - 1
    else:
        if not str(source).strip():
            raise ValueError("set 'source' to the Blender object to duplicate (or feed points from the instance node, which name their objects)")
        src_names = [str(source).strip()]
        pick = np.arange(n)
        src_index = np.zeros(n, np.int64)
    if len(pick) > max_objects:
        raise ValueError(f"{len(pick):,} objects is more than max_objects ({max_objects:,}); raise the limit (at most "
                         f"{MAX_OBJECTS_LIMIT:,}) or make fewer points")
    ids = np.arange(len(pick), dtype=np.int64)
    idattr = geo.point.get(id_attrib) if id_attrib else None
    if idattr is not None and idattr.ndim == 1 and idattr.dtype.kind in "iu":
        ids = idattr[pick].astype(np.int64)
        if len(np.unique(ids)) != len(ids):
            raise ValueError(f"point attribute {id_attrib!r} has repeated values, so it cannot identify objects; "
                             f"fix it or clear id_attrib to use point numbers")
    elif idattr is not None:
        raise ValueError(f"point attribute {id_attrib!r} must be an integer to identify objects")
    P = np.asarray(geo.P, np.float64)[pick]
    rot = geo.point.get("rot")
    rot = rows(rot)[pick, :3].astype(np.float64) if rot is not None and rot.ndim == 2 and rot.shape[1] == 3 else np.zeros((len(pick), 3))
    scale = geo.point.get("scale")
    if scale is not None and scale.ndim == 2 and scale.shape[1] == 3:
        sc = rows(scale)[pick].astype(np.float64)
    else:
        ps = geo.point.get("pscale")
        sc = np.repeat((ps[pick].astype(np.float64) if ps is not None and ps.ndim == 1 else np.ones(len(pick)))[:, None], 3, 1)
    return {"sources": src_names, "id": ids.tolist(), "src": src_index.tolist(), "loc": np.round(P, 6).tolist(),
            "rot": np.round(rot, 7).tolist(), "scale": np.round(sc, 6).tolist()}


def build_manifest(node_id: str, params: dict, geo: Geometry, key: str) -> dict:
    """The message the add-on realises: ``{"type": "scene_objects", "node", "key", "mode", "prefix", "collection", ...columns}``.
    Columns: ``sources`` (Blender object names), and per object ``id``, ``src`` (index into sources), ``loc``, ``rot`` (Euler XYZ
    radians), ``scale``."""
    body = plan(geo, params["mode"], params["source"], params["id_attrib"], params["max_objects"])
    prefix = str(params["prefix"]).strip() or "MiniHou"
    return dict(type="scene_objects", node=node_id, key=key, mode=params["mode"], prefix=prefix[:60],
                collection=(str(params["collection"]).strip() or prefix)[:60], **body)
