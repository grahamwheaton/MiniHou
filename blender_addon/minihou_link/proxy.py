"""Scene objects and the Geometry Nodes proxy. Main thread only.

* ``MiniHou_Output`` -- a hidden mesh object whose mesh the mesh writer fills with engine output.
* ``MiniHou_Proxy``  -- the object users work with. It carries a Geometry Nodes modifier whose
  node group reads the output object through an *Object Info* node. The group's inputs mirror the
  exposed engine parameters (they appear as ordinary modifier sliders).

The helpers here also build stack-modifier and wired-placement groups. Those use a nested layout (see
``ensure_nested``): the generated nodes live in a private inner "Plumbing" tree, so every node operation goes through
``plumbing(ng)`` while interface sockets and custom properties stay on the outer group. Since 0.2.1 the live-link proxy
(``ensure_node_group``) uses the same nested layout; its flat groups from older add-ons migrate in place.
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
IDS_KEY = "minihou_input_ids"

_SOCKET_TYPES = {"float": "NodeSocketFloat", "int": "NodeSocketInt", "bool": "NodeSocketBool",
                 "vec3": "NodeSocketVector", "vector": "NodeSocketVector", "color": "NodeSocketColor",
                 "rotation": "NodeSocketRotation", "matrix": "NodeSocketMatrix", "string": "NodeSocketString",
                 "menu": "NodeSocketMenu", "object": "NodeSocketObject", "collection": "NodeSocketCollection",
                 "material": "NodeSocketMaterial", "image": "NodeSocketImage", "texture": "NodeSocketTexture",
                 "geometry": "NodeSocketGeometry", "bundle": "NodeSocketBundle", "closure": "NodeSocketClosure"}
_STORE_TYPES = {"float": "FLOAT", "int": "INT", "bool": "BOOLEAN", "vec3": "FLOAT_VECTOR",
                "vector": "FLOAT_VECTOR", "color": "FLOAT_COLOR", "rotation": "QUATERNION",
                "matrix": "FLOAT4X4", "string": "STRING", "menu": "INT"}
_REFERENCE_COMPARE_TYPES = {"NodeSocketObject": "OBJECT", "NodeSocketCollection": "COLLECTION",
                            "NodeSocketMaterial": "MATERIAL", "NodeSocketImage": "IMAGE",
                            "NodeSocketTexture": "TEXTURE"}
MENU_NODE_PREFIX = "MH_menu_"
_VECTOR_SUBTYPES = {"translation": "NodeSocketVectorTranslation", "xyz": "NodeSocketVectorXYZ",
                    "euler": "NodeSocketVectorEuler", "direction": "NodeSocketVectorDirection",
                    "velocity": "NodeSocketVectorVelocity", "acceleration": "NodeSocketVectorAcceleration"}
_FLOAT_SUBTYPES = {"angle": "NodeSocketFloatAngle", "distance": "NodeSocketFloatDistance",
                   "factor": "NodeSocketFloatFactor", "percentage": "NodeSocketFloatPercentage",
                   "time": "NodeSocketFloatTime"}
_INT_SUBTYPES = {"factor": "NodeSocketIntFactor", "percentage": "NodeSocketIntPercentage"}
_STRING_SUBTYPES = {"filepath": "NodeSocketStringFilePath"}


def _socket_type(p):
    kind = p["kind"]
    subtypes = {"vector": _VECTOR_SUBTYPES, "float": _FLOAT_SUBTYPES,
                "int": _INT_SUBTYPES, "string": _STRING_SUBTYPES}.get(kind)
    if subtypes is not None:
        subtype = str(p.get("subtype", "")).lower()
        if subtype and subtype not in subtypes:
            raise ValueError(f"Unsupported {kind} subtype {subtype!r} for {p['name']}")
        return subtypes.get(subtype, _SOCKET_TYPES[kind])
    if p.get("subtype"):
        raise ValueError(f"{kind} input {p['name']!r} has no subtype")
    return _SOCKET_TYPES[p["kind"]]


def _menu_source(tree, src, identifier, options):
    """Create the Menu Switch that gives a Blender menu socket its editable choices."""
    menu = tree.nodes.get(MENU_NODE_PREFIX + identifier)
    if menu is None:
        menu = tree.nodes.new("GeometryNodeMenuSwitch")
        menu.name = MENU_NODE_PREFIX + identifier
        menu.data_type = "INT"
    options = list(options)
    if [item.name for item in menu.enum_items] != options:
        menu.enum_items.clear()
        for option in options:
            menu.enum_items.new(option)
    for index, option in enumerate(options):
        menu.inputs[option].default_value = index
    _link(tree, src, menu.inputs["Menu"])
    return menu.outputs[0]
USE_NODE_PREFIX = "MH_use_"
INST_PREFIX = "MH_inst_"
INST_KEY = "minihou_instances"
CACHE_INFO_NODE = "MH_cache_info"
OBJECT_USAGE_PREFIX = "__MiniHou_object_usage_"
# Nested layout: a modifier/placement group holds only Group Input -> one private group node -> Group Output.
# The generated plumbing (cache Object Info, object compares, "use" stores, instancing) lives in that inner tree.
# The outer group keeps the interface, the modifier values and every identity/custom property.
PLUMBING_NODE = "MH_plumbing"
PLUMBING_KEY = "minihou_plumbing"     # on the inner tree: JSON {outer input identifier: inner input identifier}
PLUMBING_LABEL = "MiniHou Plumbing"
_GENERATED = (USE_NODE_PREFIX, INST_PREFIX, OBJECT_USAGE_PREFIX, CACHE_INFO_NODE)


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

    mod = proxy.modifiers.get(MODIFIER)
    if mod is None:
        mod = proxy.modifiers.new(MODIFIER, "NODES")
    # Keep the group the modifier already uses when it is the session group (possibly renamed or made private by a
    # copy): re-pointing the modifier at another group on every update orphaned a copy per sync, and each orphan kept
    # a user on the nested plumbing tree (an ever-growing user count).
    current = mod.node_group
    ng = ensure_node_group(out, current if current is not None and SESSION_KEY in current else None)
    if mod.node_group is None or mod.node_group.name != ng.name:
        mod.node_group = ng
    return out, proxy, mod


def output_mesh():
    obj = bpy.data.objects.get(OUTPUT_OBJECT)
    return obj.data if obj else None


SESSION_KEY = "minihou_session_group"   # marks the live-link proxy's group, whatever its name


def ensure_node_group(output_obj, ng=None):
    """The session proxy's group, in the nested layout built by the same code as the Add MiniHou Modifier button.

    Flat groups from add-ons before 0.2.1 migrate in place (outer interface, modifier values and drivers kept).
    Idempotent and write-free when nothing changed.
    """
    if ng is None:
        ng = next((g for g in bpy.data.node_groups if SESSION_KEY in g and g.library is None), None) \
            or bpy.data.node_groups.get(GROUP)
    if ng is None:
        ng = bpy.data.node_groups.new(GROUP, "GeometryNodeTree")
    if SESSION_KEY not in ng:
        ng[SESSION_KEY] = 1
    if not is_nested(ng):
        if cache_info(ng) is None:
            _build_nodes(ng, output_obj)          # flat first, so the migration wires the cache (not a passthrough)
        ensure_nested(ng)
    else:
        ensure_plumbing(ng)
    info = cache_info(ng)
    if info is None:
        _build_nodes(ng, output_obj)
        info = cache_info(ng)
    cur = info.inputs["Object"].default_value
    if cur is None or cur.name != output_obj.name:
        info.inputs["Object"].default_value = output_obj
    return ng


def cache_info(ng):
    """Return the private cooked-mesh reader, never a source-object instancing node.

    Legacy groups predate its stable name; adopt only a non-instancing Object Info.
    Nested groups keep it inside their plumbing tree.
    """
    tree = plumbing(ng)
    info = tree.nodes.get(CACHE_INFO_NODE)
    if info is None:
        info = next((n for n in tree.nodes if n.bl_idname == "GeometryNodeObjectInfo"
                     and not n.name.startswith(INST_PREFIX)), None)
        if info is not None:
            info.name = CACHE_INFO_NODE
    return info


def _sockets(ng, in_out: str):
    return [i for i in ng.interface.items_tree if i.item_type == "SOCKET" and i.in_out == in_out]


# -- nested layout ---------------------------------------------------------------------------------------------------

def plumbing_node(ng):
    """The outer group's node holding the private plumbing tree, or None for a flat (legacy) group."""
    node = ng.nodes.get(PLUMBING_NODE)
    if node is not None and node.bl_idname == "GeometryNodeGroup" and node.node_tree is not None \
            and PLUMBING_KEY in node.node_tree:
        return node
    return None


