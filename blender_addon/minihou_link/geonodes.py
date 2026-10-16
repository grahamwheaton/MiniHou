"""Staged, wired-input Geometry Nodes bridge. No Python geometry execution callback.

Native input subgraphs are evaluated by Blender on disposable copies, then cooked by
the external engine in dependency order. Persistent private meshes publish results.
Call pump(force=True) after scene changes and before rendering. See docs/GEONODES.md.
"""
from __future__ import annotations

import hashlib
import json
import time
import uuid
from types import SimpleNamespace

import bpy
from bpy.app.handlers import persistent

from . import modifiers, proxy, mesh_reader, mesh_writer, scene_objects
from .protocol import GeoUpdate, Mirror, instance_payload

KEY = "minihou_gn_id"
CACHE = "minihou_gn_cache"
_digests = {}
_busy = False
_last_error = ""
_retry_at = 0.0
_known = set()
_live = set()
_source_sent = {}


def is_group(node):
    return node.bl_idname == "GeometryNodeGroup" and node.node_tree is not None and (KEY in node.node_tree or modifiers.KEY in node.node_tree)


def contains(tree, seen=None):
    if tree is None:
        return False
    seen = set() if seen is None else seen
    if tree.as_pointer() in seen:
        return False
    seen.add(tree.as_pointer())
    return KEY in tree or modifiers.KEY in tree or any(contains(getattr(n, "node_tree", None), seen) for n in tree.nodes)


def status(node):
    return node.node_tree.get(modifiers.STATUS, "Waiting for first cook")


def _cache(tree):
    info = proxy.cache_info(tree)
    old = info.inputs["Object"].default_value if info else None
    if old is not None and old.get(CACHE) == tree[KEY]:
        return old
    mesh = old.data.copy() if old is not None else bpy.data.meshes.new("MiniHou GN result")
    obj = bpy.data.objects.new("MiniHou GN " + tree[KEY][:8], mesh)
    obj[CACHE] = tree[KEY]
    modifiers._collection().objects.link(obj)
    obj.hide_render = True
    obj.hide_set(True)
    proxy._build_nodes(tree, obj)
    proxy.cache_info(tree).transform_space = "ORIGINAL"
    return obj


def add_group(tree, graph_path, *, host=None, port=None):
    """Insert a reusable graph placement; caller connects its Geometry sockets."""
    import os
    path = os.path.abspath(bpy.path.abspath(graph_path))
    header = modifiers._connection(host, port).describe(path)
    ng = bpy.data.node_groups.new("MiniHou - " + os.path.basename(path), "GeometryNodeTree")
    ng[KEY] = uuid.uuid4().hex
    ng[modifiers.PATH] = path
    ng[modifiers.GRAPH] = json.dumps(header["graph"])
    ng[modifiers.PARAMS] = json.dumps(header["interface"])
    ng[modifiers.STATUS] = "Waiting for first cook (empty output)"
    proxy.ensure_nested(ng)  # plumbing in one private nested group, like stack modifiers
    cache = _cache(ng)
    proxy.sync_interface(ng, cache, header["interface"], json.dumps(header["interface"]))
    node = tree.nodes.new("GeometryNodeGroup")
    node.node_tree = ng
    node.label = "MiniHou: " + os.path.basename(path)
    node.width = 240
    _object_interface(ng)
    return node


def reload_graph(node, *, host=None, port=None):
    tree = node.node_tree
    header = modifiers._connection(host, port).describe(tree[modifiers.PATH])
    old = {s.name: s.default_value[:] if s.type == "VECTOR" else s.default_value
           for s in node.inputs if hasattr(s, "default_value")}
    proxy.sync_interface(tree, _cache(tree), header["interface"], json.dumps(header["interface"]))
    for s in node.inputs:
        if s.name in old and hasattr(s, "default_value"):
            s.default_value = old[s.name]
    tree[modifiers.GRAPH] = json.dumps(header["graph"])
    tree[modifiers.PARAMS] = json.dumps(header["interface"])
    _object_interface(tree)
    _digests.pop(tree[KEY], None)


