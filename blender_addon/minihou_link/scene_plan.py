"""Deciding what to do with the objects of a ``scene_objects`` node. **No bpy in this module** (unit-tested outside Blender).

The engine sends a manifest: for each object an ``id`` (stable), the Blender object it duplicates (``src`` index into ``sources``)
and ``loc`` / ``rot`` / ``scale``. The add-on finds the objects it made earlier *by marker, never by name* (Blender renames
clashes, and a saved file brings them back), then :func:`plan` says which to create, move, retarget, show again and hide.
Objects that are no longer needed are hidden, not deleted: they may carry modifiers, materials and constraints the user added.
"""
from __future__ import annotations

from dataclasses import dataclass, field
from typing import Dict, List, Optional, Sequence

SHAPED = ("objects", "pieces", "curves", "points")    # modes whose objects get their geometry from the manifest's arrays (not a duplicate of a source)
LOC_EPS = 1e-5
ROT_EPS = 1e-6
SCALE_EPS = 1e-5

MARK = "minihou_scene"            # custom property: "<owner>:<node>" (owner: "session" or a modifier instance id)
MARK_ID = "minihou_id"            # the stable id inside the node
MARK_SRC = "minihou_src"          # name of the object it duplicates
MARK_HIDDEN = "minihou_hidden"    # 1 while the object is parked because the engine no longer asks for it
MARK_PLACEHOLDER = "minihou_placeholder"   # 1 while it is an empty standing in for a missing source


def owner_tag(owner: str, node: str) -> str:
    return f"{owner}:{node}"


def object_name(prefix: str, ident: int) -> str:
    """``Tree_0007``; a negative id (``-3``) becomes ``Tree_n0003``."""
    return f"{prefix}_{ident:04d}" if ident >= 0 else f"{prefix}_n{-ident:04d}"


@dataclass
class Existing:
    """What the add-on knows about one object it made earlier."""
    ident: int
    name: str
    src: str = ""
    hidden: bool = False
    placeholder: bool = False
    loc: Sequence[float] = (0.0, 0.0, 0.0)
    rot: Sequence[float] = (0.0, 0.0, 0.0)
    scale: Sequence[float] = (1.0, 1.0, 1.0)
    parent: str = ""              # name of the parent object ("" if none)


@dataclass
class Item:
    """One object the manifest asks for."""
    ident: int
    name: str
    src: str
    loc: Sequence[float]
    rot: Sequence[float]
    scale: Sequence[float]


@dataclass
class Plan:
    create: List[Item] = field(default_factory=list)
    update: List[Item] = field(default_factory=list)       # exists: move it (and show it if hidden)
    retarget: List[Item] = field(default_factory=list)     # exists but should duplicate another object (or its source appeared)
    unhide: List[int] = field(default_factory=list)
    hide: List[int] = field(default_factory=list)          # exists, is shown, and is no longer asked for
    duplicates: List[str] = field(default_factory=list)    # extra objects carrying an id that another object already has: hide them
    reparent: List[int] = field(default_factory=list)
    missing_sources: List[str] = field(default_factory=list)


def items(manifest: dict) -> List[Item]:
    sources = [str(s) for s in manifest.get("sources", [])]
    prefix = str(manifest.get("prefix") or "MiniHou")
    shaped = manifest.get("mode") in SHAPED                 # these carry their shape in the arrays and sit where the manifest says
    out: List[Item] = []
    seen = set()
    for i, ident in enumerate(manifest.get("id", [])):
        ident = int(ident)
        if ident in seen:                                   # the engine refuses repeated ids; be safe anyway
            continue
        seen.add(ident)
        if shaped:                                          # an object is placed at its centre; its mesh carries the shape
            out.append(Item(ident, object_name(prefix, ident), "", manifest["loc"][i], (0.0, 0.0, 0.0), (1.0, 1.0, 1.0)))
            continue
        s = int(manifest["src"][i])
        out.append(Item(ident, object_name(prefix, ident), sources[s] if 0 <= s < len(sources) else "",
                        manifest["loc"][i], manifest["rot"][i], manifest["scale"][i]))
    return out


def _close(a: Sequence[float], b: Sequence[float], eps: float) -> bool:
    return len(a) == len(b) and all(abs(float(x) - float(y)) <= eps for x, y in zip(a, b))


def changed(item: Item, ex: Existing) -> bool:
    """Whether writing the item's transform would change anything (writes tag the depsgraph, so skip no-ops)."""
    return not (_close(item.loc, ex.loc, LOC_EPS) and _close(item.rot, ex.rot, ROT_EPS) and _close(item.scale, ex.scale, SCALE_EPS))


def plan(manifest: dict, existing: Dict[int, Existing], present_sources: Sequence[str], parent: str = "",
         extras: Optional[Sequence[Existing]] = None) -> Plan:
    """What to do. ``existing``: the node's earlier objects by id; ``extras``: further objects with an id already in ``existing``
    (a copied-and-pasted object keeps its marker); ``present_sources``: names of Blender objects that exist now."""
    p = Plan()
    have = set(present_sources)
    wanted = items(manifest)
    wanted_ids = {it.ident for it in wanted}
    for it in wanted:
        if it.src and it.src not in have and it.src not in p.missing_sources:
            p.missing_sources.append(it.src)
        ex = existing.get(it.ident)
        if ex is None:
            p.create.append(it)
            continue
        if ex.src != it.src or (ex.placeholder and it.src in have):
            p.retarget.append(it)
        if ex.hidden:
            p.unhide.append(it.ident)
        if ex.hidden or changed(it, ex):
            p.update.append(it)
        if parent and parent != ex.parent:                  # only an owner-given parent is enforced; the user's own parenting is left alone
            p.reparent.append(it.ident)
    for ident, ex in existing.items():
        if ident not in wanted_ids and not ex.hidden:
            p.hide.append(ident)
    p.duplicates = [e.name for e in (extras or []) if not e.hidden]
    return p