def is_nested(ng) -> bool:
    return plumbing_node(ng) is not None


def plumbing(ng):
    """The tree containing the generated nodes: the inner plumbing tree, or ``ng`` itself when flat. Read-only."""
    node = plumbing_node(ng)
    return node.node_tree if node is not None else ng


def plumbing_name(ng) -> str:
    if SESSION_KEY in ng:
        return "MiniHou Session Plumbing"          # never confused with a modifier's "MiniHou Plumbing"
    return (ng.name.strip() or "MiniHou") + " Plumbing"


def _io_node(tree, idname, location):
    node = next((n for n in tree.nodes if n.bl_idname == idname), None)
    if node is None:
        node = tree.nodes.new(idname)
        node.location = location
    return node


def _mirror_interface(ng, inner) -> dict:
    """Make the inner inputs mirror the outer inputs (same order, names and types) plus one Geometry output.

    Writes only what differs. Returns {outer identifier: inner identifier}.
    """
    try:
        saved = json.loads(inner.get(PLUMBING_KEY, "{}"))
    except ValueError:
        saved = {}
    have = {i.identifier: i for i in _sockets(inner, "INPUT")}
    mapping, order = {}, []
    for src in _sockets(ng, "INPUT"):
        item = have.pop(saved.get(src.identifier, ""), None)
        if item is not None and item.socket_type != src.socket_type:
            inner.interface.remove(item)
            item = None
        if item is None:
            item = inner.interface.new_socket(src.name, in_out="INPUT", socket_type=src.socket_type)
        if item.name != src.name:
            item.name = src.name
        for attr in ("structure_type", "force_non_field"):    # plain values, like the modifier's own inputs
            if hasattr(src, attr) and hasattr(item, attr) and getattr(item, attr) != getattr(src, attr):
                try:
                    setattr(item, attr, getattr(src, attr))
                except (TypeError, AttributeError):
                    pass
        mapping[src.identifier] = item.identifier
        order.append(item)
    for stale in have.values():
        inner.interface.remove(stale)
    for k, item in enumerate(order):
        target = order[k - 1].position + 1 if k else min(i.position for i in order)
        if item.position != target:
            inner.interface.move(item, target)
    if not any(i.socket_type == "NodeSocketGeometry" for i in _sockets(inner, "OUTPUT")):
        inner.interface.new_socket("Geometry", in_out="OUTPUT", socket_type="NodeSocketGeometry")
    encoded = json.dumps(mapping, sort_keys=True)
    if inner.get(PLUMBING_KEY) != encoded:
        inner[PLUMBING_KEY] = encoded
    return mapping