def _object_interface(tree):
    """Use reference comparisons, never Object Info, to activate safe picker sockets."""
    adapter = SimpleNamespace(node_group=tree)
    keys = modifiers._object_keys(adapter)
    saved = json.loads(tree.get("minihou_object_sockets", "{}"))
    for key in list(saved):
        if key not in keys:
            item = next((s for s in proxy._sockets(tree, "INPUT") if s.identifier == saved[key]), None)
            if item is not None:
                proxy._remove_input(tree, item)
            del saved[key]
    encoded = json.dumps(saved)
    if tree.get("minihou_object_sockets") != encoded:
        tree["minihou_object_sockets"] = encoded
    for key in keys:
        modifiers._object_socket(adapter, key)
    proxy._sync_usage_chain(tree, json.loads(tree[modifiers.PARAMS]),
                            json.loads(tree.get(proxy.MAP_KEY, "{}")))


def _bindings(obj, mod, path, node):
    tree = node.node_tree
    adapter = SimpleNamespace(node_group=tree)
    result = {}
    for key in modifiers._object_keys(adapter):
        item = modifiers._object_socket(adapter, key)
        source = _value(mod.node_group, mod, path[:-1], _socket(node.inputs, item.identifier))
        if source is not None:
            modifiers._validate_source(obj, source)
        result[key] = source
    return result


def _sources(client, ident, bindings, materials):
    import numpy as np
    sources = {}
    for key, source in bindings.items():
        if source is None:
            continue
        _, arrays = mesh_reader.read_object(source, bpy.context.evaluated_depsgraph_get())
        slots = source.evaluated_get(bpy.context.evaluated_depsgraph_get()).data.materials
        mapping = []
        for material in slots:
            material = material.original if material else None
            if material not in materials:
                materials.append(material)
            mapping.append(materials.index(material))
        if mapping and "prim/material_index" in arrays:
            arrays["prim/material_index"] = np.asarray(mapping, dtype=np.int32)[
                np.clip(arrays["prim/material_index"], 0, len(mapping) - 1)]
        sources[key] = arrays
    h = hashlib.sha256()
    for key, arrays in sources.items():
        h.update(key.encode())
        for name, array in sorted(arrays.items()):
            h.update(str((name, array.dtype, array.shape)).encode())
            h.update(array.tobytes())
    token = h.hexdigest()
    if _source_sent.get(ident) != (client, token):
        client.set_sources(ident, sources)
        _source_sent[ident] = (client, token)
    return token


def _apply_live(node, header):
    if not header.get("live"):
        _live.discard(node.node_tree[KEY])
        return
    tree = node.node_tree
    graph = json.dumps(header["graph"])
    if tree[modifiers.GRAPH] != graph or tree[modifiers.PARAMS] != json.dumps(header["interface"]):
        tree[modifiers.GRAPH] = graph
        tree[modifiers.PARAMS] = json.dumps(header["interface"])
        proxy.sync_interface(tree, _cache(tree), header["interface"], json.dumps(header["interface"]))
        _object_interface(tree)
        for p in header["interface"]:
            socket = node.inputs.get(p["name"])
            if socket is not None and not socket.is_linked:
                socket.default_value = p["value"]
        _digests.pop(tree[KEY], None)
    tree[modifiers.PATH] = header["path"]


def edit_live(obj, node):
    """Resolve a unique occurrence on this object's stack, then bind actual wired input."""
    if obj is None:
        raise ValueError("Select the mesh object owning this Geometry Nodes network")
    matches = [(mod, path) for mod in obj.modifiers if mod.type == "NODES" and mod.node_group
               for path, candidate in _placements(mod.node_group) if candidate == node]
    if len(matches) != 1:
        raise ValueError("Select the group on its owning object; cook shared groups before editing")
    mod, path = matches[0]
    tree = node.node_tree
    if KEY not in tree:
        raise ValueError("Cook Wired Groups first to convert this modifier wrapper")
    _validate(obj, mod.node_group, mod)
    client = modifiers._connection()
    arrays, materials, _ = _probe(obj, mod, path)
    materials = list(materials)
    _sources(client, tree[KEY], _bindings(obj, mod, path, node), materials)
    values = {p["name"]: _value(mod.node_group, mod, path[:-1], node.inputs[p["name"]])
              for p in json.loads(tree[modifiers.PARAMS])}
    scene = bpy.context.scene
    header = client.edit_live(tree[KEY], json.loads(tree[modifiers.GRAPH]), values, arrays,
                              label=obj.name + " / " + " / ".join(path),
                              frame=scene.frame_current + scene.frame_subframe,
                              fps=scene.render.fps / scene.render.fps_base)
    _live.add(tree[KEY])
    _apply_live(node, header)
    return {"state": "ready", "editor_url": client.welcome.get("editor_url", ""),
            "native_live": header.get("live") and header.get("editor") == "native"}, header["path"]


