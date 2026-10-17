"""Interactive stack bridge: upstream prefix -> isolated engine instance -> GN cache.

Blender has no Python mesh modifier callback. A main-thread timer evaluates a disposable
copy containing only the preceding stack, then publishes the result to this modifier's
private Object Info cache. This is real upstream geometry, not the original datablock.
Updates are interactive/eventual, except RBD timeline frames which synchronously pump.
Call cook_frame before scripted renders, or bake an RBD mesh sequence for native rendering.
"""
from __future__ import annotations

import hashlib
import json
import os
import time
import uuid

import bpy
from bpy.app.handlers import persistent

from . import embed, mesh_reader, mesh_writer, proxy, scene_objects
from .protocol import GeoUpdate, Mirror, instance_payload

KEY = "minihou_modifier_id"
GRAPH = "minihou_graph"
PATH = "minihou_graph_path"
PARAMS = "minihou_parameters"
STATUS = "minihou_status"
CACHE = "minihou_cache_id"
COLLECTION = "MiniHou Modifier Caches"
SOURCE = "Source Object"
SOURCE_TOKEN = "@source"
USAGE_VERSION = 3  # 2: object-usage compares; 3: plumbing nested in a private group (proxy.ensure_nested)
_client = None
_live_ids = set()
_source_digests = {}
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


class SourceBindingError(ValueError):
    """A live Blender dependency may recover without any engine input changing."""


def _source_socket(mod):
    return next((item for item in proxy._sockets(mod.node_group, "INPUT")
                 if item.socket_type == "NodeSocketObject" and item.name == SOURCE), None)


def _ensure_source(mod):
    item = _source_socket(mod)
    if item is None:
        item = mod.node_group.interface.new_socket(SOURCE, in_out="INPUT", socket_type="NodeSocketObject")
        if hasattr(item, "hide_in_modifier"):
            item.hide_in_modifier = True
        item.description = "Blender object instanced by @source; local geometry around its origin, including native modifiers and materials"
    if hasattr(item, "hide_in_modifier"):
        item.hide_in_modifier = True
    return item


def source_owner(mod):
    item = _source_socket(mod)
    return proxy.input_owner(mod, item.identifier) if item else None


def get_source(mod):
    item = _source_socket(mod)
    return proxy.get_input(mod, item.identifier) if item and proxy.has_input(mod, item.identifier) else None


def set_source(mod, obj):
    item = _ensure_source(mod)
    proxy.set_input(mod, item.identifier, obj)
    mod.id_data.update_tag()


def _validate_source(owner, source):
    """Check transitive native dependencies before connecting a live Object Info node.

    Cross-object MiniHou evaluation is deliberately unsupported: conservative rejection
    avoids timer feedback, including cycles hidden behind a native GN group or collection.
    """
    from . import geonodes
    seen, trees, driver_ids = set(), set(), set()
    forbidden_ids = {owner.data.as_pointer()}
    forbidden_ids.update(obj.data.as_pointer() for obj in bpy.data.objects
                         if obj.data is not None and (CACHE in obj or geonodes.CACHE in obj or obj.name == proxy.OUTPUT_OBJECT))
    forbidden_ids.update(group.as_pointer() for group in bpy.data.node_groups
                         if KEY in group or geonodes.KEY in group or proxy.PLUMBING_KEY in group)

    def drivers(value):
        if value is None or not isinstance(value, bpy.types.ID):
            return
        ptr = value.as_pointer()
        if ptr in driver_ids:
            return
        driver_ids.add(ptr)
        animation = getattr(value, "animation_data", None)
        for curve in animation.drivers if animation else ():
            for variable in curve.driver.variables:
                for binding in variable.targets:
                    target(binding.id)
        drivers(getattr(value, "shape_keys", None))
        nested = getattr(value, "node_tree", None)
        if nested is not None:
            tree(nested)

    def target(value):
        if isinstance(value, bpy.types.ID) and value.as_pointer() in forbidden_ids:
            raise SourceBindingError("Source Object driver references destination or MiniHou cache data (cycle prevention)")
        if isinstance(value, bpy.types.Collection):
            for obj in value.all_objects:
                target(obj)
        elif isinstance(value, bpy.types.Object):
            if value == owner or CACHE in value or geonodes.CACHE in value or value.name == proxy.OUTPUT_OBJECT:
                raise SourceBindingError("Source Object references itself or a MiniHou cache (cycle prevention)")
            ptr = value.as_pointer()
            if ptr in seen:
                return
            seen.add(ptr)
            if any(is_modifier(m) or geonodes.contains(getattr(m, "node_group", None)) for m in value.modifiers):
                raise SourceBindingError("Source Object depends on MiniHou output; cross-object MiniHou dependencies are unsupported")
            drivers(value)
            drivers(value.data)
            if value.instance_collection:
                target(value.instance_collection)
            for modifier in value.modifiers:
                if not modifier.show_viewport:
                    continue
                for prop in modifier.bl_rna.properties:
                    if prop.type == "POINTER" and prop.identifier not in {"rna_type", "id_data", "node_group"}:
                        target(getattr(modifier, prop.identifier, None))
                if modifier.type == "NODES" and modifier.node_group:
                    tree(modifier.node_group)
                    for item in proxy._sockets(modifier.node_group, "INPUT"):
                        if item.socket_type in {"NodeSocketObject", "NodeSocketCollection"} and proxy.has_input(modifier, item.identifier):
                            target(proxy.get_input(modifier, item.identifier))
            # Constraints and parenting may depend on the destination as well.
            if value.parent:
                target(value.parent)
            for constraint in value.constraints:
                target(getattr(constraint, "target", None))
        elif isinstance(value, bpy.types.ID):
            drivers(value)

    def tree(group):
        if group.as_pointer() in trees:
            return
        trees.add(group.as_pointer())
        drivers(group)
        for node in group.nodes:
            for socket in node.inputs:
                target(getattr(socket, "default_value", None))
            nested = getattr(node, "node_tree", None)
            if nested:
                tree(nested)

    if source is None:
        raise SourceBindingError("Select a Source Object for this modifier (@source binding is missing)")
    if source.type != "MESH":
        raise SourceBindingError("Source Object must be a mesh")
    target(source)