def _link(tree, src, dst) -> None:
    if not (dst.is_linked and dst.links[0].from_socket == src):
        tree.links.new(src, dst)


def ensure_plumbing(ng):
    """Keep a nested group consistent: private inner tree, mirrored interface, outer wiring. Returns the plumbing tree.

    A copied outer group (duplicate modifier/object) still references the original inner tree; it gets its own copy
    here, so plumbing is never shared between groups. Flat groups are returned unchanged. Idempotent and write-free
    when nothing changed (every bpy write tags the depsgraph).
    """
    node = plumbing_node(ng)
    if node is None:
        return ng
    inner = node.node_tree
    if shared(inner):
        inner = inner.copy()
        inner.use_fake_user = False
        inner.name = plumbing_name(ng)
        node.node_tree = inner
    mapping = _mirror_interface(ng, inner)
    _io_node(inner, "NodeGroupInput", (-450, 0))
    _io_node(inner, "NodeGroupOutput", (400, 0))
    gin = _io_node(ng, "NodeGroupInput", (-350, 0))
    gout = _io_node(ng, "NodeGroupOutput", (350, 0))
    for src in gin.outputs:
        ident = mapping.get(src.identifier)
        dst = next((s for s in node.inputs if s.identifier == ident), None) if ident else None
        if dst is not None:
            _link(ng, src, dst)
    out = next((s for s in node.outputs if s.type == "GEOMETRY"), None)
    target = next((s for s in gout.inputs if s.type == "GEOMETRY"), None)
    if out is not None and target is not None:
        _link(ng, out, target)
    return inner


