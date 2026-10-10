"""Scene objects and the Geometry Nodes proxy. Main thread only.

* ``MiniHou_Output`` -- a hidden mesh object whose mesh the mesh writer fills with engine output.
* ``MiniHou_Proxy``  -- the object users work with. It carries a Geometry Nodes modifier whose
  node group reads the output object through an *Object Info* node. The group's inputs mirror the
  exposed engine parameters (they appear as ordinary modifier sliders).
"""
from __future__ import annotations

import json
from typing import Dict, List, Optional

import bpy

from .mesh_writer import assert_main_thread

OUTPUT_OBJECT = "MiniHou_Output"
PROXY_OBJECT = "MiniHou_Proxy"
COLLECTION = "MiniHou"
GROUP = "MiniHou"
MODIFIER = "MiniHou"
STRUCTURE_KEY = "minihou_structure"
MAP_KEY = "minihou_map"

_SOCKET_TYPES = {"float": "NodeSocketFloat", "int": "NodeSocketInt", "bool": "NodeSocketBool",
                 "vec3": "NodeSocketVector"}


def ensure_objects():
    """Create (or fetch) the collection, output object/mesh, proxy object and modifier.

    Returns ``(output_obj, proxy_obj, modifier)``. Called on every geometry update, so it only *writes*
    when something is actually missing or different: every bpy write here would tag the depsgraph and
    force a pointless re-evaluation. (Blender hands out a new Python wrapper on each attribute access,
    so identity checks such as ``mod.node_group is ng`` are always False; compare names instead.)
    """
    assert_main_thread()
    scene = bpy.context.scene
    col = bpy.data.collections.get(COLLECTION)
    if col is None:
        col = bpy.data.collections.new(COLLECTION)
    if col.name not in scene.collection.children:
        scene.collection.children.link(col)

    out = bpy.data.objects.get(OUTPUT_OBJECT)
    if out is None:
        mesh = bpy.data.meshes.get(OUTPUT_OBJECT) or bpy.data.meshes.new(OUTPUT_OBJECT)
        out = bpy.data.objects.new(OUTPUT_OBJECT, mesh)
    if out.name not in col.objects:
        col.objects.link(out)
    if not out.hide_render:
        out.hide_render = True
    try:
        if not out.hide_get():
            out.hide_set(True)      # eye icon: not drawn, but still evaluated because Object Info uses it
    except RuntimeError:
        pass                         # object not in the active view layer (e.g. collection excluded)

    proxy = bpy.data.objects.get(PROXY_OBJECT)
    if proxy is None:
        proxy = bpy.data.objects.new(PROXY_OBJECT, bpy.data.meshes.new(PROXY_OBJECT))
    if proxy.name not in col.objects:
        col.objects.link(proxy)

    ng = ensure_node_group(out)
    mod = proxy.modifiers.get(MODIFIER)
    if mod is None:
        mod = proxy.modifiers.new(MODIFIER, "NODES")
    if mod.node_group is None or mod.node_group.name != ng.name:
        mod.node_group = ng
    return out, proxy, mod


def output_mesh():
    obj = bpy.data.objects.get(OUTPUT_OBJECT)
    return obj.data if obj else None


def ensure_node_group(output_obj):
    ng = bpy.data.node_groups.get(GROUP)
    if ng is None:
        ng = bpy.data.node_groups.new(GROUP, "GeometryNodeTree")
        _build_nodes(ng, output_obj)
    else:
        info = next((n for n in ng.nodes if n.bl_idname == "GeometryNodeObjectInfo"), None)
        if info is not None:
            cur = info.inputs["Object"].default_value
            if cur is None or cur.name != output_obj.name:
                info.inputs["Object"].default_value = output_obj
    return ng


def _build_nodes(ng, output_obj, params: Optional[List[dict]] = None) -> None:
    iface = ng.interface
    iface.clear()
    iface.new_socket("Geometry", in_out="INPUT", socket_type="NodeSocketGeometry")
    iface.new_socket("Geometry", in_out="OUTPUT", socket_type="NodeSocketGeometry")
    mapping = {}
    for p in params or []:
        s = iface.new_socket(p["name"], in_out="INPUT", socket_type=_SOCKET_TYPES[p["kind"]])
        _configure_socket(s, p)
        mapping[p["name"]] = s.identifier
    ng[MAP_KEY] = json.dumps(mapping)
    gin = next((n for n in ng.nodes if n.bl_idname == "NodeGroupInput"), None) or ng.nodes.new("NodeGroupInput")
    gout = next((n for n in ng.nodes if n.bl_idname == "NodeGroupOutput"), None) or ng.nodes.new("NodeGroupOutput")
    info = next((n for n in ng.nodes if n.bl_idname == "GeometryNodeObjectInfo"), None) or ng.nodes.new("GeometryNodeObjectInfo")
    gin.location, info.location, gout.location = (-400, 0), (-150, -150), (150, 0)
    info.transform_space = "RELATIVE"          # engine output follows the proxy's transform, not its own
    info.inputs["As Instance"].default_value = False
    info.inputs["Object"].default_value = output_obj
    for l in list(ng.links):
        ng.links.remove(l)
    ng.links.new(info.outputs["Geometry"], gout.inputs["Geometry"])


def _configure_socket(s, p: dict) -> None:
    kind = p["kind"]
    if kind == "bool":
        s.default_value = bool(p["value"])
        return
    s.default_value = tuple(p["value"]) if kind == "vec3" else p["value"]
    # Blender sockets only have hard limits; the engine's hard limits map straight onto them.
    for src, dst in (("min", "min_value"), ("max", "max_value")):
        if src in p and hasattr(s, dst):
            setattr(s, dst, p[src])


def rebuild_interface(ng, output_obj, params: List[dict], structure: str) -> None:
    """Replace the group's inputs with the engine's exposed parameters."""
    assert_main_thread()
    _build_nodes(ng, output_obj, params)
    ng[STRUCTURE_KEY] = structure


def mapping(ng) -> Dict[str, str]:
    try:
        return json.loads(ng.get(MAP_KEY, "{}"))
    except ValueError:
        return {}


def canonical(kind: str, v):
    if kind == "vec3":
        return tuple(float(c) for c in v)
    if kind == "bool":
        return bool(v)
    if kind == "int":
        return int(v)
    return float(v)


def write_values(mod, ng, params: List[dict], values: Dict[str, object]) -> None:
    assert_main_thread()
    m = mapping(ng)
    for p in params:
        ident = m.get(p["name"])
        if ident is None or p["name"] not in values:
            continue
        v = values[p["name"]]
        mod[ident] = list(v) if p["kind"] == "vec3" else v
    mod.id_data.update_tag()


def read_values(mod, ng, params: List[dict]) -> Dict[str, object]:
    assert_main_thread()
    m = mapping(ng)
    out = {}
    for p in params:
        ident = m.get(p["name"])
        if ident is None or ident not in mod.keys():
            continue
        out[p["name"]] = canonical(p["kind"], mod[ident])
    return out