def _object_keys(mod):
    keys = []
    for nid, node in json.loads(mod.node_group[GRAPH]).get("nodes", {}).items():
        params = node.get("params", {})
        if node.get("type") == "object_input":
            keys.append(params.get("binding") or nid)
        if node.get("type") == "instance":
            keys.extend(n[1:] for n in params.get("objects", "").replace(",", " ").split() if n.startswith("@"))
    return list(dict.fromkeys(keys))


def _object_socket(mod, key):
    if key == "source":
        return _ensure_source(mod)
    ng = mod.node_group
    saved = json.loads(ng.get("minihou_object_sockets", "{}"))
    item = next((s for s in proxy._sockets(ng, "INPUT") if s.socket_type == "NodeSocketObject"
                 and (s.identifier == saved.get(key) or s.name == "Object: " + key)), None)
    graph = json.loads(ng[GRAPH])
    label = next((n.get("input_label") for nid, n in graph.get("nodes", {}).items()
                  if n.get("type") == "object_input" and (n.get("params", {}).get("binding") or nid) == key
                  and n.get("input_label")), None)
    if item is None:
        item = ng.interface.new_socket("Object: " + key, in_out="INPUT", socket_type="NodeSocketObject")
    if item.name != (label or "Object: " + key):
        item.name = label or "Object: " + key
    if item.description != "MiniHou object input: " + key:
        item.description = "MiniHou object input: " + key
    saved[key] = item.identifier
    encoded = json.dumps(saved)
    if ng.get("minihou_object_sockets") != encoded:
        ng["minihou_object_sockets"] = encoded
    if hasattr(item, "hide_in_modifier"):
        item.hide_in_modifier = True
    return item


def get_object(mod, key):
    item = _object_socket(mod, key)
    return proxy.get_input(mod, item.identifier) if proxy.has_input(mod, item.identifier) else None


def set_object(mod, key, obj):
    proxy.set_input(mod, _object_socket(mod, key).identifier, obj)
    mod.id_data.update_tag()