def ensure_nested(ng):
    """Give ``ng`` the nested layout, migrating a flat group in place. Returns the plumbing tree.

    Migration keeps the outer interface untouched (so modifier values, object pickers, keyframes and drivers stay),
    keeps every custom property on the outer group, re-points the cache Object Info at the same cache object, rebuilds
    source instancing with the same objects and preserves an uncooked pass-through. The caller re-syncs the parameter
    usage chain (``sync_interface`` / ``_sync_usage_chain``) afterwards.
    """
    assert_main_thread()
    if is_nested(ng):
        return ensure_plumbing(ng)
    iface = ng.interface
    if not any(i.socket_type == "NodeSocketGeometry" for i in _sockets(ng, "INPUT")):
        iface.new_socket("Geometry", in_out="INPUT", socket_type="NodeSocketGeometry")
    if not any(i.socket_type == "NodeSocketGeometry" for i in _sockets(ng, "OUTPUT")):
        iface.new_socket("Geometry", in_out="OUTPUT", socket_type="NodeSocketGeometry")
    info = cache_info(ng)
    cache = info.inputs["Object"].default_value if info is not None else None
    space = info.transform_space if info is not None else None
    names = json.loads(ng.get(INST_KEY, "[]"))
    objects = []
    for k in range(len(names)):
        oi = ng.nodes.get(INST_PREFIX + f"obj{k}")
        objects.append(oi.inputs["Object"].default_value if oi is not None else None)
    through = info is None or passthrough(ng)
    for node in list(ng.nodes):
        if node.name.startswith(_GENERATED):
            ng.nodes.remove(node)
    stale = ng.nodes.get(PLUMBING_NODE)            # e.g. a group node whose tree was deleted
    if stale is not None:
        ng.nodes.remove(stale)
    inner = bpy.data.node_groups.new(plumbing_name(ng), "GeometryNodeTree")
    inner[PLUMBING_KEY] = "{}"
    if hasattr(inner, "is_modifier"):
        inner.is_modifier = False                  # not offered in the modifier's group picker
    node = ng.nodes.new("GeometryNodeGroup")
    node.name = PLUMBING_NODE
    node.label = PLUMBING_LABEL
    node.node_tree = inner
    node.width = 200
    node.location = (0, 0)
    ensure_plumbing(ng)
    if cache is not None:
        _build_nodes(ng, cache)
        if space is not None:
            cache_info(ng).transform_space = space
        if names:
            ng[INST_KEY] = "[]"                    # force a rebuild inside the plumbing tree
            sync_instances(ng, names, objects=objects)
    if through:
        link_passthrough(ng)
    return inner


def passthrough(ng) -> bool:
    """True while upstream geometry is routed straight to the output (before the first successful cook)."""
    tree = plumbing(ng)
    gout = next((n for n in tree.nodes if n.bl_idname == "NodeGroupOutput"), None)
    if gout is None:
        return False
    targets = [gout.inputs["Geometry"], _chain_target(ng)]
    return any(link.from_node.bl_idname == "NodeGroupInput" for target in targets for link in target.links)


def link_passthrough(ng) -> None:
    tree = plumbing(ng)
    gin = next(n for n in tree.nodes if n.bl_idname == "NodeGroupInput")
    _link(tree, gin.outputs["Geometry"], _chain_target(ng))


# Custom properties that mark a group as MiniHou-owned (modifier, placement, plumbing, session proxy).
OWNED_KEYS = ("minihou_modifier_id", "minihou_gn_id", PLUMBING_KEY, SESSION_KEY)


def _kept(tree) -> bool:
    """A root the user keeps on purpose even when nothing evaluates it."""
    return bool(tree.use_fake_user or getattr(tree, "asset_data", None) is not None
                or getattr(tree, "is_tool", False))


def _live_trees() -> Dict[int, object]:
    """Every node group reachable from an object's modifier or a kept root, by pointer."""
    live, stack = {}, []
    for obj in bpy.data.objects:
        for mod in getattr(obj, "modifiers", ()):
            group = getattr(mod, "node_group", None)
            if group is not None:
                stack.append(group)
    stack.extend(t for t in bpy.data.node_groups if _kept(t))
    while stack:
        tree = stack.pop()
        if tree.as_pointer() in live:
            continue
        live[tree.as_pointer()] = tree
        for node in tree.nodes:
            child = getattr(node, "node_tree", None)
            if child is not None:
                stack.append(child)
    return live


def shared(tree) -> bool:
    """True when ``tree`` is really used from more than one place.

    ``ID.users`` also counts references held by orphaned groups (unassigned copies that Blender keeps until the file
    is saved and reopened), so it over-reports. Trusting it made every sync copy a "shared" group again, which
    orphaned one more copy, which raised the count again. Only references from live trees and modifiers count here.
    """
    if tree.users - int(tree.use_fake_user) <= 1:
        return False                     # cheap common case: ``users`` never under-reports
    ptr, count = tree.as_pointer(), 0
    for obj in bpy.data.objects:
        for mod in getattr(obj, "modifiers", ()):
            group = getattr(mod, "node_group", None)
            if group is not None and group.as_pointer() == ptr:
                count += 1
    for parent in _live_trees().values():
        for node in parent.nodes:
            child = getattr(node, "node_tree", None)
            if child is not None and child.as_pointer() == ptr:
                count += 1
    return count > 1


def _owned(tree, seen=None) -> bool:
    """Made by MiniHou, or a container holding a MiniHou group (e.g. a GN tree whose placements were promoted)."""
    seen = set() if seen is None else seen
    if tree.as_pointer() in seen:
        return False
    seen.add(tree.as_pointer())
    return any(key in tree for key in OWNED_KEYS) or any(
        _owned(child, seen) for child in (getattr(n, "node_tree", None) for n in tree.nodes) if child is not None)


