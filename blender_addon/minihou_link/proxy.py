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
_STORE_TYPES = {"float": "FLOAT", "int": "INT", "bool": "BOOLEAN", "vec3": "FLOAT_VECTOR"}
USE_NODE_PREFIX = "MH_use_"


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


def _sockets(ng, in_out: str):
    return [i for i in ng.interface.items_tree if i.item_type == "SOCKET" and i.in_out == in_out]


def _build_nodes(ng, output_obj) -> None:
    """Geometry in/out sockets plus the Object Info -> Group Output wiring. Only creates what is missing."""
    iface = ng.interface
    if not any(i.socket_type == "NodeSocketGeometry" for i in _sockets(ng, "INPUT")):
        iface.new_socket("Geometry", in_out="INPUT", socket_type="NodeSocketGeometry")
    if not any(i.socket_type == "NodeSocketGeometry" for i in _sockets(ng, "OUTPUT")):
        iface.new_socket("Geometry", in_out="OUTPUT", socket_type="NodeSocketGeometry")
    nodes = {n.bl_idname: n for n in ng.nodes}
    gin = nodes.get("NodeGroupInput") or ng.nodes.new("NodeGroupInput")
    gout = nodes.get("NodeGroupOutput") or ng.nodes.new("NodeGroupOutput")
    info = nodes.get("GeometryNodeObjectInfo") or ng.nodes.new("GeometryNodeObjectInfo")
    if "GeometryNodeObjectInfo" not in nodes:
        gin.location, info.location, gout.location = (-400, 0), (-150, -150), (150, 0)
        info.transform_space = "RELATIVE"          # engine output follows the proxy's transform, not its own
        info.inputs["As Instance"].default_value = False
    cur = info.inputs["Object"].default_value
    if cur is None or cur.name != output_obj.name:
        info.inputs["Object"].default_value = output_obj
    if not gout.inputs["Geometry"].is_linked:
        ng.links.new(info.outputs["Geometry"], gout.inputs["Geometry"])


def _active_inputs(ng):
    return [i for i in _sockets(ng, "INPUT") if i.socket_type != "NodeSocketGeometry"]


def _remove_input(ng, item) -> None:
    # Dependency order: the store node (and its link *from* this socket) first, then the socket, so there is never a
    # link to a socket that no longer exists.
    use = ng.nodes.get(USE_NODE_PREFIX + item.identifier)
    if use is not None:
        ng.nodes.remove(use)
    ng.interface.remove(item)


def _configure_socket(s, p: dict) -> None:
    """Set default/limits, writing only what differs (each write tags the depsgraph)."""
    kind = p["kind"]
    if hasattr(s, "structure_type") and s.structure_type != "SINGLE":
        s.structure_type = "SINGLE"          # a plain value: no "use attribute" toggle in the modifier panel
    elif not hasattr(s, "structure_type") and hasattr(s, "force_non_field") and not s.force_non_field:
        s.force_non_field = True             # Blender < 5.0 equivalent
    desc = p.get("doc", "")
    if s.description != desc:
        s.description = desc                 # tooltip
    want = {"default_value": bool(p["value"]) if kind == "bool" else
            (tuple(float(c) for c in p["value"]) if kind == "vec3" else p["value"])}
    if kind != "bool":
        # Blender sockets only have hard limits; the engine's hard limits map straight onto them.
        for src, dst in (("min", "min_value"), ("max", "max_value")):
            if src in p and hasattr(s, dst):
                want[dst] = p[src]
    for attr, v in want.items():
        cur = getattr(s, attr)
        cur = tuple(cur) if kind == "vec3" and attr == "default_value" else cur
        if cur != v:
            setattr(s, attr, v)