def _source_meshes(obj, mod, materials=None):
    sources = {}
    inputs = [(key, get_object(mod, key)) for key in _object_keys(mod)]
    for p in parameters(mod):
        if p["kind"] not in {"object", "collection"} or not p.get("id"):
            continue
        ident = proxy.mapping(mod.node_group).get(p["name"])
        if ident is None or not proxy.has_input(mod, ident):
            continue
        ref = proxy.get_input(mod, ident)
        key = "input:" + p["node"] + ":" + p["id"]
        if p["kind"] == "object":
            inputs.append((key, ref))
        elif ref is not None:
            inputs.extend((key + ":" + member.name, member) for member in ref.all_objects)
    for key, source in inputs:
        if source is None:
            continue
        _validate_source(obj, source)
        _, arrays = mesh_reader.read_object(source, bpy.context.evaluated_depsgraph_get())
        if materials is not None:
            import numpy as np
            slots = source.evaluated_get(bpy.context.evaluated_depsgraph_get()).data.materials
            mapping = []
            for material in slots:
                material = material.original if material else None
                if material not in materials:
                    materials.append(material)
                mapping.append(materials.index(material))
            if mapping and "prim/material_index" in arrays:
                indices = arrays["prim/material_index"]
                arrays["prim/material_index"] = np.asarray(mapping, dtype=np.int32)[np.clip(indices, 0, len(mapping) - 1)]
        sources[key] = arrays
    return sources


def _input_references(mod):
    def value_of(p):
        ident = proxy.mapping(mod.node_group).get(p["name"])
        return proxy.get_input(mod, ident) if ident is not None and proxy.has_input(mod, ident) else None
    return proxy.reference_metadata(parameters(mod), value_of)


def _send_sources(client, obj, mod, materials=None):
    sources = _source_meshes(obj, mod, materials)
    digest = hashlib.blake2b(digest_size=16)
    for name, arrays in sources.items():
        digest.update(name.encode())
        for key, array in sorted(arrays.items()):
            digest.update(key.encode()); digest.update(str((array.dtype, array.shape)).encode()); digest.update(array.tobytes())
    token = digest.hexdigest()
    ident = instance_id(mod)
    if _source_digests.get(ident) != token:
        render_uv = next((layer.name for layer in obj.data.uv_layers if layer.active_render), None)
        revision = client.sync_materials(ident, materials, render_uv) if materials is not None else None
        client.set_sources(ident, sources, materials=materials, manifest_revision=revision)
        _source_digests[ident] = token
    return token


def _instance_objects(obj, mod, names):
    objects = []
    for name in names:
        if not name.startswith("@"):
            raise SourceBindingError("Stack instance objects must use @source or another modifier object input")
        source = get_object(mod, name[1:])
        _validate_source(obj, source)
        objects.append(source)
    return objects


def _disconnect_unsafe_sources(obj, mod):
    """Break a newly introduced native cycle without discarding cached engine points.

    A bad pending picker selection must not disconnect a still-valid previous source.
    Only previously connected sources which are now themselves unsafe are detached.
    """
    for node in proxy.plumbing(mod.node_group).nodes:
        if not node.name.startswith(proxy.INST_PREFIX + "obj") or node.bl_idname != "GeometryNodeObjectInfo":
            continue
        source = node.inputs["Object"].default_value
        if source is None:
            continue
        try:
            _validate_source(obj, source)
        except SourceBindingError:
            node.inputs["Object"].default_value = None


def _settings(host=None, port=None):
    wm = bpy.context.window_manager
    addon = bpy.context.preferences.addons.get(__package__)
    if host is None and port is None and addon and not addon.preferences.auto_connect:
        return addon.preferences.manual_host, addon.preferences.manual_port
    return (host or getattr(wm, "minihou_host", "127.0.0.1"),
            int(port if port is not None else getattr(wm, "minihou_port", 8765)))


def _connection(host=None, port=None):
    global _client, _endpoint
    addon = bpy.context.preferences.addons.get(__package__)
    automatic = host is None and port is None and (not addon or addon.preferences.auto_connect)
    required = {"live_modifier_v1", "object_inputs_v1"}
    if automatic and _client is not None and _client.sock is not None:
        if required.issubset(_client.welcome.get("capabilities", [])):
            return _client
        disconnect()
    endpoint = _settings(host, port)
    if _client is None or endpoint != _endpoint or _client.sock is None:
        disconnect()
        from .instance_client import InstanceClient
        from .discovery import candidates
        choices = list(dict.fromkeys(candidates() + [endpoint])) if automatic else [endpoint]
        for address in choices:
            client = InstanceClient(*address, timeout=0.4 if automatic else 30.0)
            try:
                header = client.connect()
                if automatic and not required.issubset(header.get("capabilities", [])):
                    raise RuntimeError("This engine needs updating")
            except (OSError, RuntimeError, ValueError):
                client.close()
                if not automatic:
                    raise
                continue
            client.sock.settimeout(30.0)
            client.timeout = 30.0
            _client, _endpoint = client, address
            break
        if _client is None:
            raise RuntimeError("Open the updated MiniHou app, then try again. Blender connects automatically.")
    return _client