def _private(tree, seen_ids):
    """Copy only ancestors containing placements, preserving unrelated native groups."""
    if tree.library is not None:
        raise ValueError("MiniHou placements require local node groups")
    if tree.users - int(tree.use_fake_user) > 1:
        tree = tree.copy()
        tree.use_fake_user = False
    if modifiers.KEY in tree:
        tree[KEY] = uuid.uuid4().hex
        del tree[modifiers.KEY]
    if KEY in tree:
        if tree[KEY] in seen_ids:
            tree[KEY] = uuid.uuid4().hex
        seen_ids.add(tree[KEY])
        proxy.ensure_plumbing(tree)  # a copied nested group gets its own plumbing tree
        _cache(tree)
        _object_interface(tree)
    else:
        for node in tree.nodes:
            child = getattr(node, "node_tree", None)
            if contains(child):
                private = _private(child, seen_ids)
                if private != child:
                    node.node_tree = private
    return tree


def _tree(root, path):
    for name in path:
        root = root.nodes[name].node_tree
    return root


def _socket(sockets, identifier):
    return next(s for s in sockets if s.identifier == identifier)


def _output(tree):
    outputs = [n for n in tree.nodes if n.bl_idname == "NodeGroupOutput"]
    return next((n for n in outputs if n.is_active_output), outputs[0] if outputs else None)


def _placements(tree, path=()):
    for node in tree.nodes:
        if is_group(node):
            yield path + (node.name,), node
        elif contains(getattr(node, "node_tree", None)):
            yield from _placements(node.node_tree, path + (node.name,))


def _order(root):
    """Trace actual socket links across group boundaries, not editor node order."""
    done, active, ordered, added = set(), set(), [], set()

    def input_socket(path, socket):
        for link in socket.links:
            output_socket(path, link.from_node, link.from_socket)

    def output_socket(path, node, socket):
        key = (path, node.name, socket.identifier)
        if key in active:
            raise ValueError("Cyclic Geometry Nodes links are unsupported")
        if key in done:
            return
        active.add(key)
        if node.bl_idname == "NodeGroupInput":
            if path:
                parent = _tree(root, path[:-1]).nodes[path[-1]]
                input_socket(path[:-1], _socket(parent.inputs, socket.identifier))
        elif is_group(node):
            if node.mute:
                raise ValueError("Muted MiniHou placements are unsupported; disconnect them instead")
            for s in node.inputs:
                input_socket(path, s)
            placement = path + (node.name,)
            if placement not in added:
                added.add(placement)
                ordered.append(placement)
        elif node.bl_idname == "GeometryNodeGroup" and node.node_tree:
            gout = _output(node.node_tree)
            if gout:
                input_socket(path + (node.name,), _socket(gout.inputs, socket.identifier))
        else:
            for s in node.inputs:
                input_socket(path, s)
        active.remove(key)
        done.add(key)

    gout = _output(root)
    if gout:
        for socket in gout.inputs:
            if socket.type == "GEOMETRY":
                input_socket((), socket)
    return ordered


