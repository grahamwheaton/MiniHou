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
INST_PREFIX = "MH_inst_"
INST_KEY = "minihou_instances"
CACHE_INFO_NODE = "MH_cache_info"


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
        info = cache_info(ng)
        if info is not None:
            cur = info.inputs["Object"].default_value
            if cur is None or cur.name != output_obj.name:
                info.inputs["Object"].default_value = output_obj
    return ng


def cache_info(ng):
    """Return the private cooked-mesh reader, never a source-object instancing node.

    Legacy groups predate its stable name; adopt only a non-instancing Object Info.
    """
    info = ng.nodes.get(CACHE_INFO_NODE)
    if info is None:
        info = next((n for n in ng.nodes if n.bl_idname == "GeometryNodeObjectInfo"
                     and not n.name.startswith(INST_PREFIX)), None)
        if info is not None:
            info.name = CACHE_INFO_NODE
    return info


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
    info = cache_info(ng)
    if info is None:
        info = ng.nodes.new("GeometryNodeObjectInfo")
        info.name = CACHE_INFO_NODE
        gin.location, info.location, gout.location = (-400, 0), (-150, -150), (150, 0)
        info.transform_space = "RELATIVE"          # engine output follows the proxy's transform, not its own
        info.inputs["As Instance"].default_value = False
    cur = info.inputs["Object"].default_value
    if cur is None or cur.name != output_obj.name:
        info.inputs["Object"].default_value = output_obj
    if not gout.inputs["Geometry"].is_linked:
        ng.links.new(info.outputs["Geometry"], gout.inputs["Geometry"])


def _active_inputs(ng):
    return [i for i in _sockets(ng, "INPUT") if i.socket_type not in {"NodeSocketGeometry", "NodeSocketObject"}]


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
    keeps its value -- and so do keyframes and drivers on it (they address the identifier; see ``input_data_path``).
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
    gin, gout = nodes["NodeGroupInput"], nodes["NodeGroupOutput"]
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
    sockets = [_chain_source(ng)] + [n.outputs["Geometry"] for n in chain]
    targets = [n.inputs["Geometry"] for n in chain] + [gout.inputs["Geometry"]]
    for src, dst in zip(sockets, targets):
        if not (dst.is_linked and dst.links[0].from_socket == src):
            ng.links.new(src, dst)
    if gout.location.x < 200.0 + 180.0 * len(chain):
        gout.location = (200.0 + 180.0 * len(chain), 0.0)


def _chain_source(ng):
    """Where the parameter chain starts: the instancing join if there is one, else the Object Info output."""
    join = ng.nodes.get(INST_PREFIX + "join")
    if join is not None:
        return join.outputs["Geometry"]
    return cache_info(ng).outputs["Geometry"]


def _chain_target(ng):
    """The socket that takes the engine geometry first (the first parameter store, or the group output)."""
    stores = sorted((n for n in ng.nodes if n.name.startswith(USE_NODE_PREFIX)), key=lambda n: n.location.x)
    if stores:
        return stores[0].inputs["Geometry"]
    return next(n for n in ng.nodes if n.bl_idname == "NodeGroupOutput").inputs["Geometry"]