def disconnect():
    global _client, _endpoint
    if _client is not None:
        _client.close()
    _client, _endpoint = None, None
    _live_ids.clear()
    _source_digests.clear()
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
    info = proxy.cache_info(ng)
    obj = info.inputs["Object"].default_value if info else None
    if obj is not None and obj.get(CACHE) == ident:
        return obj
    # A lost reader node (e.g. rebuilt plumbing) must not orphan this instance's existing cache.
    obj = next((o for o in bpy.data.objects if o.get(CACHE) == ident and o.type == "MESH"), None)
    if obj is not None:
        proxy._build_nodes(ng, obj)
        proxy.cache_info(ng).transform_space = "ORIGINAL"
        return obj
    mesh = bpy.data.meshes.new("MiniHou Cache " + ident[:8])
    obj = bpy.data.objects.new(mesh.name, mesh)
    obj[CACHE] = ident
    _collection().objects.link(obj)
    obj.hide_render = True
    obj.hide_set(True)
    proxy._build_nodes(ng, obj)
    info = proxy.cache_info(ng)
    info.transform_space = "ORIGINAL"  # cooked arrays are object local, independent of object transforms
    return obj


def _interface(mod, header, initial=False, activate=False):
    ng = mod.node_group
    passthrough = initial or proxy.passthrough(ng)
    # Plumbing lives in one private nested group; flat groups from older add-ons migrate here.
    proxy.ensure_nested(ng)
    previous = parameters(mod) if not initial else []
    old = proxy.read_values(mod, ng, previous) if not initial else {}
    old_ids = {p["node"] + ":" + p["id"]: old[p["name"]] for p in previous
               if p.get("id") and p.get("node") and p["name"] in old}
    params = header["interface"]
    cache = _cache(mod)
    proxy.sync_interface(ng, cache, params, json.dumps([(p["name"], p["kind"]) for p in params]))
    _ensure_source(mod)
    keys = _object_keys(mod)
    saved = json.loads(ng.get("minihou_object_sockets", "{}"))
    for key in list(saved):
        if key not in keys:
            item = next((s for s in proxy._sockets(ng, "INPUT") if s.identifier == saved[key]), None)
            if item is not None:
                ng.interface.remove(item)
            del saved[key]
    ng["minihou_object_sockets"] = json.dumps(saved)
    for key in keys:
        _object_socket(mod, key)
    proxy._sync_usage_chain(ng, params, json.loads(ng.get(proxy.MAP_KEY, "{}")))
    if ng.get("minihou_object_usage_version") != USAGE_VERSION:
        ng["minihou_object_usage_version"] = USAGE_VERSION
    ng[PARAMS] = json.dumps(params)
    values = {p["name"]: old_ids.get(p.get("node", "") + ":" + p.get("id", ""),
              old.get(p["name"], p["value"])) for p in params}
    proxy.write_values(mod, ng, params, values)
    # sync_interface uses the proxy's relative cache convention; stack caches always stay local.
    proxy.cache_info(ng).transform_space = "ORIGINAL"
    if passthrough and not activate:
        proxy.link_passthrough(ng)


def _apply_live(mod, header):
    if not header.get("live"):
        _live_ids.discard(instance_id(mod))
        return
    ng = mod.node_group
    data = json.dumps(header["graph"])
    values = {p["name"]: p["value"] for p in header["interface"]}
    current = proxy.read_values(mod, ng, parameters(mod))
    if ng[GRAPH] != data or any(not proxy.same_value(current.get(k), v) for k, v in values.items()):
        graph_changed = ng[GRAPH] != data
        ng[GRAPH] = data
        _interface(mod, header)
        proxy.write_values(mod, ng, parameters(mod), values)
        _digests.pop(instance_id(mod), None)
        _failures.pop(instance_id(mod), None)
        if graph_changed:   # an edit made in MiniHou: the embedded copy follows (debounced)
            embed.note_graph(ng, pending=bool(header.get("embed_pending")))
    if ng.get(PATH) != header["path"]:
        ng[PATH] = header["path"]


