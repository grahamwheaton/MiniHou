"""Interactive stack bridge: upstream prefix -> isolated engine instance -> GN cache.

Blender has no Python mesh modifier callback. A main-thread timer evaluates a disposable
copy containing only the preceding stack, then publishes the result to this modifier's
private Object Info cache. This is real upstream geometry, not the original datablock.
Updates are interactive/eventual; call pump(force=True) before deterministic still renders.
Animation/background rendering needs an explicit per-frame pump (not a render handler).
"""
from __future__ import annotations

import hashlib
import json
import os
import time
import uuid

import bpy
from bpy.app.handlers import persistent

from . import mesh_reader, mesh_writer, proxy
from .protocol import GeoUpdate, Mirror

KEY = "minihou_modifier_id"
GRAPH = "minihou_graph"
PATH = "minihou_graph_path"
PARAMS = "minihou_parameters"
STATUS = "minihou_status"
CACHE = "minihou_cache_id"
COLLECTION = "MiniHou Modifier Caches"
_client = None
_endpoint = None
_owners = {}
_digests = {}
_failures = {}
_registered = False
_rendering = False
_busy = False
_retry_at = 0.0
_last_error = ""


def is_modifier(mod):
    return mod.type == "NODES" and mod.node_group is not None and KEY in mod.node_group


def instance_id(mod):
    return str(mod.node_group[KEY]) if is_modifier(mod) else ""


def graph_path(mod):
    return str(mod.node_group.get(PATH, ""))


def status(mod):
    return str(mod.node_group.get(STATUS, "Waiting for first cook"))


def parameters(mod):
    return json.loads(mod.node_group.get(PARAMS, "[]"))


def _settings(host=None, port=None):
    wm = bpy.context.window_manager
    return (host or getattr(wm, "minihou_host", "127.0.0.1"),
            int(port if port is not None else getattr(wm, "minihou_port", 8765)))


def _connection(host=None, port=None):
    global _client, _endpoint
    endpoint = _settings(host, port)
    if _client is None or endpoint != _endpoint:
        disconnect()
        from .instance_client import InstanceClient
        _client = InstanceClient(*endpoint, timeout=5.0)
        _client.connect()
        _endpoint = endpoint
    return _client


def disconnect():
    global _client, _endpoint
    if _client is not None:
        _client.close()
    _client, _endpoint = None, None
    _digests.clear()
    _failures.clear()


def _collection():
    col = bpy.data.collections.get(COLLECTION)
    if col is None:
        col = bpy.data.collections.new(COLLECTION)
    if col.name not in bpy.context.scene.collection.children:
        bpy.context.scene.collection.children.link(col)
    return col


def _cache(mod):
    ident = instance_id(mod)
    ng = mod.node_group
    info = next((n for n in ng.nodes if n.bl_idname == "GeometryNodeObjectInfo"), None)
    obj = info.inputs["Object"].default_value if info else None
    if obj is not None and obj.get(CACHE) == ident:
        return obj
    mesh = bpy.data.meshes.new("MiniHou Cache " + ident[:8])
    obj = bpy.data.objects.new(mesh.name, mesh)
    obj[CACHE] = ident
    _collection().objects.link(obj)
    obj.hide_render = True
    obj.hide_set(True)
    proxy._build_nodes(ng, obj)
    info = next(n for n in ng.nodes if n.bl_idname == "GeometryNodeObjectInfo")
    info.transform_space = "ORIGINAL"  # cooked arrays are object local, independent of object transforms
    return obj


def _interface(mod, header, initial=False, activate=False):
    ng = mod.node_group
    gout = next((n for n in ng.nodes if n.bl_idname == "NodeGroupOutput"), None)
    passthrough = initial or (gout is not None and any(
        link.from_node.bl_idname == "NodeGroupInput" for link in gout.inputs["Geometry"].links))
    old = proxy.read_values(mod, ng, parameters(mod)) if not initial else {}
    params = header["interface"]
    cache = _cache(mod)
    proxy.sync_interface(ng, cache, params, json.dumps([(p["name"], p["kind"]) for p in params]))
    ng[PARAMS] = json.dumps(params)
    values = {p["name"]: old.get(p["name"], p["value"]) for p in params}
    proxy.write_values(mod, ng, params, values)
    # sync_interface uses the proxy's relative cache convention; stack caches always stay local.
    next(n for n in ng.nodes if n.bl_idname == "GeometryNodeObjectInfo").transform_space = "ORIGINAL"
    if passthrough and not activate:
        gin = next(n for n in ng.nodes if n.bl_idname == "NodeGroupInput")
        gout = next(n for n in ng.nodes if n.bl_idname == "NodeGroupOutput")
        ng.links.new(gin.outputs["Geometry"], gout.inputs["Geometry"])