def _value(root, mod, path, socket, seen=None):
    seen = set() if seen is None else seen
    key = (path, socket.as_pointer())
    if key in seen:
        raise ValueError("Cyclic parameter binding")
    seen.add(key)
    if not socket.is_linked:
        value = socket.default_value
        return list(value) if socket.type == "VECTOR" else value
    link = socket.links[0]
    node, source = link.from_node, link.from_socket
    if node.bl_idname == "NodeReroute":
        return _value(root, mod, path, node.inputs[0], seen)
    if node.bl_idname == "NodeGroupInput":
        if path:
            parent = _tree(root, path[:-1]).nodes[path[-1]]
            return _value(root, mod, path[:-1], _socket(parent.inputs, source.identifier), seen)
        owner = proxy.input_owner(mod, source.identifier)
        if owner is not None and owner[1] == "value":
            if getattr(owner[0], "type", "VALUE") != "VALUE":
                raise ValueError("MiniHou parameters require uniform values; modifier attribute/field mode is unsupported")
        elif mod.get(source.identifier + "_use_attribute", False):
            raise ValueError("MiniHou parameters require uniform values; modifier attribute/field mode is unsupported")
        if proxy.has_input(mod, source.identifier):
            value = proxy.get_input(mod, source.identifier)
        else:
            value = _socket(proxy._sockets(root, "INPUT"), source.identifier).default_value
        return list(value) if source.type == "VECTOR" else value
    if node.bl_idname == "ShaderNodeValue":
        return source.default_value
    raise ValueError("MiniHou parameters support literal values, Value nodes, reroutes and Group Input bindings; "
                     "computed fields/linked expressions are unsupported: " + socket.name)


def _validate(obj, tree, mod):
    seen = set()
    def check(target):
        targets = list(target.all_objects) if isinstance(target, bpy.types.Collection) else [target]
        for item in targets:
            if isinstance(item, bpy.types.Object):
                modifiers._validate_source(obj, item)
    def walk(group):
        if group.as_pointer() in seen or KEY in group:
            return
        seen.add(group.as_pointer())
        for node in group.nodes:
            name = node.bl_idname
            if node.mute and contains(getattr(node, "node_tree", None)):
                raise ValueError("Muted groups containing MiniHou are unsupported; disconnect them instead")
            if any(word in name for word in ("Simulation", "Repeat", "ForeachGeometryElement", "Bake")):
                raise ValueError("Native simulation/repeat/foreach/bake zones are unsupported in bridged trees")
            if name == "GeometryNodeSelfObject":
                raise ValueError("Self Object is unsupported in staged probes (feedback prevention)")
            for socket in node.inputs:
                target = getattr(socket, "default_value", None)
                check(target)
            child = getattr(node, "node_tree", None)
            if child:
                walk(child)
    walk(tree)
    for socket in proxy._sockets(tree, "INPUT"):
        if socket.socket_type in {"NodeSocketObject", "NodeSocketCollection"} and proxy.has_input(mod, socket.identifier):
            check(proxy.get_input(mod, socket.identifier))