def edit_live(obj, mod, *, push=True):
    """Bind actual pre-modifier geometry to the editor without using a legacy proxy."""
    client = _connection()
    ng = mod.node_group
    if push and embed.mode(ng) == embed.EMBEDDED:
        # graph_embed_v1: the app binds a document to this network; a conflicting open
        # document or concurrent edit is answered in MiniHou's dialog (polled by pump()).
        if embed.push(client, ng, obj.name + " / " + mod.name) == "conflict":
            raise embed.ConflictPending("MiniHou is asking how to combine your open work with this "
                                        "embedded network; answer in the MiniHou window")
    arrays, materials = read_prefix(obj, mod, with_materials=True)
    materials = list(materials)
    scene = bpy.context.scene
    _send_sources(client, obj, mod, materials)
    render_uv = next((layer.name for layer in obj.data.uv_layers if layer.active_render), None)
    manifest_revision = client.sync_materials(instance_id(mod), materials, render_uv)
    header = client.edit_live(instance_id(mod), json.loads(mod.node_group[GRAPH]),
                              proxy.read_values(mod, mod.node_group, parameters(mod)), arrays,
                              label=obj.name + " / " + mod.name,
                              frame=scene.frame_current + scene.frame_subframe,
                              fps=scene.render.fps / scene.render.fps_base,
                              references=_input_references(mod), materials=materials,
                              manifest_revision=manifest_revision)
    _live_ids.add(instance_id(mod))
    _apply_live(mod, header)
    return {"state": "ready", "editor_url": client.welcome.get("editor_url", ""),
            "native_live": header.get("live") and header.get("editor") == "native"}, header["path"]


def add_modifier(obj, graph_path, *, host=None, port=None):
    """Add an independent graph instance to a mesh. No source mesh is written."""
    mesh_writer.assert_main_thread()
    if obj.type != "MESH" or obj.library is not None:
        raise ValueError("Select a local editable mesh object")
    path = os.path.abspath(bpy.path.abspath(graph_path)) if graph_path.strip() else ""
    if path and not path.lower().endswith((".json", ".minihou")):
        raise ValueError("Graph must name a .minihou or .json file; leave Graph empty to start a new modifier")
    starter = {"nodes": {"input": {"type": "modifier_input"},
                           "transform": {"type": "transform", "inputs": {"geo": "input"}},
                           "output": {"type": "output", "inputs": {"geo": "transform"}}},
               "display": "output", "exposed": [
                   {"name": "Translate", "node": "transform", "param": "translate"},
                   {"name": "Scale", "node": "transform", "param": "uniform_scale"},
                   {"name": "Rotate", "node": "transform", "param": "rotate"}]}
    header = _connection(host, port).describe(path, None if path else starter)
    mod = obj.modifiers.new("MiniHou", "NODES")
    ng = bpy.data.node_groups.new("MiniHou " + os.path.basename(path), "GeometryNodeTree")
    ng[KEY], ng[PATH] = uuid.uuid4().hex, path
    ng[GRAPH] = json.dumps(header["graph"])
    ng[PARAMS] = "[]"
    mod.node_group = ng
    proxy.ensure_nested(ng)
    _interface(mod, header, initial=True)
    # Until the first successful cook, pass upstream geometry through unchanged.
    proxy.link_passthrough(ng)
    ng[STATUS] = "Waiting for first cook"
    _owners[instance_id(mod)] = (obj.as_pointer(), mod.as_pointer())
    # 0.3.0: new networks live inside the .blend; a source file is remembered for comparison/extract.
    try:
        embed.embed(ng, label=obj.name + " " + mod.name, file=path or None)
    except embed.EmbedError as exc:
        ng[embed.ERROR] = f"Kept in Linked File mode: {exc}"
    return mod


