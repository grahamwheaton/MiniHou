"""Reads a Blender object into engine arrays (Blender -> engine, for ``blender_object`` nodes). Main thread only.

What is sent (evaluated mesh: modifiers applied), keyed like ``geo`` messages:

* ``point/P``, ``topo/vertex_point`` (corner -> point), ``topo/prim_offsets`` (CSR faces)
* every generic attribute on POINT / CORNER / FACE under its own name (CORNER is the engine's "vertex" domain,
  FACE its "prim"), except internal ones (``.name``), ``position`` and the encoded ``custom_normal``
* the render UV map as ``vertex/uv`` (other UV maps keep their names), the render color attribute as ``Cd`` (RGB) plus
  ``Alpha`` when it is not opaque; ``material_index`` and ``sharp_face`` come through as face attributes of those names
* vertex groups as point floats named after the group (Blender keeps them outside the attribute API; skipped above
  ``MAX_GROUP_POINTS`` points)
* ``vertex/N`` only when the mesh has *custom* normals (authored data); otherwise Blender recomputes normals anyway

Edge attributes (e.g. ``sharp_edge``) are skipped: the engine has no edge domain.
"""
from __future__ import annotations

import time
from typing import Dict, List, Tuple

import numpy as np

from .protocol import TOPO_PRIM_OFFSETS, TOPO_VERTEX_POINT, attr_key

_DOMAIN = {"POINT": "point", "CORNER": "vertex", "FACE": "prim"}
# data_type -> (foreach property, components, numpy dtype)
_TYPES = {
    "FLOAT": ("value", 1, np.float32), "FLOAT2": ("vector", 2, np.float32), "FLOAT_VECTOR": ("vector", 3, np.float32),
    "FLOAT_COLOR": ("color", 4, np.float32), "BYTE_COLOR": ("color", 4, np.float32),
    "INT": ("value", 1, np.int32), "INT8": ("value", 1, np.int32), "BOOLEAN": ("value", 1, bool),
    "INT32_2D": ("value", 2, np.int32), "QUATERNION": ("value", 4, np.float32),
}
SKIP = {"position", "custom_normal"}
OWN_OBJECTS = ("MiniHou_Output", "MiniHou_Proxy")
# Vertex groups have no bulk API: they are read point by point (~0.8 us/point), so huge meshes skip them. A Geometry
# Nodes modifier with Store Named Attribute turns a group into a real attribute, which is read at full speed.
MAX_GROUP_POINTS = 250_000


def _get(collection, prop: str, n: int, width: int, dtype) -> np.ndarray:
    raw = np.empty(n * width, np.int8 if dtype is bool else dtype)
    collection.foreach_get(prop, raw)
    a = raw.astype(bool) if dtype is bool else raw
    return a.reshape(n, width) if width > 1 else a


def read_object(obj, depsgraph) -> Tuple[dict, Dict[str, np.ndarray]]:
    """``(header fields, arrays)`` for an ``object_geo`` message. Raises ValueError if it cannot be read."""
    if obj.name in OWN_OBJECTS:
        raise ValueError("MiniHou's own objects cannot be inputs (the output would feed itself)")
    t0 = time.perf_counter()
    ev = obj.evaluated_get(depsgraph)
    try:
        me = ev.to_mesh()
    except RuntimeError as e:
        raise ValueError(f"cannot be converted to a mesh ({e})") from None
    if me is None:
        raise ValueError(f"a {obj.type.lower()} object has no mesh data")
    try:
        arrays, info = _read_mesh(obj, me)
    finally:
        ev.to_mesh_clear()
    info["read_ms"] = round((time.perf_counter() - t0) * 1000, 2)
    M = [v for row in obj.matrix_world for v in row]
    return {"matrix": M, "info": info}, arrays


def _read_mesh(obj, me) -> Tuple[Dict[str, np.ndarray], dict]:
    n, nl, nf = len(me.vertices), len(me.loops), len(me.polygons)
    arrays: Dict[str, np.ndarray] = {}
    P = np.empty(n * 3, np.float32)
    try:
        me.attributes["position"].data.foreach_get("vector", P)
    except (KeyError, RuntimeError, TypeError):
        me.vertices.foreach_get("co", P)
    arrays[attr_key("point", "P")] = P.reshape(n, 3)
    vp = np.empty(nl, np.int32)
    try:
        me.attributes[".corner_vert"].data.foreach_get("value", vp)
    except (KeyError, RuntimeError, TypeError):
        me.loops.foreach_get("vertex_index", vp)
    starts = np.empty(nf, np.int32)
    me.polygons.foreach_get("loop_start", starts)
    arrays[TOPO_VERTEX_POINT] = vp
    arrays[TOPO_PRIM_OFFSETS] = np.append(starts, np.int32(nl)).astype(np.int32)

    render_uv = next((l.name for l in me.uv_layers if l.active_render), None)
    render_color = None
    try:
        idx = me.color_attributes.render_color_index
        if 0 <= idx < len(me.color_attributes):
            render_color = me.color_attributes[idx].name
    except (AttributeError, IndexError):
        pass
    names: List[str] = []
    skipped: List[str] = []
    sizes = {"POINT": n, "CORNER": nl, "FACE": nf}
    for a in me.attributes:
        if a.name.startswith(".") or a.name in SKIP:
            continue
        spec = _TYPES.get(a.data_type)
        if spec is None or a.domain not in _DOMAIN:
            skipped.append(f"{a.name} ({a.domain} {a.data_type})")
            continue
        prop, width, dtype = spec
        data = _get(a.data, prop, sizes[a.domain], width, dtype)
        dom = _DOMAIN[a.domain]
        name = a.name
        if a.name == render_uv:
            name = "uv"
        elif a.name == render_color and width == 4:
            if not np.allclose(data[:, 3], 1.0):
                arrays[attr_key(dom, "Alpha")] = np.ascontiguousarray(data[:, 3])
            data, name = np.ascontiguousarray(data[:, :3]), "Cd"
        if dtype is bool:
            data = data.astype(np.int32)                     # the engine keeps flags as ints (sharp_face, ...)
        arrays[attr_key(dom, name)] = data
        names.append(name)
    groups = _vertex_groups(obj, me, n, set(names))
    if n > MAX_GROUP_POINTS and len(getattr(obj, "vertex_groups", [])):
        skipped.append(f"vertex groups (over {MAX_GROUP_POINTS:,} points: store them as attributes with Geometry Nodes)")
    for gname, w in groups.items():
        arrays[attr_key("point", gname)] = w
    if getattr(me, "has_custom_normals", False) and nl:
        cn = np.empty(nl * 3, np.float32)
        me.corner_normals.foreach_get("vector", cn)
        arrays[attr_key("vertex", "N")] = cn.reshape(nl, 3)
        names.append("N")
    return arrays, {"points": n, "vertices": nl, "prims": nf, "attributes": names, "vertex_groups": list(groups),
                    "skipped": skipped}


def _vertex_groups(obj, me, n: int, taken: set) -> Dict[str, np.ndarray]:
    groups = list(getattr(obj, "vertex_groups", []))
    if not groups or n > MAX_GROUP_POINTS:
        return {}
    weights = {g.index: np.zeros(n, np.float32) for g in groups}
    for v in me.vertices:
        for ge in v.groups:
            w = weights.get(ge.group)
            if w is not None:
                w[v.index] = ge.weight
    return {g.name: weights[g.index] for g in groups if g.name not in taken and g.name not in ("P", "uv", "Cd")}