def add_modifier(obj, graph_path, *, host=None, port=None):
    """Add an independent graph instance to a mesh. No source mesh is written."""
    mesh_writer.assert_main_thread()
    if obj.type != "MESH" or obj.library is not None:
        raise ValueError("Select a local editable mesh object")
    path = os.path.abspath(bpy.path.abspath(graph_path))
    header = _connection(host, port).describe(path)
    mod = obj.modifiers.new("MiniHou", "NODES")
    ng = bpy.data.node_groups.new("MiniHou " + os.path.basename(path), "GeometryNodeTree")
    ng[KEY], ng[PATH] = uuid.uuid4().hex, path
    ng[GRAPH] = json.dumps(header["graph"])
    ng[PARAMS] = "[]"
    mod.node_group = ng
    _interface(mod, header, initial=True)
    # Until the first successful cook, pass upstream geometry through unchanged.
    gin = next(n for n in ng.nodes if n.bl_idname == "NodeGroupInput")
    gout = next(n for n in ng.nodes if n.bl_idname == "NodeGroupOutput")
    ng.links.new(gin.outputs["Geometry"], gout.inputs["Geometry"])
    ng[STATUS] = "Waiting for first cook"
    _owners[instance_id(mod)] = (obj.as_pointer(), mod.as_pointer())
    return mod


def reload_graph(mod, *, host=None, port=None):
    """Explicitly adopt the external editor's saved graph, preserving matching parameter values."""
    header = _connection(host, port).describe(graph_path(mod))
    mod.node_group[GRAPH] = json.dumps(header["graph"])
    _interface(mod, header)
    _digests.pop(instance_id(mod), None)
    _failures.pop(instance_id(mod), None)


def _unique(entries):
    """Duplicate modifiers and objects get private groups, caches and UUIDs, preserving the original."""
    seen = set()
    entries.sort(key=lambda pair: _owners.get(instance_id(pair[1])) !=
                 (pair[0].as_pointer(), pair[1].as_pointer()))
    for obj, mod in entries:
        ident = instance_id(mod)
        owner = (obj.as_pointer(), mod.as_pointer())
        if ident in seen:
            old_cache = _cache(mod)
            ng = mod.node_group.copy()
            mod.node_group = ng
            ident = uuid.uuid4().hex
            ng[KEY] = ident
            cache = _cache(mod)
            empty = cache.data
            cache.data = old_cache.data.copy()
            bpy.data.meshes.remove(empty)
            ng[STATUS] = "Duplicated instance; waiting for cook"
        seen.add(ident)
        _owners[ident] = owner
    return seen


def _check_prefix_dependencies(obj, preceding):
    """Reject known evaluated-output cycles instead of repeatedly cooking feedback.

    Cross-object MiniHou dependencies require scheduling the complete object graph and
    are deliberately unsupported. Ordinary independent native modifier inputs are fine.
    """
    def check_target(target):
        if isinstance(target, bpy.types.Collection):
            for member in target.all_objects:
                check_target(member)
        if isinstance(target, bpy.types.Object):
            if target == obj or CACHE in target or any(is_modifier(m) for m in target.modifiers):
                raise ValueError("upstream modifier references a MiniHou output or itself; "
                                 "cross-object MiniHou dependencies are unsupported (feedback prevention)")

    visited = set()
    def check_tree(tree):
        if tree.as_pointer() in visited:
            return
        visited.add(tree.as_pointer())
        for node in tree.nodes:
            for socket in node.inputs:
                check_target(getattr(socket, "default_value", None))
            nested = getattr(node, "node_tree", None)
            if nested is not None:
                check_tree(nested)

    for modifier in preceding:
        if is_modifier(modifier) or not modifier.show_viewport:
            continue  # an earlier private MiniHou cache is the intended upstream result
        for prop in modifier.bl_rna.properties:
            if prop.type == "POINTER" and prop.identifier not in {"rna_type", "id_data", "node_group"}:
                check_target(getattr(modifier, prop.identifier, None))
        if modifier.type == "NODES" and modifier.node_group is not None:
            check_tree(modifier.node_group)
            for item in proxy._sockets(modifier.node_group, "INPUT"):
                if item.socket_type in {"NodeSocketObject", "NodeSocketCollection"}:
                    if proxy.has_input(modifier, item.identifier):
                        check_target(proxy.get_input(modifier, item.identifier))