def cleanup_plumbing() -> int:
    """Remove MiniHou-owned groups nothing live uses any more. Returns how many were removed.

    That is plumbing trees whose outer group is gone, and orphaned MiniHou groups that are only referenced by other
    orphans (left by older add-ons on each sync); removing them releases the users they held. Groups with a fake
    user, asset or tool flag are kept, as are groups not made by MiniHou.
    """
    removed = 0
    while True:
        live = _live_trees()
        dead = [t for t in bpy.data.node_groups if t.as_pointer() not in live and t.library is None
                and not _kept(t) and _owned(t)]
        if not dead:
            return removed
        if hasattr(bpy.data, "batch_remove"):
            bpy.data.batch_remove(dead)
        else:
            for tree in dead:
                bpy.data.node_groups.remove(tree)
        removed += len(dead)


def _build_nodes(ng, output_obj) -> None:
    """Geometry in/out sockets plus the Object Info -> Group Output wiring. Only creates what is missing."""
    iface = ng.interface
    if not any(i.socket_type == "NodeSocketGeometry" for i in _sockets(ng, "INPUT")):
        iface.new_socket("Geometry", in_out="INPUT", socket_type="NodeSocketGeometry")
    if not any(i.socket_type == "NodeSocketGeometry" for i in _sockets(ng, "OUTPUT")):
        iface.new_socket("Geometry", in_out="OUTPUT", socket_type="NodeSocketGeometry")
    tree = ensure_plumbing(ng)
    nodes = {n.bl_idname: n for n in tree.nodes}
    gin = nodes.get("NodeGroupInput") or tree.nodes.new("NodeGroupInput")
    gout = nodes.get("NodeGroupOutput") or tree.nodes.new("NodeGroupOutput")
    info = cache_info(ng)
    if info is None:
        info = tree.nodes.new("GeometryNodeObjectInfo")
        info.name = CACHE_INFO_NODE
        gin.location, info.location, gout.location = (-400, 0), (-150, -150), (150, 0)
        info.transform_space = "RELATIVE"          # engine output follows the proxy's transform, not its own
        info.inputs["As Instance"].default_value = False
    cur = info.inputs["Object"].default_value
    if cur is None or cur.name != output_obj.name:
        info.inputs["Object"].default_value = output_obj
    if not gout.inputs["Geometry"].is_linked:
        tree.links.new(info.outputs["Geometry"], gout.inputs["Geometry"])


def _active_inputs(ng):
    return [i for i in _sockets(ng, "INPUT") if i.name != "Geometry" and i.name != "Source Object"
            and not i.name.startswith("Object: ")]


