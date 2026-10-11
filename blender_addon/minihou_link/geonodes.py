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

import bpy
from bpy.app.handlers import persistent

from . import modifiers, proxy, mesh_reader, mesh_writer
from .protocol import GeoUpdate, Mirror

KEY = "minihou_gn_id"
CACHE = "minihou_gn_cache"
_digests = {}
_busy = False
_last_error = ""
_retry_at = 0.0
_known = set()


def is_group(node):
    return node.bl_idname == "GeometryNodeGroup" and node.node_tree is not None and KEY in node.node_tree


def contains(tree, seen=None):
    if tree is None:
        return False
    seen = set() if seen is None else seen
    if tree.as_pointer() in seen:
        return False
    seen.add(tree.as_pointer())
    return KEY in tree or any(contains(getattr(n, "node_tree", None), seen) for n in tree.nodes)


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
    cache = _cache(ng)
    proxy.sync_interface(ng, cache, header["interface"], json.dumps(header["interface"]))
    node = tree.nodes.new("GeometryNodeGroup")
    node.node_tree = ng
    node.label = "MiniHou: " + os.path.basename(path)
    node.width = 240
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
    _digests.pop(tree[KEY], None)


def _private(tree, seen_ids):
    """Copy only ancestors containing placements, preserving unrelated native groups."""
    if tree.library is not None:
        raise ValueError("MiniHou placements require local node groups")
    if tree.users > 1:
        tree = tree.copy()
    if KEY in tree:
        if tree[KEY] in seen_ids:
            tree[KEY] = uuid.uuid4().hex
        seen_ids.add(tree[KEY])
        _cache(tree)
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
            values = {p["name"]: _value(root, mod, path[:-1], node.inputs[p["name"]])
                      for p in json.loads(tree[modifiers.PARAMS])}
            arrays, materials, info = _probe(obj, mod, path)
            signature = _signature(tree, arrays, values, materials)
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
                fps=scene.render.fps / scene.render.fps_base, graph=json.loads(tree[modifiers.GRAPH]))
            update = GeoUpdate.from_message(header, output)
            if update.detail.get("instance_objects"):
                raise ValueError("Wired groups require mesh output; external object instancing is unsupported")
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
    used = {tree[KEY] for tree in bpy.data.node_groups if KEY in tree}
    for ident in (_known | set(_digests)) - used:
        if modifiers._client is not None:
            try:
                modifiers._client.remove(ident)
            except Exception:
                pass  # client.close/next cook handles transport failure
        _known.discard(ident)
        _digests.pop(ident, None)
    for obj in list(bpy.data.objects):
        if CACHE in obj and obj[CACHE] not in used:
            mesh = obj.data
            bpy.data.objects.remove(obj, do_unlink=True)
            if mesh.users == 0:
                bpy.data.meshes.remove(mesh)


def pump(host=None, port=None, *, force=False):
    """Cook GN placements and legacy MiniHou modifiers in object stack order."""
    global _busy, _last_error, _retry_at
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
                if mod.type == "NODES" and contains(mod.node_group):
                    tree = _private(mod.node_group, seen)
                    if tree != mod.node_group:
                        mod.node_group = tree
        _cleanup()
        for obj in list(bpy.context.scene.objects):
            if obj.library or obj.type != "MESH" or CACHE in obj:
                continue
            for mod in obj.modifiers:
                if not mod.show_viewport:
                    continue
                if modifiers.is_modifier(mod):
                    cooked += modifiers.pump(host, port, force=force, _only=(obj, mod))
                    if modifiers.last_error():
                        errors.append(modifiers.last_error())
                        break
                elif mod.type == "NODES" and contains(mod.node_group):
                    try:
                        if obj.mode != "OBJECT":
                            raise ValueError("Object mode required; last valid output retained")
                        cooked += _cook_tree(obj, mod, host, port, force)
                    except Exception as exc:
                        errors.append(str(exc))
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