def _probe(obj, mod, path):
    """Lift the placement's wired input through temporary ancestor output sockets."""
    copies = []
    temp = obj.copy()
    temp.name = "__MiniHou_GN_probe__"
    temp.hide_render = True
    temp.hide_viewport = False
    try:
        index = list(obj.modifiers).index(mod)
        for preceding in list(obj.modifiers)[:index]:
            if (preceding.show_viewport and preceding.type == "NODES" and preceding.node_group
                    and not modifiers.is_modifier(preceding)):
                _validate(obj, preceding.node_group, preceding)
        modifiers._check_prefix_dependencies(obj, [m for m in list(obj.modifiers)[:index]
                                                  if not contains(getattr(m, "node_group", None))])
        for suffix in list(temp.modifiers)[index + 1:]:
            temp.modifiers.remove(suffix)
        root = mod.node_group.copy()
        copies.append(root)
        temp.modifiers[index].node_group = root
        tree = root
        ancestors = []
        for name in path[:-1]:
            node = tree.nodes[name]
            child = node.node_tree.copy()
            copies.append(child)
            node.node_tree = child
            ancestors.append((tree, node))
            tree = child
        placement = tree.nodes[path[-1]]
        geometry = placement.inputs["Geometry"]
        if len(geometry.links) != 1:
            raise ValueError("MiniHou Geometry input must have one wired mesh input")
        source = geometry.links[0].from_socket
        for parent, node in reversed(ancestors):
            item = tree.interface.new_socket("__MiniHou_probe__", in_out="OUTPUT", socket_type="NodeSocketGeometry")
            tree.links.new(source, _socket(_output(tree).inputs, item.identifier))
            source = _socket(node.outputs, item.identifier)
            tree = parent
        gout = _output(root)
        target = next(s for s in gout.inputs if s.type == "GEOMETRY")
        # Unused output branches are not evaluation targets in the disposable graph.
        for s in gout.inputs:
            for link in list(s.links):
                root.links.remove(link)
        realize = root.nodes.new("GeometryNodeRealizeInstances")
        root.links.new(source, realize.inputs["Geometry"])
        root.links.new(realize.outputs["Geometry"], target)
        bpy.context.scene.collection.objects.link(temp)
        temp.hide_set(False)
        bpy.context.view_layer.update()
        dg = bpy.context.evaluated_depsgraph_get()
        evaluated = temp.evaluated_get(dg)
        if not hasattr(evaluated, "evaluated_geometry"):
            raise ValueError("Wired bridge requires Blender with Object.evaluated_geometry (5.1+)")
        geo = evaluated.evaluated_geometry()
        for component in ("curves", "pointcloud", "volume", "grease_pencil"):
            if getattr(geo, component, None) is not None:
                raise ValueError("MiniHou wired input requires mesh geometry; convert " + component + " upstream")
        if geo.instances_pointcloud() is not None:
            raise ValueError("MiniHou wired input contains residual instances after realization")
        mesh = evaluated.to_mesh(preserve_all_data_layers=True, depsgraph=dg)
        try:
            arrays, info = mesh_reader._read_mesh(temp, mesh)
            materials = tuple(m.original if m else None for m in mesh.materials)
        finally:
            evaluated.to_mesh_clear()
        return arrays, materials, info
    finally:
        bpy.data.objects.remove(temp, do_unlink=True)
        for tree in copies:
            bpy.data.node_groups.remove(tree)


def _signature(tree, arrays, params, materials):
    h = hashlib.sha256()
    scene = bpy.context.scene
    h.update(json.dumps([tree[modifiers.GRAPH], params,
                         [m.as_pointer() if m else 0 for m in materials],
                         scene.frame_current, scene.frame_subframe, scene.render.fps, scene.render.fps_base],
                        sort_keys=True).encode())
    for name, arr in sorted(arrays.items()):
        h.update(str((name, arr.dtype.str, arr.shape)).encode())
        h.update(arr.tobytes())
    return h.hexdigest()