def _remove_input(ng, item) -> None:
    # Dependency order: the store node (and its link *from* this socket) first, then the socket, so there is never a
    # link to a socket that no longer exists.
    tree = plumbing(ng)
    use = tree.nodes.get(USE_NODE_PREFIX + item.identifier)
    if use is not None:
        tree.nodes.remove(use)
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
    want = {}
    if kind not in {"geometry", "bundle", "closure"} and hasattr(s, "default_value"):
        value = p["value"]
        if kind == "bool": value = bool(value)
        elif kind in {"vec3", "vector", "rotation", "color"}: value = tuple(float(c) for c in value)
        elif kind == "matrix":
            values = list(value)
            value = tuple(tuple(float(values[r * 4 + c]) for c in range(4)) for r in range(4))
        elif kind in {"object", "collection", "material", "image", "texture"} and isinstance(value, str):
            collection = getattr(bpy.data, {"object":"objects", "collection":"collections", "material":"materials", "image":"images", "texture":"textures"}[kind])
            value = collection.get(value)
        if kind != "menu":
            want["default_value"] = value
    if kind != "bool":
        # Blender sockets only have hard limits; the engine's hard limits map straight onto them.
        for src, dst in (("min", "min_value"), ("max", "max_value")):
            if src in p and hasattr(s, dst):
                want[dst] = p[src]
    for attr, v in want.items():
        cur = getattr(s, attr)
        if cur is not None and attr == "default_value":
            if kind in {"vec3", "vector", "rotation", "color"}: cur = tuple(cur)
            elif kind == "matrix": cur = tuple(tuple(row) for row in cur)
        if cur != v:
            try:
                setattr(s, attr, v)
            except (TypeError, ValueError) as exc:
                raise ValueError(f"Blender cannot set {kind} input {p['name']!r} {attr}: {exc}") from exc


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
    for p in params:
        socket_type = _socket_type(p)
        if socket_type is None or (p["kind"] in {"bundle", "closure"} and bpy.app.version < (5, 0, 0)):
            raise ValueError(f"Blender {bpy.app.version_string} does not support {p['kind']} input sockets")
        if hasattr(bpy, "types") and hasattr(bpy.types, "NodeSocketFloat") and not hasattr(bpy.types, socket_type):
            raise ValueError(f"Blender {bpy.app.version_string} does not support {socket_type}")
    def stable(p):
        return str(p.get("node", "")) + ":" + str(p["id"]) if p.get("id") else p["name"]
    old_ids = json.loads(ng.get(IDS_KEY, "{}"))
    current = {i.identifier: i for i in _active_inputs(ng)}
    claimed = set()
    chosen = []
    for p in params:
        wanted = _socket_type(p)
        item = current.get(old_ids.get(stable(p))) or next((i for i in current.values() if i.name == p["name"]), None)
        if item is not None and item.socket_type != wanted:
            _remove_input(ng, item); item = None
        if item is None:
            item = iface.new_socket(p["name"], in_out="INPUT", socket_type=wanted)
        if item.name != p["name"]:
            item.name = p["name"]  # same identifier: modifier values, drivers and GN links survive a rename
        claimed.add(item.identifier)
        chosen.append(item)
    for item in _active_inputs(ng):
        if item.identifier not in claimed:
            _remove_input(ng, item)
    ids = {}
    for p, item in zip(params, chosen):
        _configure_socket(item, p)
        ids[p["name"]] = item.identifier
    for k, p in enumerate(params):                    # order: Geometry input, then the engine's order
        geo_in = next(i for i in _sockets(ng, "INPUT") if i.name == "Geometry")
        item = next(i for i in _sockets(ng, "INPUT") if i.identifier == ids[p["name"]])
        target = geo_in.position + 1 + k
        if item.position != target:
            iface.move(item, target)
    _sync_usage_chain(ng, params, ids)
    # Menu choices are defined by the linked Menu Switch inside the plumbing tree.
    # Only after that link exists can Blender accept a nonempty menu default.
    for p in params:
        if p["kind"] == "menu" and p["value"]:
            item = next(i for i in _sockets(ng, "INPUT") if i.identifier == ids[p["name"]])
            if item.default_value != p["value"]:
                try:
                    item.default_value = p["value"]
                except (TypeError, ValueError) as exc:
                    raise ValueError(f"Blender cannot set menu input {p['name']!r}: {exc}") from exc
    new_map = json.dumps(ids)
    if ng.get(MAP_KEY) != new_map:
        ng[MAP_KEY] = new_map
    new_ids = json.dumps({stable(p): ids[p["name"]] for p in params})
    if ng.get(IDS_KEY) != new_ids:
        ng[IDS_KEY] = new_ids
    if ng.get(STRUCTURE_KEY) != structure:
        ng[STRUCTURE_KEY] = structure