def read_prefix(obj, mod, *, with_materials=False):
    """Read evaluated geometry immediately BEFORE mod without toggling the real object's stack."""
    index = list(obj.modifiers).index(mod)
    _check_prefix_dependencies(obj, list(obj.modifiers)[:index])
    temp = obj.copy()  # mesh datablock is read-only and shared; modifiers belong to this disposable copy
    temp.name = "__MiniHou_prefix__"
    temp.hide_render = True
    temp.hide_viewport = False
    try:
        for suffix in list(temp.modifiers)[index:]:
            temp.modifiers.remove(suffix)
        bpy.context.scene.collection.objects.link(temp)
        temp.hide_set(False)
        bpy.context.view_layer.update()
        evaluated = temp.evaluated_get(bpy.context.evaluated_depsgraph_get())
        mesh = evaluated.to_mesh()
        try:
            arrays, _ = mesh_reader._read_mesh(temp, mesh)
            materials = tuple(material.original if material is not None else None
                              for material in mesh.materials)
        finally:
            evaluated.to_mesh_clear()
        return (arrays, materials) if with_materials else arrays
    finally:
        bpy.data.objects.remove(temp, do_unlink=True)


def _digest(mod, arrays, values, materials=()):
    ng = mod.node_group
    scene = bpy.context.scene
    digest = hashlib.sha256()
    digest.update(ng[GRAPH].encode("utf-8"))
    digest.update(json.dumps(values, sort_keys=True).encode("utf-8"))
    # Material assignments can change without any geometry/index array changing.
    digest.update(str(tuple(m.as_pointer() if m is not None else 0 for m in materials)).encode("ascii"))
    digest.update(str((scene.frame_current, scene.frame_subframe, scene.render.fps, scene.render.fps_base)).encode("ascii"))
    for name, arr in sorted(arrays.items()):
        digest.update(name.encode("utf-8"))
        digest.update(str((arr.dtype.str, arr.shape)).encode("ascii"))
        digest.update(arr.tobytes())
    return digest.hexdigest()


def _cleanup(live):
    for ident in set(_owners) - live:
        if _client is not None:
            _client.remove(ident)
        _owners.pop(ident, None)
        _digests.pop(ident, None)
        _failures.pop(ident, None)
    # Do not destroy caches of objects in other scenes or data-only objects.
    used = {instance_id(m) for o in bpy.data.objects for m in o.modifiers if is_modifier(m)}
    for obj in list(bpy.data.objects):
        if CACHE in obj and obj[CACHE] not in used:
            mesh = obj.data
            bpy.data.objects.remove(obj, do_unlink=True)
            if mesh.users == 0:
                bpy.data.meshes.remove(mesh)
    for ng in list(bpy.data.node_groups):
        if KEY in ng and ng.users == 0:
            bpy.data.node_groups.remove(ng)


