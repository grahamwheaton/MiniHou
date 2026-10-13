"""Real Blender objects from a ``scene_objects`` node. Main thread only.

The engine sends manifests (see :mod:`scene_plan` and ``minihou/nodes/scene_objects.py``); this module makes the objects: one
**linked duplicate** of a source object per entry (they share its mesh, so memory stays small and every object can still have its
own modifiers, materials and constraints). Objects are found again by custom properties, never by name; a cook only creates,
moves, shows and hides, so work the user adds to the objects survives. ``cleanup`` deletes the parked ones on request.

Objects are never created or deleted from a depsgraph, frame-change or render handler: those only :func:`queue`, and the add-on's
timer calls :func:`flush`.
"""
from __future__ import annotations

import hashlib
from typing import Callable, Dict, List, Optional, Tuple

import bpy
import mathutils
import numpy as np

from . import scene_plan as sp
from .mesh_writer import NATIVE, apply_material_names, assert_main_thread, material_for, set_corner_verts, set_positions, write_attribute, write_native

SESSION = "session"
_pending: Dict[Tuple[str, str], Tuple[dict, str]] = {}      # (owner, node) -> (manifest, parent object name)
_pending_nodes: Dict[str, List[str]] = {}                   # owner -> nodes that exist now
_log: Callable[[str], None] = print
last_summary: Dict[str, dict] = {}


def set_logger(fn: Callable[[str], None]) -> None:
    global _log
    _log = fn


# -- queueing (safe anywhere) ---------------------------------------------------------------------------------------------
def queue(owner: str, manifest: dict, parent: str = "") -> None:
    _pending[(owner, str(manifest["node"]))] = (manifest, parent)


def queue_nodes(owner: str, nodes: List[str]) -> None:
    _pending_nodes[owner] = [str(n) for n in nodes]


def flush() -> int:
    """Apply what was queued. Returns the number of objects touched."""
    assert_main_thread()
    if not _pending and not _pending_nodes:
        return 0
    manifests, nodes = dict(_pending), dict(_pending_nodes)
    _pending.clear()
    _pending_nodes.clear()
    touched = 0
    for (owner, node), (manifest, parent) in manifests.items():
        try:
            s = apply(manifest, owner, parent)
        except Exception as e:  # noqa: BLE001 - one bad manifest must not stop the others (or the timer)
            _log(f"scene_objects {node!r}: {type(e).__name__}: {e}")
            continue
        last_summary[f"{owner}:{node}"] = s
        touched += s["created"] + s["moved"] + s["hidden"] + s["shown"] + s["rewritten"]
    for owner, names in nodes.items():
        touched += hide_removed_nodes(owner, names)
    return touched


# -- finding what we made --------------------------------------------------------------------------------------------------
def marked(tag: Optional[str] = None, owner: Optional[str] = None):
    for obj in bpy.data.objects:
        t = obj.get(sp.MARK)
        if t is None:
            continue
        if tag is not None and t != tag:
            continue
        if owner is not None and not str(t).startswith(owner + ":"):
            continue
        yield obj


def _existing(tag: str):
    by_id: Dict[int, "bpy.types.Object"] = {}
    extras = []
    for obj in sorted(marked(tag), key=lambda o: o.name):
        ident = int(obj.get(sp.MARK_ID, 0))
        if ident in by_id:
            extras.append(obj)
        else:
            by_id[ident] = obj
    return by_id, extras


def _info(obj) -> sp.Existing:
    return sp.Existing(int(obj.get(sp.MARK_ID, 0)), obj.name, str(obj.get(sp.MARK_SRC, "")), bool(obj.get(sp.MARK_HIDDEN)),
                       bool(obj.get(sp.MARK_PLACEHOLDER)), tuple(obj.location), tuple(obj.rotation_euler), tuple(obj.scale),
                       obj.parent.name if obj.parent else "")