def _sync_usage_chain(ng, params: List[dict], ids: Dict[str, str]) -> None:
    """Route every input into the output path so Blender treats it as *used*.

    The engine consumes these values, not the node tree, so unconnected inputs would be drawn greyed out ("unused")
    in the modifier panel and look disabled (seen in the GUI smoke test). Each input feeds a Store Named Attribute on
    the Instance domain, chained between Object Info and Group Output. The engine output is a plain mesh with no
    instances, so the stores write nothing (measured: ~0.08 ms per evaluation, output unchanged).

    Nested groups build the chain inside their plumbing tree; node names keep the *outer* socket identifiers and the
    inner Group Input socket carrying each outer input is found through the mirrored-interface map.
    """
    tree = ensure_plumbing(ng)
    inner_ids = json.loads(tree.get(PLUMBING_KEY, "{}")) if tree is not ng else None
    nodes = {n.bl_idname: n for n in tree.nodes}
    gin, gout = nodes["NodeGroupInput"], nodes["NodeGroupOutput"]
    params = [p for p in params if p["kind"] in _STORE_TYPES]
    ids = dict(ids)
    menu_ids = {ids[p["name"]] for p in params if p["kind"] == "menu"}
    for old in [n for n in tree.nodes if n.name.startswith(MENU_NODE_PREFIX)
                and n.name[len(MENU_NODE_PREFIX):] not in menu_ids]:
        tree.nodes.remove(old)
    # Reference-only comparisons keep ID controls active without evaluating
    # Object Info geometry (which could feed this modifier back into itself).
    reference_ids = ({item.identifier: _REFERENCE_COMPARE_TYPES[item.socket_type]
                      for item in _sockets(ng, "INPUT") if item.socket_type in _REFERENCE_COMPARE_TYPES}
                     if bpy.app.version >= (5, 2, 0) else {})
    compare_prefix = OBJECT_USAGE_PREFIX
    for node in list(tree.nodes):
        if node.name.startswith(compare_prefix) and node.name[len(compare_prefix):] not in reference_ids:
            tree.nodes.remove(node)
    for identifier in sorted(reference_ids):
        params.append({"name": identifier, "kind": "bool"})
        ids[identifier] = identifier
    wanted = [USE_NODE_PREFIX + ids[p["name"]] for p in params]
    for n in [n for n in tree.nodes if n.name.startswith(USE_NODE_PREFIX)]:
        if n.name not in wanted:
            tree.nodes.remove(n)
    chain = []
    for k, (p, name) in enumerate(zip(params, wanted)):
        node = tree.nodes.get(name)
        if node is not None and node.data_type != _STORE_TYPES[p["kind"]]:
            tree.nodes.remove(node)
            node = None
        if node is None:
            node = tree.nodes.new("GeometryNodeStoreNamedAttribute")
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
        outer_ident = ids[p["name"]]
        inner_ident = inner_ids.get(outer_ident) if inner_ids is not None else outer_ident
        src = next(o for o in gin.outputs if o.identifier == inner_ident)
        if p["kind"] == "menu":
            src = _menu_source(tree, src, outer_ident, p.get("options", []))
        if outer_ident in reference_ids:
            compare = tree.nodes.get(compare_prefix + outer_ident)
            if compare is not None and compare.data_type != reference_ids[outer_ident]:
                tree.nodes.remove(compare)
                compare = None
            if compare is None:
                compare = tree.nodes.new("FunctionNodeCompare")
                compare.name = compare_prefix + outer_ident
                compare.data_type = reference_ids[outer_ident]
                compare.operation = "EQUAL"
            if not (compare.inputs["A"].is_linked and compare.inputs["A"].links[0].from_socket == src):
                tree.links.new(src, compare.inputs["A"])
            src = compare.outputs["Result"]
        value_in = node.inputs["Value"]
        if not (value_in.is_linked and value_in.links[0].from_socket == src):
            tree.links.new(src, value_in)
        chain.append(node)
    sockets = [_chain_source(ng)] + [n.outputs["Geometry"] for n in chain]
    targets = [n.inputs["Geometry"] for n in chain] + [gout.inputs["Geometry"]]
    for src, dst in zip(sockets, targets):
        if not (dst.is_linked and dst.links[0].from_socket == src):
            tree.links.new(src, dst)
    if gout.location.x < 200.0 + 180.0 * len(chain):
        gout.location = (200.0 + 180.0 * len(chain), 0.0)


def _chain_source(ng):
    """Where the parameter chain starts: the instancing join if there is one, else the Object Info output."""
    join = plumbing(ng).nodes.get(INST_PREFIX + "join")
    if join is not None:
        return join.outputs["Geometry"]
    return cache_info(ng).outputs["Geometry"]


def _chain_target(ng):
    """The socket that takes the engine geometry first (the first parameter store, or the group output)."""
    tree = plumbing(ng)
    stores = sorted((n for n in tree.nodes if n.name.startswith(USE_NODE_PREFIX)), key=lambda n: n.location.x)
    if stores:
        return stores[0].inputs["Geometry"]
    return next(n for n in tree.nodes if n.bl_idname == "NodeGroupOutput").inputs["Geometry"]


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
    tree = plumbing(ng)                      # nodes live in the plumbing tree; INST_KEY stays on the outer group
    if ng.get(INST_KEY, "[]") == key and (not names or tree.nodes.get(INST_PREFIX + "join") is not None):
        _refresh_instance_objects(ng, names, objects=resolved)
        return missing
    tree = ensure_plumbing(ng)
    for n in [n for n in tree.nodes if n.name.startswith(INST_PREFIX)]:
        tree.nodes.remove(n)
    info = cache_info(ng)
    target = _chain_target(ng)
    if names:
        def new(idname, name, loc):
            nd = tree.nodes.new(idname)
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
            tree.links.new(oi.outputs["Geometry"], to_inst.inputs["Geometry"])
        index = new("FunctionNodeIntegerMath", "index", (-150, -380))
        index.operation = "SUBTRACT"
        inst_attr = attr("instance", "INT", (-400, -300))
        tree.links.new(inst_attr, index.inputs[0])
        index.inputs[1].default_value = 1
        iop = new("GeometryNodeInstanceOnPoints", "on_points", (50, -300))
        tree.links.new(info.outputs["Geometry"], iop.inputs["Points"])
        tree.links.new(inst_attr, iop.inputs["Selection"])                 # int -> bool: instance > 0
        tree.links.new(to_inst.outputs["Instances"], iop.inputs["Instance"])
        iop.inputs["Pick Instance"].default_value = True
        tree.links.new(index.outputs[0], iop.inputs["Instance Index"])
        tree.links.new(attr("rot", "FLOAT_VECTOR", (-150, -250)), iop.inputs["Rotation"])   # Euler XYZ -> rotation
        tree.links.new(attr("scale", "FLOAT_VECTOR", (-150, -200)), iop.inputs["Scale"])
        drop = new("GeometryNodeDeleteGeometry", "drop_points", (50, -120))
        drop.domain = "POINT"
        tree.links.new(info.outputs["Geometry"], drop.inputs["Geometry"])
        tree.links.new(inst_attr, drop.inputs["Selection"])
        join = new("GeometryNodeJoinGeometry", "join", (250, -150))
        tree.links.new(drop.outputs["Geometry"], join.inputs["Geometry"])
        tree.links.new(iop.outputs["Instances"], join.inputs["Geometry"])
        tree.links.new(join.outputs["Geometry"], target)
    else:
        tree.links.new(info.outputs["Geometry"], target)
    ng[INST_KEY] = key
    return missing


