"""Material names that travel with geometry.

The engine does not shade anything; it only has to say which faces use which material. A geometry carries that as

* the primitive attribute ``material_index`` (an int per face), and
* the detail attribute ``materials``: a list of material names, where ``materials[i]`` is the name for index ``i``.

A name of ``""`` means "no material of ours": whatever the target already has in that slot stays (a Blender slot, a USD
binding). Index ``i`` past the end of the list means the same. Geometry with a ``material_index`` and no list behaves as it
always did: Blender gets the integer, nothing else changes.

Everything that combines geometry from several inputs goes through :func:`unify`, so the indices of every input mean the
same thing in the result.
"""
from __future__ import annotations

from typing import List, Sequence

import numpy as np

KEY = "materials"
ATTR = "material_index"
MAX_NAME = 63                      # Blender's limit for a data-block name


def clean(name) -> str:
    """A tidy material name: text, no surrounding spaces, at most Blender's limit."""
    return str(name if name is not None else "").strip()[:MAX_NAME]


def names(geo) -> List[str]:
    """The geometry's material names (empty when it has none)."""
    raw = geo.detail.get(KEY)
    if not isinstance(raw, (list, tuple)):
        return []
    return [clean(n) for n in raw]


def name_of(geo, index: int) -> str:
    n = names(geo)
    return n[index] if 0 <= index < len(n) else ""


def used(geo) -> List[str]:
    """Names (non-empty, in slot order) that at least one face actually uses."""
    lst = names(geo)
    a = geo.prim.get(ATTR)
    if not lst or a is None or not len(a):
        return []
    idx = np.unique(np.asarray(a).reshape(len(a), -1)[:, 0].astype(np.int64))
    return [lst[i] for i in idx if 0 <= i < len(lst) and lst[i]]


def slot_for(lst: List[str], name: str) -> int:
    """Index of ``name`` in ``lst``, appending it when missing. The list is changed in place."""
    name = clean(name)
    if name in lst:
        return lst.index(name)
    lst.append(name)
    return len(lst) - 1


def indices(geo) -> np.ndarray:
    """``material_index`` as int64, one per primitive (zeros when the attribute is missing)."""
    a = geo.prim.get(ATTR)
    if a is None:
        return np.zeros(geo.nprims, np.int64)
    return np.asarray(a).reshape(geo.nprims, -1)[:, 0].astype(np.int64)


def assign(geo, mask, name: str):
    """A copy of ``geo`` where the primitives in ``mask`` (``None``: all) use material ``name``. Faces outside the mask keep
    what they had. The name list grows by one when the name is new; slot 0 stays whatever it was (``""`` when the geometry
    had no list)."""
    out = geo.copy()
    lst = names(geo)
    idx = indices(geo)
    top = int(idx.max()) + 1 if len(idx) else 0
    while len(lst) < max(top, 1):
        lst.append("")                                   # untouched faces keep meaning "no material of ours"
    slot = slot_for(lst, name)
    sel = np.ones(geo.nprims, bool) if mask is None else np.asarray(mask, bool)
    idx = np.where(sel, slot, idx)
    out.prim[ATTR] = idx.astype(np.int32)
    out.detail[KEY] = _compact(lst, idx)
    return out


def _compact(lst: List[str], idx: np.ndarray) -> List[str]:
    """The list, with trailing empty names that no face uses dropped (keeps the detail attribute short)."""
    n = len(lst)
    top = int(idx.max()) + 1 if len(idx) else 0
    while n > max(top, 1) and not lst[n - 1]:
        n -= 1
    return lst[:n]


def combine(lists: Sequence[Sequence[str]], indices: Sequence[np.ndarray], first_empty: bool = False):
    """One name list for several inputs: ``(combined, remapped index arrays)``. Index ``i`` of an input with names ``L`` means
    ``L[i]`` (or ``""`` past its end); in the result it means ``combined[j]`` for the same name. ``first_empty`` makes slot 0
    ``""``, for inputs that have no index array at all (they count as zeros)."""
    combined: List[str] = [""] if first_empty else []
    out = []
    for lst, idx in zip(lists, indices):
        idx = np.asarray(idx, np.int64)
        top = max(len(lst), int(idx.max()) + 1 if len(idx) else 0, 1)
        padded = list(lst) + [""] * (top - len(lst))
        lut = np.array([slot_for(combined, n) for n in padded], np.int64)
        out.append(lut[idx].astype(np.int32))
    return combined, out


def merge_sources(geos: Sequence, out) -> None:
    """Where material names were read from (``detail['usd_materials']``): the union over the inputs, first one wins."""
    merged: dict = {}
    for g in geos:
        src = g.detail.get("usd_materials")
        if isinstance(src, dict):
            for k, v in src.items():
                merged.setdefault(k, v)
    if merged:
        out.detail["usd_materials"] = merged


def unify(geos: Sequence):
    """The inputs, with their material indices rewritten to share one name list (stored in each one's detail). Inputs that
    already agree are returned untouched, so the common case costs nothing."""
    geos = list(geos)
    lists = [names(g) for g in geos]
    if not any(lists) or all(lst == lists[0] for lst in lists):
        return geos                                       # no names anywhere, or the same names: the indices already agree
    combined, remapped = combine(lists, [indices(g) for g in geos])
    out = []
    for g, idx in zip(geos, remapped):
        h = g.copy()
        h.prim[ATTR] = idx
        h.detail[KEY] = list(combined)
        out.append(h)
    return out