# -- hiding and showing -----------------------------------------------------------------------------------------------------
def _hide(obj) -> None:
    obj[sp.MARK_HIDDEN] = 1
    obj.hide_viewport = True
    obj.hide_render = True
    try:
        obj.hide_set(True)
    except RuntimeError:                                  # not in the active view layer: the two flags above are enough
        pass


def _show(obj) -> None:
    if obj.get(sp.MARK_HIDDEN):
        del obj[sp.MARK_HIDDEN]
    obj.hide_viewport = False
    obj.hide_render = False
    try:
        obj.hide_set(False)
    except RuntimeError:
        pass


# -- the collection ---------------------------------------------------------------------------------------------------------
def _collection(name: str):
    scene = bpy.context.scene
    col = bpy.data.collections.get(name)
    if col is None:
        col = bpy.data.collections.new(name)
    if col != scene.collection and col.name not in {c.name for c in scene.collection.children_recursive}:
        scene.collection.children.link(col)
    return col


# -- making objects ---------------------------------------------------------------------------------------------------------
def _make(name: str, src_name: str, tag: str, ident: int, col, placeholder_log: List[str]):
    src = bpy.data.objects.get(src_name) if src_name else None
    if src is not None and src.get(sp.MARK):
        src = None                                        # never duplicate our own objects
    if src is None:
        obj = bpy.data.objects.new(name, None)
        obj.empty_display_type = "PLAIN_AXES"
        obj.empty_display_size = 0.5
        obj[sp.MARK_PLACEHOLDER] = 1
        if src_name:
            placeholder_log.append(src_name)
    else:
        obj = bpy.data.objects.new(name, src.data)        # a linked duplicate: shares the mesh/curve/light data
        if src.data is None:                              # an empty (maybe a collection instance)
            obj.empty_display_type = src.empty_display_type
            obj.empty_display_size = src.empty_display_size
            obj.instance_type = src.instance_type
            obj.instance_collection = src.instance_collection
    obj[sp.MARK_ID] = int(ident)
    obj[sp.MARK_SRC] = src_name
    obj.rotation_mode = "XYZ"
    col.objects.link(obj)
    return obj


DIGEST = "minihou_digest"        # on a piece's mesh: hash of the shape last written, so an unchanged piece is not rewritten


def _make_shaped(mode: str, name: str, ident: int, col):
    """The object for a shaped mode: a mesh object (pieces), a curves object or a point-cloud object, with data of its own."""
    if mode == "curves":
        data = bpy.data.hair_curves.new(name)
    elif mode == "points":
        data = bpy.data.pointclouds.new(name)
    else:
        data = bpy.data.meshes.new(name)
    obj = bpy.data.objects.new(name, data)
    obj[sp.MARK_ID] = int(ident)
    obj[sp.MARK_SRC] = ""
    obj.rotation_mode = "XYZ"
    col.objects.link(obj)
    return obj


_OBJECT_TYPE = {"pieces": "MESH", "curves": "CURVES", "points": "POINTCLOUD"}
COLOR_NAME = "Color"                                 # what the engine's Cd becomes on curves and point clouds


def _blender_attr(name: str) -> str:
    return COLOR_NAME if name == "Cd" else name