def _refresh_instance_objects(ng, names: List[str], *, objects=None) -> None:
    """Re-point Object Info nodes whose object was missing (or renamed back) -- only when it differs."""
    for k, name in enumerate(names):
        oi = plumbing(ng).nodes.get(INST_PREFIX + f"obj{k}")
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
    if kind in {"vec3", "vector", "rotation", "color"}:
        return tuple(float(c) for c in v)
    if kind == "matrix":
        return tuple(float(c) for row in v for c in row) if len(v) == 4 else tuple(float(c) for c in v)
    if kind == "bool":
        return bool(v)
    if kind == "int":
        return int(v)
    if kind == "float":
        return float(v)
    if kind in {"object", "collection", "material", "image", "texture"}:
        return v.name if v is not None else ""
    if kind in {"geometry", "bundle", "closure"}:
        raise ValueError(f"{kind} input cannot be read as a uniform modifier value; connect a supported value source")
    return str(v)


def reference_metadata(params, value_of):
    """JSON-safe Blender ID identity and transforms; geometry arrays travel via instance_sources."""
    result = {}
    for p in params:
        if p["kind"] not in {"object", "collection", "material", "image", "texture"} or not p.get("id"):
            continue
        ref = value_of(p)
        key = "input:" + p["node"] + ":" + p["id"]
        entry = {"kind": p["kind"], "name": ref.name if ref is not None else ""}
        if p["kind"] == "object" and ref is not None:
            entry["matrix"] = [float(v) for row in ref.matrix_world for v in row]
        elif p["kind"] == "collection" and ref is not None:
            entry["objects"] = [{"name": obj.name,
                                  "matrix": [float(v) for row in obj.matrix_world for v in row]}
                                 for obj in ref.all_objects]
        elif p["kind"] == "image" and ref is not None:
            entry["size"] = list(ref.size)
        result[key] = entry
    return result


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
    changed = False
    for p in params:
        ident = m.get(p["name"])
        if ident is None or p["name"] not in values or not has_input(mod, ident):
            continue
        v = values[p["name"]]
        if p["kind"] in {"geometry", "bundle", "closure"}:
            continue
        if not same_value(canonical(p["kind"], get_input(mod, ident)), v):
            if p["kind"] in {"object", "collection", "material", "image", "texture"} and isinstance(v, str):
                v = getattr(bpy.data, {"object":"objects", "collection":"collections", "material":"materials", "image":"images", "texture":"textures"}[p["kind"]]).get(v)
            if p["kind"] == "matrix":
                v = tuple(tuple(v[r * 4 + c] for c in range(4)) for r in range(4))
            set_input(mod, ident, tuple(v) if p["kind"] in {"vec3", "vector", "rotation", "color"} else v)
            changed = True
    if changed:
        mod.id_data.update_tag()


def read_values(mod, ng, params: List[dict]) -> Dict[str, object]:
    assert_main_thread()
    m = mapping(ng)
    out = {}
    for p in params:
        ident = m.get(p["name"])
        if ident is None or not has_input(mod, ident):
            continue
        if p["kind"] in {"geometry", "bundle", "closure"}:
            continue
        out[p["name"]] = canonical(p["kind"], get_input(mod, ident))
    return out


def same_value(a, b):
    """Compare using Blender's storage precision, including vector components."""
    import struct
    if isinstance(a, (list, tuple)) and isinstance(b, (list, tuple)):
        return len(a) == len(b) and all(same_value(x, y) for x, y in zip(a, b))
    if isinstance(a, float) or isinstance(b, float):
        try:
            return struct.pack("f", a) == struct.pack("f", b)
        except (OverflowError, TypeError):
            return a == b
    return a == b