def sync_instances(ng, names: List[str], *, objects=None) -> List[str]:
    """Instance Blender objects on the engine's instance points (``instance`` > 0, ``rot``, ``scale``).

    Builds, once per object list: Object Info (as instance) per object -> Geometry to Instance -> Instance on Points
    (pick by ``instance`` - 1), joined with the rest of the geometry (minus the instance points). Nothing in the user's
    scene changes. Returns the names of objects that do not exist (they place nothing). Writes nothing when the list
    is unchanged.

    ``objects`` optionally supplies already validated Blender object references in the same order as
    ``names``. Stack modifiers use this to bind persistent object pickers; legacy proxy calls keep
    name-based resolution. References are applied only on this explicit successful cook path, so an
    invalid pending picker selection cannot replace valid output or create a GN dependency cycle.
    Source geometry is read live in ORIGINAL space: its mesh coordinates (including evaluated native
    modifiers and materials) retain the local pivot; source object location, rotation and scale do not
    offset the scattered copies. Point rot/scale attributes control copy transforms.
    """
    assert_main_thread()
    if objects is not None and len(objects) != len(names):
        raise ValueError("Instance object references must match instance_objects length")
    resolved = list(objects) if objects is not None else [bpy.data.objects.get(n) for n in names]
    key = json.dumps(list(names))
    missing = [n for n, obj in zip(names, resolved) if obj is None]
    if ng.get(INST_KEY, "[]") == key and (not names or ng.nodes.get(INST_PREFIX + "join") is not None):
        _refresh_instance_objects(ng, names, objects=resolved)
        return missing
    for n in [n for n in ng.nodes if n.name.startswith(INST_PREFIX)]:
        ng.nodes.remove(n)
    info = cache_info(ng)
    target = _chain_target(ng)
    if names:
        def new(idname, name, loc):
            nd = ng.nodes.new(idname)
            nd.name = nd.label = INST_PREFIX + name
            nd.location = loc
            return nd

        def attr(name, dtype, loc):
            nd = new("GeometryNodeInputNamedAttribute", "attr_" + name, loc)
            nd.data_type = dtype
            nd.inputs["Name"].default_value = name
            return nd.outputs["Attribute"]

        to_inst = new("GeometryNodeGeometryToInstance", "objects", (-150, -520))
        for k, name in enumerate(names):
            oi = new("GeometryNodeObjectInfo", f"obj{k}", (-400, -450 - 160 * k))
            oi.transform_space = "ORIGINAL"
            oi.inputs["As Instance"].default_value = True
            oi.inputs["Object"].default_value = resolved[k]
            ng.links.new(oi.outputs["Geometry"], to_inst.inputs["Geometry"])
        index = new("FunctionNodeIntegerMath", "index", (-150, -380))
        index.operation = "SUBTRACT"
        inst_attr = attr("instance", "INT", (-400, -300))
        ng.links.new(inst_attr, index.inputs[0])
        index.inputs[1].default_value = 1
        iop = new("GeometryNodeInstanceOnPoints", "on_points", (50, -300))
        ng.links.new(info.outputs["Geometry"], iop.inputs["Points"])
        ng.links.new(inst_attr, iop.inputs["Selection"])                 # int -> bool: instance > 0
        ng.links.new(to_inst.outputs["Instances"], iop.inputs["Instance"])
        iop.inputs["Pick Instance"].default_value = True
        ng.links.new(index.outputs[0], iop.inputs["Instance Index"])
        ng.links.new(attr("rot", "FLOAT_VECTOR", (-150, -250)), iop.inputs["Rotation"])   # Euler XYZ -> rotation
        ng.links.new(attr("scale", "FLOAT_VECTOR", (-150, -200)), iop.inputs["Scale"])
        drop = new("GeometryNodeDeleteGeometry", "drop_points", (50, -120))
        drop.domain = "POINT"
        ng.links.new(info.outputs["Geometry"], drop.inputs["Geometry"])
        ng.links.new(inst_attr, drop.inputs["Selection"])
        join = new("GeometryNodeJoinGeometry", "join", (250, -150))
        ng.links.new(drop.outputs["Geometry"], join.inputs["Geometry"])
        ng.links.new(iop.outputs["Instances"], join.inputs["Geometry"])
        ng.links.new(join.outputs["Geometry"], target)
    else:
        ng.links.new(info.outputs["Geometry"], target)
    ng[INST_KEY] = key
    return missing


def _refresh_instance_objects(ng, names: List[str], *, objects=None) -> None:
    """Re-point Object Info nodes whose object was missing (or renamed back) -- only when it differs."""
    for k, name in enumerate(names):
        oi = ng.nodes.get(INST_PREFIX + f"obj{k}")
        if oi is None:
            continue
        want = objects[k] if objects is not None else bpy.data.objects.get(name)
        cur = oi.inputs["Object"].default_value
        if cur != want:
            oi.inputs["Object"].default_value = want


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


# Modifier input values. Up to Blender 5.1 they are ID properties on the modifier (``mod["Socket_2"]``); Blender 5.2
# removed ID properties from modifiers and exposes them as RNA (``mod.properties.inputs.Socket_2.value``).

def input_owner(mod, ident: str):
    """``(struct, property)`` holding the input's value (for ``layout.prop``), or None if the modifier lacks it."""
    props = getattr(mod, "properties", None)
    if props is not None and hasattr(props, "inputs"):
        item = getattr(props.inputs, ident, None)
        return (item, "value") if item is not None else None
    return (mod, f'["{ident}"]') if ident in mod.keys() else None


def has_input(mod, ident: str) -> bool:
    return input_owner(mod, ident) is not None


def get_input(mod, ident: str):
    owner = input_owner(mod, ident)
    if owner is None:
        raise KeyError(ident)
    return owner[0].value if owner[1] == "value" else mod[ident]


def set_input(mod, ident: str, value) -> None:
    owner = input_owner(mod, ident)
    if owner is None:
        raise KeyError(ident)
    if owner[1] == "value":
        owner[0].value = value
    else:
        mod[ident] = list(value) if isinstance(value, (tuple, list)) else value


def input_data_path(mod, ident: str) -> str:
    """Animation data path of the input relative to the object (keyframes, drivers)."""
    owner = input_owner(mod, ident)
    if owner is None:
        raise KeyError(ident)
    return owner[0].path_from_id(owner[1]) if owner[1] == "value" else f'{mod.path_from_id()}["{ident}"]'


def write_values(mod, ng, params: List[dict], values: Dict[str, object]) -> None:
    assert_main_thread()
    m = mapping(ng)
    for p in params:
        ident = m.get(p["name"])
        if ident is None or p["name"] not in values or not has_input(mod, ident):
            continue
        v = values[p["name"]]
        set_input(mod, ident, tuple(v) if p["kind"] == "vec3" else v)
    mod.id_data.update_tag()


def read_values(mod, ng, params: List[dict]) -> Dict[str, object]:
    assert_main_thread()
    m = mapping(ng)
    out = {}
    for p in params:
        ident = m.get(p["name"])
        if ident is None or not has_input(mod, ident):
            continue
        out[p["name"]] = canonical(p["kind"], get_input(mod, ident))
    return out