def _write_attributes(data, domain_of: Dict[str, int], attrs: Dict[str, np.ndarray]) -> List[str]:
    """Write ``"domain:name"`` arrays as attributes of a Curves / PointCloud (``domain_of``: 'point'/'prim' -> blender domain name and
    size are chosen by the caller). Returns warnings."""
    warnings: List[str] = []
    for key, arr in attrs.items():
        dom, name = key.split(":", 1)
        blender_domain = "POINT" if dom == "point" else "CURVE"
        a = np.asarray(arr)
        bname = _blender_attr(name)
        try:
            if a.ndim == 2 and a.shape[1] == 3 and name != "Cd":
                kind, prop, flat = "FLOAT_VECTOR", "vector", a.astype(np.float32)
            elif a.ndim == 2 and a.shape[1] in (3, 4) and name == "Cd":
                rgba = np.ones((len(a), 4), np.float32)
                rgba[:, :a.shape[1]] = a
                kind, prop, flat = "FLOAT_COLOR", "color", rgba
            elif a.ndim == 2 and a.shape[1] == 2:
                kind, prop, flat = "FLOAT2", "vector", a.astype(np.float32)
            elif a.ndim == 2:
                raise ValueError("unsupported width")
            elif a.dtype.kind == "b":
                kind, prop, flat = "BOOLEAN", "value", a
            elif a.dtype.kind in "iu":
                kind, prop, flat = "INT", "value", a.astype(np.int32)
            else:
                kind, prop, flat = "FLOAT", "value", a.astype(np.float32)
            existing = data.attributes.get(bname)
            if existing is not None and (existing.data_type != kind or existing.domain != blender_domain):
                data.attributes.remove(existing)
                existing = None
            if existing is None:
                existing = data.attributes.new(bname, kind, blender_domain)
            existing.data.foreach_set(prop, np.ascontiguousarray(flat).ravel())
        except (RuntimeError, TypeError, ValueError, KeyError) as e:
            warnings.append(f"attribute {key!r} could not be written ({e})")
    return warnings


def _write_curves(data, sizes: np.ndarray, P: np.ndarray, cyclic: np.ndarray, attrs: Dict[str, np.ndarray]) -> List[str]:
    """Fill a fresh Curves datablock: poly curves with the given points, cyclic flags and attributes."""
    data.add_curves([int(x) for x in sizes])
    data.set_types(type="POLY")
    data.attributes["position"].data.foreach_set("vector", np.ascontiguousarray(P, np.float32).ravel())
    if len(cyclic) and cyclic.any():
        data.attributes.new("cyclic", "BOOLEAN", "CURVE").data.foreach_set("value", cyclic.astype(np.int8))
    return _write_attributes(data, {}, attrs)


def _write_points(data, P: np.ndarray, attrs: Dict[str, np.ndarray]) -> List[str]:
    data.resize(len(P))
    data.attributes["position"].data.foreach_set("vector", np.ascontiguousarray(P, np.float32).ravel())
    return _write_attributes(data, {}, attrs)


def _apply_shaped(mode: str, manifest: dict, arrays: dict, objs: Dict[int, "bpy.types.Object"], col, tag: str, node: str) -> int:
    """curves / points: one object (id 0) whose data is replaced when the shape changed. Returns how many were rewritten."""
    obj = objs.get(0)
    if obj is None:
        return 0
    if obj.type != _OBJECT_TYPE[mode]:                    # the node used to make something else: start over (its modifiers cannot move)
        _log(f"scene_objects {node!r}: {obj.name} was a {obj.type.lower()} object, now the node makes {mode}; it is replaced "
             f"(modifiers on the old one are lost)")
        name = obj.name
        bpy.data.objects.remove(obj, do_unlink=True)
        obj = _make_shaped(mode, name, 0, col)
        obj[sp.MARK] = tag
        objs[0] = obj
    h = hashlib.blake2b(digest_size=12)
    for key in sorted(arrays):
        h.update(key.encode())
        h.update(np.ascontiguousarray(arrays[key]).tobytes())
    digest = h.hexdigest()
    if obj.data.get(DIGEST) == digest:
        return 0
    name = obj.data.name
    old = obj.data
    new = bpy.data.hair_curves.new(name) if mode == "curves" else bpy.data.pointclouds.new(name)
    attrs = {k: arrays[k] for k in manifest.get("attributes", []) if k in arrays}
    try:
        warnings = (_write_curves(new, arrays["sizes"], arrays["P"], arrays["cyclic"], attrs) if mode == "curves"
                    else _write_points(new, arrays["P"], attrs))
    except Exception:
        (bpy.data.hair_curves if mode == "curves" else bpy.data.pointclouds).remove(new)
        raise
    for w in dict.fromkeys(warnings):
        _log(f"scene_objects {node!r}: {w}")
    new[DIGEST] = digest
    for slot in range(len(old.materials)):                # materials the user put on the object stay
        new.materials.append(old.materials[slot])
    obj.data = new
    if old.users == 0:
        (bpy.data.hair_curves if mode == "curves" else bpy.data.pointclouds).remove(old)
    return 1