def pump(host=None, port=None, *, force=False):
    """Synchronously refresh visible instances in stack order. Returns number of engine cooks.

    All Blender access occurs here on the main thread. Content hashes suppress engine feedback.
    A failure retains the last successful cache and exposes the error through status(mod).
    """
    global _busy, _last_error, _retry_at
    mesh_writer.assert_main_thread()
    if _rendering or _busy or (not force and time.monotonic() < _retry_at):
        return 0
    _busy = True
    cooked = 0
    errors = []
    try:
        entries = [(o, m) for o in bpy.context.scene.objects if o.type == "MESH" and CACHE not in o
                   and o.library is None for m in o.modifiers if is_modifier(m)]
        all_entries = [(o, m) for o in bpy.data.objects if o.type == "MESH" and CACHE not in o
                       and o.library is None for m in o.modifiers if is_modifier(m)]
        live = _unique(all_entries)  # copied objects in another scene also need fresh identities
        _cleanup(live)
        if not entries:
            return 0
        client = _connection(host, port)
        # Walk each object's actual stack after identity repair (which deliberately reorders entries).
        for obj in dict.fromkeys(o for o, _ in entries):
            if obj.mode == "EDIT":
                for mod in obj.modifiers:
                    if is_modifier(mod):
                        mod.node_group[STATUS] = "Edit mode; using last cache"
                continue
            upstream_error = None
            for mod in obj.modifiers:
                if not is_modifier(mod) or not mod.show_viewport:
                    continue
                ng = mod.node_group
                if upstream_error:
                    ng[STATUS] = "Waiting: preceding MiniHou instance failed"
                    continue
                signature = None
                try:
                    arrays, materials = read_prefix(obj, mod, with_materials=True)
                    values = proxy.read_values(mod, ng, parameters(mod))
                    signature = _digest(mod, arrays, values, materials)
                    ident = instance_id(mod)
                    failed = _failures.get(ident)
                    if not force and failed and failed[0] == signature:
                        upstream_error = failed[1]
                        errors.append(upstream_error)
                        continue
                    if not force and _digests.get(ident) == signature:
                        if status(mod) != "Ready":
                            ng[STATUS] = "Ready"
                        continue
                    scene = bpy.context.scene
                    header, output = client.cook(ident, graph_path(mod), values, arrays,
                                                  frame=scene.frame_current + scene.frame_subframe,
                                                  fps=scene.render.fps / scene.render.fps_base,
                                                  graph=json.loads(ng[GRAPH]))
                    update = GeoUpdate.from_message(header, output)
                    mirror = Mirror()
                    mirror.apply(update)
                    cache = _cache(mod)
                    # Native upstream modifiers may add/reorder slots; indices refer to that prefix.
                    # Establish slots before assigning polygon material indices.
                    cache.data.materials.clear()
                    for material in materials:
                        cache.data.materials.append(material)
                    mesh_writer.apply_update(cache.data, mirror, update)
                    _interface(mod, header, activate=True)
                    bpy.context.view_layer.update()
                    _digests[ident] = signature
                    ng[STATUS] = "Ready"
                    _failures.pop(ident, None)
                    cooked += 1
                except Exception as exc:
                    ng[STATUS] = f"Error (last cache retained): {exc}"
                    upstream_error = str(exc)
                    errors.append(upstream_error)
                    if isinstance(exc, (OSError, ConnectionError)):
                        raise
                    _failures[instance_id(mod)] = (signature, upstream_error)
        _last_error = "; ".join(dict.fromkeys(errors))
        _retry_at = 0.0
    except Exception as exc:
        _last_error = str(exc)
        _retry_at = time.monotonic() + 2.0
        disconnect()
        for obj, mod in locals().get("entries", []):
            if status(mod) == "Ready" or status(mod).startswith("Waiting"):
                mod.node_group[STATUS] = f"Error (last cache retained): {exc}"
    finally:
        _busy = False
    return cooked


def last_error():
    return _last_error


def _timer():
    if not _registered:
        return None
    pump()
    return 0.2


@persistent
def _render_pre(_dummy):
    global _rendering
    _rendering = True


@persistent
def _render_done(_dummy):
    global _rendering
    _rendering = False


@persistent
def _load_post(_dummy):
    _render_done(None)
    _owners.clear()
    disconnect()


def register():
    global _registered
    _registered = True
    for handlers, callback in ((bpy.app.handlers.render_init, _render_pre),
                               (bpy.app.handlers.render_complete, _render_done),
                               (bpy.app.handlers.render_cancel, _render_done)):
        if callback not in handlers:
            handlers.append(callback)
    if _load_post not in bpy.app.handlers.load_post:
        bpy.app.handlers.load_post.append(_load_post)
    if not bpy.app.timers.is_registered(_timer):
        bpy.app.timers.register(_timer, first_interval=0.2, persistent=True)


def unregister():
    global _registered
    _registered = False
    _render_done(None)
    for handlers, callback in ((bpy.app.handlers.render_init, _render_pre),
                               (bpy.app.handlers.render_complete, _render_done),
                               (bpy.app.handlers.render_cancel, _render_done)):
        if callback in handlers:
            handlers.remove(callback)
    if bpy.app.timers.is_registered(_timer):
        bpy.app.timers.unregister(_timer)
    if _load_post in bpy.app.handlers.load_post:
        bpy.app.handlers.load_post.remove(_load_post)
    _owners.clear()
    disconnect()