def reload_graph(mod, *, host=None, port=None):
    """Explicitly adopt the saved graph (embedded text or linked file), preserving matching parameter values."""
    ng = mod.node_group
    embedded = embed.mode(ng) == embed.EMBEDDED
    path = embed.linked_file(ng) if embedded else (embed.linked_file(ng) or graph_path(mod))
    if embedded and not (path and os.path.exists(path)):
        header = _connection(host, port).describe("", embed.read(ng)[0])   # no file: re-read the .blend copy
    else:
        header = _connection(host, port).describe(path)
    mod.node_group[GRAPH] = json.dumps(header["graph"])
    if embedded:
        embed.write(ng)   # the reloaded file becomes the embedded copy
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
            old_source = get_source(mod)
            old_values = proxy.read_values(mod, mod.node_group, parameters(mod))
            ng = mod.node_group.copy()
            mod.node_group = ng
            proxy.ensure_plumbing(ng)  # the copy still references the original's plumbing tree
            ident = uuid.uuid4().hex
            ng[KEY] = ident
            set_source(mod, old_source)
            proxy.write_values(mod, ng, parameters(mod), old_values)
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
    from . import geonodes
    def check_target(target):
        if isinstance(target, bpy.types.Collection):
            for member in target.all_objects:
                check_target(member)
        if isinstance(target, bpy.types.Object):
            if target == obj or CACHE in target or geonodes.CACHE in target or any(
                    is_modifier(m) or geonodes.contains(getattr(m, "node_group", None)) for m in target.modifiers):
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
        if is_modifier(modifier) or geonodes.contains(getattr(modifier, "node_group", None)) or not modifier.show_viewport:
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
    realize_group = None
    try:
        for suffix in list(temp.modifiers)[index:]:
            temp.modifiers.remove(suffix)
        # to_mesh() excludes unrealized GN instances. Realize only this disposable
        # prefix read; the user's actual output continues to instance shared sources.
        realize_group = bpy.data.node_groups.new("__MiniHou_prefix_realize__", "GeometryNodeTree")
        realize_group.interface.new_socket("Geometry", in_out="INPUT", socket_type="NodeSocketGeometry")
        realize_group.interface.new_socket("Geometry", in_out="OUTPUT", socket_type="NodeSocketGeometry")
        gin = realize_group.nodes.new("NodeGroupInput")
        gout = realize_group.nodes.new("NodeGroupOutput")
        realize = realize_group.nodes.new("GeometryNodeRealizeInstances")
        realize_group.links.new(gin.outputs["Geometry"], realize.inputs["Geometry"])
        realize_group.links.new(realize.outputs["Geometry"], gout.inputs["Geometry"])
        bpy.context.scene.collection.objects.link(temp)
        temp.modifiers.new("MiniHou prefix realization", "NODES").node_group = realize_group
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
        if realize_group is not None and realize_group.users == 0:
            bpy.data.node_groups.remove(realize_group)


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
            embed.release(ng)  # a deleted modifier's embedded network goes with it
            bpy.data.node_groups.remove(ng)
    proxy.cleanup_plumbing()  # after the outer groups, whose removal releases them
    embed.repair()  # duplicated networks get their own text; orphaned texts are removed
    # Wired GN placements own scene sinks just like stack modifiers.
    gn_owners = {ng.get("minihou_gn_id") for ng in bpy.data.node_groups if ng.get("minihou_gn_id")}
    scene_objects.retain_owners(set(live) | gn_owners)  # removed owners are parked, never deleted