def _write_piece(mesh, P: np.ndarray, vp: np.ndarray, sizes: np.ndarray, attrs: Optional[Dict[str, np.ndarray]] = None) -> List[str]:
    """Fill ``mesh`` with one piece. ``attrs``: ``"domain:name"`` -> values. Returns warnings."""
    import types
    warnings: List[str] = []
    mesh.clear_geometry()
    if len(P):
        mesh.vertices.add(len(P))
    if len(vp):
        mesh.loops.add(len(vp))
        set_corner_verts(mesh, vp)
    if len(sizes):
        mesh.polygons.add(len(sizes))
        mesh.polygons.foreach_set("loop_start", (np.cumsum(sizes) - sizes).astype(np.int32))
    if len(P):
        set_positions(mesh, P)
    mesh.update(calc_edges=True)
    shape = types.SimpleNamespace(vertex_point=vp, prim_offsets=np.concatenate([[0], np.cumsum(sizes)]).astype(np.int32))
    for key, arr in (attrs or {}).items():
        domain, name = key.split(":", 1)
        try:
            ok = bool(write_native(mesh, shape, domain, name, arr)) if name in NATIVE else write_attribute(mesh, domain, name, arr)
        except (RuntimeError, TypeError, ValueError) as e:
            ok, name = False, f"{name} ({e})"
        if not ok:
            warnings.append(f"attribute {key!r} could not be written to the piece meshes")
    mesh.update()
    return warnings


def _piece_slices(manifest: dict, arrays: dict):
    """Per manifest entry: (id, P, vp, sizes, attrs) views into the arrays."""
    pb = np.concatenate([[0], np.cumsum(manifest["npts"])]).astype(np.int64)
    cb = np.concatenate([[0], np.cumsum(manifest["ncorner"])]).astype(np.int64)
    fb = np.concatenate([[0], np.cumsum(manifest["nprim"])]).astype(np.int64)
    P, vp, sizes = arrays["P"].reshape(-1, 3), arrays["vp"], arrays["sizes"]
    if pb[-1] != len(P) or cb[-1] != len(vp) or fb[-1] != len(sizes):
        raise ValueError("the piece arrays do not match the counts in the manifest")
    bounds = {"point": pb, "vertex": cb, "prim": fb}
    wanted = [k for k in manifest.get("attributes", []) if k in arrays and k.split(":", 1)[0] in bounds]
    for k, ident in enumerate(manifest["id"]):
        attrs = {key: arrays[key][bounds[key.split(":", 1)[0]][k]:bounds[key.split(":", 1)[0]][k + 1]] for key in wanted}
        yield int(ident), P[pb[k]:pb[k + 1]], vp[cb[k]:cb[k + 1]], sizes[fb[k]:fb[k + 1]], attrs


_noted: set = set()                           # warnings already logged, so a cook per frame does not repeat them
MARK_MAT = "minihou_material"                 # on an instance object whose first slot we pointed at a material of its own


def _apply_object_materials(manifest: dict, objs: Dict[int, "bpy.types.Object"], node: str, warned: set) -> None:
    """Instances share their source's mesh, so a material of their own is an *object-linked* first slot. Objects the node names a
    material for get it; ones we overrode before and no longer name one go back to the source's. The source needs a slot."""
    col = manifest.get("material")
    names = {int(i): n for i, n in zip(manifest.get("id", []), col)} if col else {}
    for ident, obj in objs.items():
        name = names.get(ident, "")
        had = obj.get(MARK_MAT)
        if not name and had is None:
            continue
        slots = getattr(obj, "material_slots", None)
        if slots is None or not len(slots):
            if name and ("noslot", node) not in warned:
                warned.add(("noslot", node))
                _log(f"scene_objects {node!r}: {obj.name} has no material slot, so it cannot take the material {name!r}; give the source object one")
            continue
        slot = slots[0]
        if name:
            mat, made = material_for(name)
            if made and ("made", name) not in warned:
                warned.add(("made", name))
                _log(f"scene_objects {node!r}: material {name!r} is not in this file; made an empty one")
            if slot.link != "OBJECT":
                slot.link = "OBJECT"
            if slot.material is not mat:
                slot.material = mat
            obj[MARK_MAT] = name
        else:
            slot.link = "DATA"
            del obj[MARK_MAT]