def sync_interface(ng, output_obj, params: List[dict], structure: str) -> None:
    """Make the group's inputs match the engine's exposed parameters, *incrementally*.

    Inputs whose name and type are unchanged are kept with the same identifier, so the modifier property behind them
    keeps its value -- and so do keyframes and drivers on it (they address ``modifiers["MiniHou"]["Socket_N"]``).
    Only inputs that disappeared are removed and new ones created, then all are put in the engine's order. The modifier
    picks up added/removed sockets by itself; it must not be re-assigned (that would recreate every property).
    Repeating a sync with the same parameters writes nothing (no depsgraph churn).
    """
    assert_main_thread()
    iface = ng.interface
    _build_nodes(ng, output_obj)
    wanted = {p["name"]: _SOCKET_TYPES[p["kind"]] for p in params}
    for item in _active_inputs(ng):
        if wanted.get(item.name) != item.socket_type:
            _remove_input(ng, item)
    existing = {i.name: i for i in _active_inputs(ng)}
    ids = {}
    for p in params:
        item = existing.get(p["name"]) or iface.new_socket(p["name"], in_out="INPUT", socket_type=wanted[p["name"]])
        _configure_socket(item, p)
        ids[p["name"]] = item.identifier
    for k, p in enumerate(params):                    # order: Geometry input, then the engine's order
        geo_in = next(i for i in _sockets(ng, "INPUT") if i.socket_type == "NodeSocketGeometry")
        item = next(i for i in _sockets(ng, "INPUT") if i.identifier == ids[p["name"]])
        target = geo_in.position + 1 + k
        if item.position != target:
            iface.move(item, target)
    _sync_usage_chain(ng, params, ids)
    new_map = json.dumps(ids)
    if ng.get(MAP_KEY) != new_map:
        ng[MAP_KEY] = new_map
    if ng.get(STRUCTURE_KEY) != structure:
        ng[STRUCTURE_KEY] = structure


def _sync_usage_chain(ng, params: List[dict], ids: Dict[str, str]) -> None:
    """Route every input into the output path so Blender treats it as *used*.

    The engine consumes these values, not the node tree, so unconnected inputs would be drawn greyed out ("unused")
    in the modifier panel and look disabled (seen in the GUI smoke test). Each input feeds a Store Named Attribute on
    the Instance domain, chained between Object Info and Group Output. The engine output is a plain mesh with no
    instances, so the stores write nothing (measured: ~0.08 ms per evaluation, output unchanged).
    """
    nodes = {n.bl_idname: n for n in ng.nodes}
    gin, gout, info = nodes["NodeGroupInput"], nodes["NodeGroupOutput"], nodes["GeometryNodeObjectInfo"]
    wanted = [USE_NODE_PREFIX + ids[p["name"]] for p in params]
    for n in [n for n in ng.nodes if n.name.startswith(USE_NODE_PREFIX)]:
        if n.name not in wanted:
            ng.nodes.remove(n)
    chain = []
    for k, (p, name) in enumerate(zip(params, wanted)):
        node = ng.nodes.get(name)
        if node is not None and node.data_type != _STORE_TYPES[p["kind"]]:
            ng.nodes.remove(node)
            node = None
        if node is None:
            node = ng.nodes.new("GeometryNodeStoreNamedAttribute")
            node.name = name
            node.data_type = _STORE_TYPES[p["kind"]]
            node.domain = "INSTANCE"
        label = f"use {p['name']}"
        if node.label != label:
            node.label = label
            node.inputs["Name"].default_value = "minihou_" + p["name"]
        loc = (0.0 + 180.0 * k, -320.0)
        if tuple(node.location) != loc:
            node.location = loc
        src = next(o for o in gin.outputs if o.identifier == ids[p["name"]])
        value_in = node.inputs["Value"]
        if not (value_in.is_linked and value_in.links[0].from_socket == src):
            ng.links.new(src, value_in)
        chain.append(node)
    sockets = [info.outputs["Geometry"]] + [n.outputs["Geometry"] for n in chain]
    targets = [n.inputs["Geometry"] for n in chain] + [gout.inputs["Geometry"]]
    for src, dst in zip(sockets, targets):
        if not (dst.is_linked and dst.links[0].from_socket == src):
            ng.links.new(src, dst)
    if gout.location.x < 200.0 + 180.0 * len(chain):
        gout.location = (200.0 + 180.0 * len(chain), 0.0)


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
