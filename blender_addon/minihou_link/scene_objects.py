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
from .mesh_writer import assert_main_thread, set_corner_verts, set_positions

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


def _make_piece(name: str, ident: int, col):
    obj = bpy.data.objects.new(name, bpy.data.meshes.new(name))      # each piece has its own mesh
    obj[sp.MARK_ID] = int(ident)
    obj[sp.MARK_SRC] = ""
    obj.rotation_mode = "XYZ"
    col.objects.link(obj)
    return obj


def _write_piece(mesh, P: np.ndarray, vp: np.ndarray, sizes: np.ndarray) -> None:
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


def _piece_slices(manifest: dict, arrays: dict):
    """Per manifest entry: (id, P, vp, sizes) views into the arrays."""
    pb = np.concatenate([[0], np.cumsum(manifest["npts"])]).astype(np.int64)
    cb = np.concatenate([[0], np.cumsum(manifest["ncorner"])]).astype(np.int64)
    fb = np.concatenate([[0], np.cumsum(manifest["nprim"])]).astype(np.int64)
    P, vp, sizes = arrays["P"].reshape(-1, 3), arrays["vp"], arrays["sizes"]
    if pb[-1] != len(P) or cb[-1] != len(vp) or fb[-1] != len(sizes):
        raise ValueError("the piece arrays do not match the counts in the manifest")
    for k, ident in enumerate(manifest["id"]):
        yield int(ident), P[pb[k]:pb[k + 1]], vp[cb[k]:cb[k + 1]], sizes[fb[k]:fb[k + 1]]


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
    pieces = manifest.get("mode") == "pieces"
    arrays = arrays if arrays is not None else manifest.get("_arrays")
    if pieces and not arrays:
        raise ValueError("a pieces manifest arrived without its mesh arrays")
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
        obj = _make_piece(item.name, item.ident, col) if pieces else _make(item.name, item.src, tag, item.ident, col, missing_log)
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
    if pieces:                                            # shapes: only the pieces whose shape really changed are rewritten
        for ident, P, vp, sizes in _piece_slices(manifest, arrays):
            obj = objs.get(ident)
            if obj is None or obj.type != "MESH":
                continue
            h = hashlib.blake2b(digest_size=12)
            for a in (P, vp, sizes):
                h.update(np.ascontiguousarray(a).tobytes())
            digest = h.hexdigest()
            if obj.data.get(DIGEST) != digest:
                _write_piece(obj.data, P, vp, sizes)
                obj.data[DIGEST] = digest
                rewritten += 1
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