def _retarget(obj, item: sp.Item, tag: str, col, placeholder_log: List[str]):
    """Make ``obj`` duplicate another source. Same kind of data: swap it in place (modifiers stay). Otherwise rebuild."""
    src = bpy.data.objects.get(item.src) if item.src else None
    if src is not None and src.get(sp.MARK):
        src = None
    if src is not None and src.data is not None and obj.type == src.type and not obj.get(sp.MARK_PLACEHOLDER):
        obj.data = src.data
        obj[sp.MARK_SRC] = item.src
        return obj
    if src is None and obj.get(sp.MARK_PLACEHOLDER):
        obj[sp.MARK_SRC] = item.src
        return obj
    mods = len(getattr(obj, "modifiers", []))
    if mods:
        _log(f"scene_objects: {obj.name} changes to a different kind of object, so its {mods} modifier(s) cannot be kept")
    name = obj.name
    bpy.data.objects.remove(obj, do_unlink=True)
    new = _make(name, item.src, tag, item.ident, col, placeholder_log)
    new[sp.MARK] = tag
    return new


def apply(manifest: dict, owner: str = SESSION, parent: str = "", arrays: Optional[dict] = None) -> dict:
    """Make the objects the manifest describes. ``arrays``: the binary part (pieces mode; session manifests carry it as
    ``_arrays``). Returns counts."""
    assert_main_thread()
    mode = str(manifest.get("mode") or "instances")
    pieces = mode == "pieces"
    shaped = mode in sp.SHAPED
    arrays = arrays if arrays is not None else manifest.get("_arrays")
    if shaped and not arrays:
        raise ValueError(f"a {mode} manifest arrived without its arrays")
    node = str(manifest["node"])
    tag = sp.owner_tag(owner, node)
    by_id, extras = _existing(tag)
    parent_obj = bpy.data.objects.get(parent) if parent else None
    present = [o.name for o in bpy.data.objects if not o.get(sp.MARK)]
    plan = sp.plan(manifest, {i: _info(o) for i, o in by_id.items()}, present, parent=parent_obj.name if parent_obj else "",
                   extras=[_info(o) for o in extras])
    col = _collection(str(manifest.get("collection") or manifest.get("prefix") or "MiniHou"))
    missing_log: List[str] = []
    objs: Dict[int, "bpy.types.Object"] = dict(by_id)
    for item in plan.create:
        obj = _make_shaped(mode, item.name, item.ident, col) if shaped else _make(item.name, item.src, tag, item.ident, col, missing_log)
        obj[sp.MARK] = tag
        objs[item.ident] = obj
    rebuilt = set()
    for item in plan.retarget:
        old = objs[item.ident]
        objs[item.ident] = _retarget(old, item, tag, col, missing_log)
        if objs[item.ident] is not old and item.ident not in {i.ident for i in plan.create}:
            rebuilt.add(item.ident)                          # a new object: it needs its transform and parent written
    for ident in plan.unhide:
        _show(objs[ident])
    created = {i.ident for i in plan.create} | rebuilt
    for item in plan.create + plan.update + [i for i in plan.retarget if i.ident in rebuilt]:
        obj = objs[item.ident]
        if item.ident in created or sp.changed(item, _info(obj)):
            obj.location = item.loc
            obj.rotation_euler = item.rot
            obj.scale = item.scale
    if parent_obj is not None:                            # a modifier's objects live in its owner's space: moving the owner moves them
        for ident in created | set(plan.reparent):
            obj = objs[ident]
            obj.parent = parent_obj
            obj.matrix_parent_inverse = mathutils.Matrix.Identity(4)
    rewritten = 0
    if mode in ("curves", "points"):
        rewritten = _apply_shaped(mode, manifest, arrays, objs, col, tag, node)
    if pieces:                                            # shapes: only the pieces whose shape really changed are rewritten
        warned = set()
        for ident, P, vp, sizes, attrs in _piece_slices(manifest, arrays):
            obj = objs.get(ident)
            if obj is None or obj.type != "MESH":
                continue
            h = hashlib.blake2b(digest_size=12)
            for a in (P, vp, sizes):
                h.update(np.ascontiguousarray(a).tobytes())
            for key in sorted(attrs):
                h.update(key.encode())
                h.update(np.ascontiguousarray(attrs[key]).tobytes())
            digest = h.hexdigest()
            if obj.data.get(DIGEST) != digest:
                for w in _write_piece(obj.data, P, vp, sizes, attrs):
                    if w not in warned:
                        warned.add(w)
                        _log(f"scene_objects {node!r}: {w}")
                obj.data[DIGEST] = digest
                rewritten += 1
            for w in apply_material_names(obj.data, manifest.get("materials")):      # slots by name; existing ones are kept
                if w not in warned:
                    warned.add(w)
                    _log(f"scene_objects {node!r}: {w}")
    if mode == "instances":
        _apply_object_materials(manifest, objs, node, _noted)
    for ident in plan.hide:
        _hide(objs[ident])
    for name in plan.duplicates:
        extra = bpy.data.objects.get(name)
        if extra is not None:
            _hide(extra)
    for name in dict.fromkeys(plan.missing_sources):
        if name and name in missing_log:
            _log(f"scene_objects {node!r}: the Blender object {name!r} was not found; empties stand in for it")
    return {"created": len(plan.create), "moved": len(plan.update), "retargeted": len(plan.retarget), "hidden": len(plan.hide),
            "shown": len(plan.unhide), "total": len(manifest.get("id", [])), "missing": list(plan.missing_sources),
            "rewritten": rewritten}


