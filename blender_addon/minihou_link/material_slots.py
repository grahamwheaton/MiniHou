"""Which material goes in which slot (bpy-free, so it can be tested without Blender).

The engine sends a list of material names; entry ``i`` is for faces with ``material_index == i``. An empty name means "not
ours": the slot keeps whatever it has. Slots are only ever added, never removed or reordered, so the materials a user put
on the object stay.
"""
from __future__ import annotations

from typing import List, Optional, Sequence


def usable(names) -> List[str]:
    """The name list from the engine, as strings (anything else is ignored)."""
    if not isinstance(names, (list, tuple)):
        return []
    return [str(n) for n in names]


def plan(current: Sequence[Optional[str]], names) -> List[Optional[str]]:
    """The wanted slot contents, as material names (``None``: an empty slot). ``current`` is what the slots hold now."""
    names = usable(names)
    out: List[Optional[str]] = list(current)
    for i, name in enumerate(names):
        if not name:
            if i >= len(out):
                out.append(None)
            continue
        if i < len(out):
            out[i] = name
        else:
            out.append(name)
    return out


def changes(current: Sequence[Optional[str]], names) -> bool:
    """True when applying ``names`` would change a slot."""
    return plan(current, names) != list(current)