def _cook_tree(obj, mod, host, port, force):
    root = mod.node_group
    for _, placement in _placements(root):
        modifiers._disconnect_unsafe_sources(obj, SimpleNamespace(node_group=placement.node_tree))
    _validate(obj, root, mod)
    paths = _order(root)
    active = set(paths)
    for path, node in _placements(root):
        if path not in active:
            node.node_tree[modifiers.STATUS] = "Disconnected from Geometry output; not cooked"
    cooked = 0
    for path in paths:
        node = _tree(root, path[:-1]).nodes[path[-1]]
        tree = node.node_tree
        try:
            graph = json.loads(tree[modifiers.GRAPH])
            if any(n.get("type") == "rbd_solver" for n in graph.get("nodes", {}).values()):
                if (obj.animation_data or obj.data.animation_data or
                        (obj.data.shape_keys and obj.data.shape_keys.animation_data) or
                        modifiers._animated_nodes(root)):
                    raise ValueError("RBD requires static input; animated objects, fields and node trees are unsupported")
                for preceding in obj.modifiers:
                    if preceding == mod:
                        break
                    if preceding.type in {"CLOTH", "SOFT_BODY", "FLUID", "PARTICLE_SYSTEM", "MESH_SEQUENCE_CACHE"} or (
                            preceding.type == "NODES" and modifiers._animated_nodes(preceding.node_group)):
                        raise ValueError("RBD requires static input; animated upstream modifiers are unsupported")
            values = {p["name"]: _value(root, mod, path[:-1], node.inputs[p["name"]])
                      for p in json.loads(tree[modifiers.PARAMS])}
            client = modifiers._connection(host, port)
            if tree[KEY] in _live:
                _apply_live(node, client.sync_live(tree[KEY], values))
                graph = json.loads(tree[modifiers.GRAPH])
                values = {p["name"]: _value(root, mod, path[:-1], node.inputs[p["name"]])
                          for p in json.loads(tree[modifiers.PARAMS])}
            # Validate pickers before evaluating any staged geometry.
            bindings = _bindings(obj, mod, path, node)
            arrays, materials, info = _probe(obj, mod, path)
            materials = list(materials)
            source_digest = _sources(client, tree[KEY], bindings, materials)
            signature = _signature(tree, arrays, values, materials) + source_digest
            names = json.loads(tree.get(proxy.INST_KEY, "[]"))
            objects = [bindings.get(name[1:]) for name in names]
            if names:
                proxy.sync_instances(tree, names, objects=objects)
            ready = "Ready (staged wired input)" + (
                "; skipped attributes: " + ", ".join(info["skipped"]) if info["skipped"] else "")
            if not force and _digests.get(tree[KEY]) == signature:
                if tree.get(modifiers.STATUS) != ready:
                    tree[modifiers.STATUS] = ready
                continue
            scene = bpy.context.scene
            header, output = modifiers._connection(host, port).cook(
                tree[KEY], tree[modifiers.PATH], values, arrays,
                frame=scene.frame_current + scene.frame_subframe,
                fps=scene.render.fps / scene.render.fps_base, graph=graph,
                materials=[m.name if m is not None else "" for m in materials])
            output, manifests = instance_payload(header, output)
            update = GeoUpdate.from_message(header, output)
            names = update.detail.get("instance_objects", [])
            if any(not name.startswith("@") for name in names):
                raise ValueError("Use @binding object inputs for wired group instances")
            objects = [bindings.get(name[1:]) for name in names]
            mirror = Mirror()
            mirror.apply(update)
            cache = _cache(tree)
            # Publish only a fully constructed mesh, keeping last valid output on failure.
            mesh = bpy.data.meshes.new("MiniHou GN cooked")
            try:
                for material in materials:
                    mesh.materials.append(material)
                mesh_writer.apply_update(mesh, mirror, update)
            except Exception:
                bpy.data.meshes.remove(mesh)
                raise
            old = cache.data
            cache.data = mesh
            if old.users == 0:
                bpy.data.meshes.remove(old)
            proxy.sync_instances(tree, names, objects=objects)
            for manifest in manifests:
                scene_objects.queue(tree[KEY], manifest, parent=obj.name)
            for error in header.get("scene_errors", []):
                scene_objects._log(f"scene_objects {error.get('node')!r} on {obj.name}: {error.get('message')}")
            scene_objects.queue_nodes(tree[KEY], [m["node"] for m in manifests]
                                      + [e["node"] for e in header.get("scene_errors", [])])
            _digests[tree[KEY]] = signature
            _known.add(tree[KEY])
            tree[modifiers.STATUS] = ready
            bpy.context.view_layer.update()
            cooked += 1
        except Exception as exc:
            tree[modifiers.STATUS] = "Error (last valid output retained): " + str(exc)
            # Do not consume stale upstream output as a successful downstream cook.
            for later in paths[paths.index(path) + 1:]:
                _tree(root, later[:-1]).nodes[later[-1]].node_tree[modifiers.STATUS] = "Waiting: preceding placement failed"
            raise
    return cooked