def hide_removed_nodes(owner: str, nodes: List[str]) -> int:
    """Park the objects of nodes that are no longer in the graph."""
    n = 0
    keep = {sp.owner_tag(owner, node) for node in nodes}
    for obj in list(marked(owner=owner)):
        if obj.get(sp.MARK) not in keep and not obj.get(sp.MARK_HIDDEN):
            _hide(obj)
            n += 1
    return n


def retain_owners(owners) -> int:
    """Park the objects of modifier instances that no longer exist (the session owner is left alone)."""
    owners = set(owners) | {SESSION}
    n = 0
    for obj in list(marked()):
        owner = str(obj.get(sp.MARK)).split(":", 1)[0]
        if owner not in owners and not obj.get(sp.MARK_HIDDEN):
            _hide(obj)
            n += 1
    return n


# -- user actions ---------------------------------------------------------------------------------------------------------------
def counts() -> Dict[str, int]:
    total = hidden = 0
    for obj in marked():
        total += 1
        hidden += 1 if obj.get(sp.MARK_HIDDEN) else 0
    return {"total": total, "hidden": hidden}


def cleanup(everything: bool = False) -> int:
    """Delete the parked objects (or all of them). Returns how many went."""
    assert_main_thread()
    gone = 0
    for obj in list(marked()):
        if everything or obj.get(sp.MARK_HIDDEN):
            mesh = obj.data if obj.type == "MESH" and obj.data is not None and obj.data.get(DIGEST) else None   # a piece's own mesh
            bpy.data.objects.remove(obj, do_unlink=True)
            if mesh is not None and mesh.users == 0:
                bpy.data.meshes.remove(mesh)
            gone += 1
    return gone