def pump(host=None, port=None, *, force=False, _only=None, render=False):
    """Synchronously refresh visible instances in stack order. Returns number of engine cooks.

    All Blender access occurs here on the main thread. Content hashes suppress engine feedback.
    A failure retains the last successful cache and exposes the error through status(mod).
    """
    global _busy, _last_error, _retry_at
    if mesh_writer.modal_ui_active():
        return 0
    if _only is None:
        from . import geonodes
        for obj in bpy.data.objects:
            if obj.type == "MESH" and obj.library is None and geonodes.CACHE not in obj:
                for mod in obj.modifiers:
                    if mod.type == "NODES":
                        geonodes._promote_wired_modifier(mod)
        geonodes._cleanup()
        embed.repair()
        if any(geonodes.contains(getattr(m, "node_group", None))
               for o in bpy.context.scene.objects for m in o.modifiers):
            count = geonodes.pump(host, port, force=force)
            _last_error = geonodes.last_error()
            return count
    mesh_writer.assert_main_thread()
    if _rendering or _busy or (not force and time.monotonic() < _retry_at):
        return 0
    _busy = True
    cooked = 0
    errors = []
    try:
        entries = [(o, m) for o in bpy.context.scene.objects if o.type == "MESH" and CACHE not in o
                   and o.library is None and not (o.hide_get() and o.hide_render)
                   for m in o.modifiers if is_modifier(m)]
        all_entries = [(o, m) for o in bpy.data.objects if o.type == "MESH" and CACHE not in o
                       and o.library is None for m in o.modifiers if is_modifier(m)]
        for _, modifier in all_entries:
            # Older layouts (flat plumbing, pre-object-usage) migrate in place; values and pickers are kept.
            if (modifier.node_group.get("minihou_object_usage_version", 0) < USAGE_VERSION
                    or not proxy.is_nested(modifier.node_group)):
                _interface(modifier, {"interface": parameters(modifier)})
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
                if _only is not None and (obj, mod) != _only:
                    continue
                if not is_modifier(mod) or not mod.show_viewport:
                    continue
                ng = mod.node_group
                if upstream_error:
                    ng[STATUS] = "Waiting: preceding MiniHou instance failed"
                    continue
                signature = None
                try:
                    if ng.get(embed.STATE) == "conflict":
                        # graph_embed_v1: the user answers in MiniHou; then open (or stay closed).
                        resolution = embed.poll(client, ng)
                        if resolution and resolution != "keep_mine" and instance_id(mod) not in _live_ids:
                            edit_live(obj, mod, push=False)  # already answered; do not ask again
                    if instance_id(mod) in _live_ids:
                        _apply_live(mod, client.sync_live(instance_id(mod), proxy.read_values(mod, ng, parameters(mod))))
                    if has_rbd(mod):
                        if (obj.animation_data or obj.data.animation_data or
                                (obj.data.shape_keys and obj.data.shape_keys.animation_data)):
                            raise ValueError('RBD requires static input: animated objects/shape keys/drivers are unsupported')
                        for preceding in obj.modifiers:
                            if preceding == mod:
                                break
                            if preceding.type in {'CLOTH', 'SOFT_BODY', 'FLUID', 'PARTICLE_SYSTEM', 'MESH_SEQUENCE_CACHE'}:
                                raise ValueError('RBD does not accept deforming/time-dependent upstream modifiers')
                            if preceding.type == 'NODES' and _animated_nodes(preceding.node_group):
                                raise ValueError('RBD does not accept animated/simulation Geometry Nodes upstream')
                    # Break invalid active native dependencies before evaluating the prefix.
                    # A changed graph may no longer instance anything, so its old binding
                    # must not prevent cooking and validating the new output.
                    _disconnect_unsafe_sources(obj, mod)
                    arrays, materials = read_prefix(obj, mod, with_materials=True)
                    materials = list(materials)
                    values = proxy.read_values(mod, ng, parameters(mod))
                    source_token = ""
                    graph_data = json.loads(ng[GRAPH])
                    has_imports = any(n.get("type") in {"object_input", "blender_object"} or
                                      (n.get("type") == "modifier_input" and any(
                                          p.get("kind") in {"object", "collection"} for p in n.get("modifier_inputs", [])))
                                      for n in graph_data.get("nodes", {}).values())
                    if instance_id(mod) in _live_ids or has_imports:
                        source_token = _send_sources(client, obj, mod, materials)
                    signature = _digest(mod, arrays, values, materials) + source_token + (":render" if render else ":display")
                    ident = instance_id(mod)
                    failed = _failures.get(ident)
                    selected_source = get_source(mod)
                    render_uv = next((layer.name for layer in obj.data.uv_layers if layer.active_render), None)
                    manifest_revision = client.sync_materials(ident, materials, render_uv)
                    source_pointer = selected_source.as_pointer() if selected_source else 0
                    if not force and failed and failed[0] == signature and failed[2] == source_pointer:
                        upstream_error = failed[1]
                        errors.append(upstream_error)
                        continue
                    if not force and _digests.get(ident) == signature:
                        # Live geometry/material edits stay native; only pointer changes
                        # need re-binding when engine inputs have not changed.
                        existing_names = json.loads(ng.get(proxy.INST_KEY, "[]"))
                        existing_objects = _instance_objects(obj, mod, existing_names)
                        if existing_names:
                            proxy.sync_instances(ng, existing_names, objects=existing_objects)
                        if status(mod) != "Ready":
                            ng[STATUS] = "Ready"
                        continue
                    scene = bpy.context.scene
                    header, output = client.cook(ident, graph_path(mod), values, arrays,
                                                  frame=scene.frame_current + scene.frame_subframe,
                                                  fps=scene.render.fps / scene.render.fps_base,
                                                  graph=json.loads(ng[GRAPH]),
                                                  materials=[m.name if m is not None else "" for m in materials], render=render,
                                                  manifest_revision=manifest_revision,
                                                  references=_input_references(mod))
                    output, manifests = instance_payload(header, output)
                    update = GeoUpdate.from_message(header, output)
                    names = update.detail.get("instance_objects", [])
                    objects = _instance_objects(obj, mod, names)
                    mirror = Mirror()
                    mirror.apply(update)
                    cache = _cache(mod)
                    # Native upstream modifiers may add/reorder slots; indices refer to that prefix.
                    # Establish slots before assigning polygon material indices.
                    cache.data.materials.clear()
                    for material in materials:
                        cache.data.materials.append(material)
                    try:
                        mesh_writer.apply_update(cache.data, mirror, update)
                    finally:
                        if getattr(update, "transport", "tcp") == "shm":
                            client.ack(update.seq, getattr(update, "shm_slot", 0))
                            output.clear()
                            update.arrays.clear()
                    _interface(mod, header, activate=True)
                    proxy.sync_instances(ng, names, objects=objects)
                    # scene_objects nodes: real objects, parented to this object. Only queued here (pump can run from a
                    # frame-change handler); the timer makes them.
                    for manifest in manifests:
                        scene_objects.queue(ident, manifest, parent=obj.name)
                    for e in header.get("scene_errors", []):
                        scene_objects._log(f"scene_objects {e.get('node')!r} on {obj.name}: {e.get('message')}")
                    scene_objects.queue_nodes(ident, [m["node"] for m in header.get("scene", [])]
                                              + [e["node"] for e in header.get("scene_errors", [])])
                    bpy.context.view_layer.update()
                    _digests[ident] = signature
                    ng[STATUS] = "Ready"
                    _failures.pop(ident, None)
                    cooked += 1
                except Exception as exc:
                    if isinstance(exc, SourceBindingError):
                        _disconnect_unsafe_sources(obj, mod)
                    ng[STATUS] = f"Error (last cache retained): {exc}"
                    upstream_error = str(exc)
                    errors.append(upstream_error)
                    if isinstance(exc, (OSError, ConnectionError)):
                        raise
                    selected_source = get_source(mod)
                    _failures[instance_id(mod)] = (None if isinstance(exc, SourceBindingError) else signature, upstream_error,
                                                   selected_source.as_pointer() if selected_source else 0)
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
    scene_objects.flush()
    return 0.2