def _cleanup():
    # Saved or asset-marked groups retain their result; unused placement groups do not.
    for tree in list(bpy.data.node_groups):
        if KEY in tree and tree.users == 0:
            bpy.data.node_groups.remove(tree)
    proxy.cleanup_plumbing()
    used = {tree[KEY] for tree in bpy.data.node_groups if KEY in tree}
    for ident in (_known | set(_digests) | set(_source_sent) | _live) - used:
        if modifiers._client is not None:
            try:
                modifiers._client.remove(ident)
            except Exception:
                pass  # client.close/next cook handles transport failure
        scene_objects.queue_nodes(ident, [])
        _known.discard(ident)
        _live.discard(ident)
        _source_sent.pop(ident, None)
        _digests.pop(ident, None)
    for obj in list(bpy.data.objects):
        if CACHE in obj and obj[CACHE] not in used:
            mesh = obj.data
            bpy.data.objects.remove(obj, do_unlink=True)
            if mesh.users == 0:
                bpy.data.meshes.remove(mesh)


def _waiting_after(obj, mod):
    for later in list(obj.modifiers)[list(obj.modifiers).index(mod) + 1:]:
        if modifiers.is_modifier(later):
            later.node_group[modifiers.STATUS] = "Waiting: preceding MiniHou computation failed"
        elif later.type == "NODES" and contains(later.node_group):
            for _, node in _placements(later.node_group):
                node.node_tree[modifiers.STATUS] = "Waiting: preceding MiniHou computation failed"


def pump(host=None, port=None, *, force=False):
    """Cook GN placements and legacy MiniHou modifiers in object stack order."""
    global _busy, _last_error, _retry_at
    if mesh_writer.modal_ui_active():
        return 0
    if _busy or modifiers._rendering or (not force and time.monotonic() < _retry_at):
        return 0
    mesh_writer.assert_main_thread()
    _busy = True
    cooked, errors = 0, []
    try:
        seen = set()
        for obj in list(bpy.data.objects):
            if obj.library or obj.type != "MESH" or CACHE in obj:
                continue
            for mod in obj.modifiers:
                if mod.type == "NODES" and not modifiers.is_modifier(mod) and contains(mod.node_group):
                    tree = _private(mod.node_group, seen)
                    if tree != mod.node_group:
                        mod.node_group = tree
        _cleanup()
        for obj in list(bpy.context.scene.objects):
            try:
                if obj.library or obj.type != "MESH" or CACHE in obj:
                    continue
            except ReferenceError:
                continue  # a stale cache object freed by an earlier modifiers.pump cleanup in this loop
            for mod in obj.modifiers:
                if not mod.show_viewport:
                    continue
                if modifiers.is_modifier(mod):
                    cooked += modifiers.pump(host, port, force=force, _only=(obj, mod))
                    if modifiers.last_error():
                        errors.append(modifiers.last_error())
                        _waiting_after(obj, mod)
                        break
                elif mod.type == "NODES" and contains(mod.node_group):
                    try:
                        if obj.mode != "OBJECT":
                            raise ValueError("Object mode required; last valid output retained")
                        cooked += _cook_tree(obj, mod, host, port, force)
                    except Exception as exc:
                        errors.append(str(exc))
                        _waiting_after(obj, mod)
                        for _, node in _placements(mod.node_group):
                            if not status(node).startswith(("Error", "Waiting: preceding")):
                                node.node_tree[modifiers.STATUS] = "Error (last valid output retained): " + str(exc)
                        if isinstance(exc, (OSError, ConnectionError)):
                            modifiers.disconnect()
                            _digests.clear()
                            _retry_at = time.monotonic() + 2
                        break
        _last_error = "; ".join(dict.fromkeys(errors))
        if not errors:
            _retry_at = 0
    except Exception as exc:
        _last_error = str(exc)
    finally:
        _busy = False
    return cooked


def last_error():
    return _last_error


@persistent
def _reset(_dummy=None):
    global _retry_at, _last_error
    _digests.clear()
    _live.clear()
    _source_sent.clear()
    _retry_at, _last_error = 0, ""


def register():
    for handlers in (bpy.app.handlers.load_post, bpy.app.handlers.undo_post, bpy.app.handlers.redo_post):
        if _reset not in handlers:
            handlers.append(_reset)


def unregister():
    for handlers in (bpy.app.handlers.load_post, bpy.app.handlers.undo_post, bpy.app.handlers.redo_post):
        if _reset in handlers:
            handlers.remove(_reset)
    _reset()