@persistent
def _render_pre(_dummy):
    global _rendering
    _rendering = True


@persistent
def _render_done(_dummy):
    global _rendering
    _rendering = False


def has_rbd(mod):
    """Saved graph inspection without importing the engine into Blender."""
    if not is_modifier(mod):
        return False
    try:
        graph = json.loads(mod.node_group.get(GRAPH, '{}'))
    except (ValueError, TypeError):
        return False
    return any(n.get('type') == 'rbd_solver' for n in graph.get('nodes', {}).values())


def _animated_nodes(group, seen=None):
    if group is None:
        return False
    seen = set() if seen is None else seen
    if group.as_pointer() in seen:
        return False
    seen.add(group.as_pointer())
    return bool(group.animation_data) or any(
        n.bl_idname in {'GeometryNodeInputSceneTime', 'GeometryNodeSimulationInput', 'GeometryNodeSimulationOutput'}
        or _animated_nodes(getattr(n, 'node_tree', None), seen) for n in group.nodes)


def cook_frame(scene=None, frame=None, *, render=True):
    """Explicit synchronous barrier before a still render. Never mutate during rendering."""
    if _rendering:
        raise RuntimeError('MiniHou cannot cook during a render; bake first or cook each frame before rendering')
    scene = scene or bpy.context.scene
    if frame is not None:
        scene.frame_set(int(frame))
    host, port = _endpoint or _settings()
    count = pump(host, port, force=True, render=render)
    if last_error():
        raise RuntimeError(last_error())
    bpy.context.view_layer.update()
    return count


@persistent
def _frame_post(scene, _depsgraph=None):
    if _busy or _rendering:
        return
    if any(has_rbd(m) and m.show_viewport for o in scene.objects if o.type == 'MESH' and not o.hide_get()
           for m in o.modifiers):
        host, port = _endpoint or _settings()
        pump(host, port)


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
    if _frame_post not in bpy.app.handlers.frame_change_post:
        bpy.app.handlers.frame_change_post.append(_frame_post)
    if not bpy.app.timers.is_registered(_timer):
        bpy.app.timers.register(_timer, first_interval=0.2, persistent=True)
    embed.register()


def unregister():
    embed.unregister()
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
    if _frame_post in bpy.app.handlers.frame_change_post:
        bpy.app.handlers.frame_change_post.remove(_frame_post)
    _owners.clear()
    disconnect()
