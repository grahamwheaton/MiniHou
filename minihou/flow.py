"""Attribute flow: what a node did to the attributes of the geometry that went through it.

Nothing is declared per node type: the cooked output is compared with the cooked inputs, so the answer is true for
built-in nodes, library nodes and systems alike. Attributes are keyed by (domain, name):

* ``added``     not on any input (under that name, on any domain)
* ``modified``  on an input with the same domain and name, but different values (or type, or length)
* ``removed``   on an input, gone from the output
* ``moved``     the name changed domain (e.g. a wrangle promoting a point attribute to primitives)

plus how the element counts and the topology changed against the first input.
"""
from __future__ import annotations

from typing import Any, Dict, List, Optional, Sequence

import numpy as np

from .geometry import Geometry
from .link.diff import arrays_equal

DOMAINS = ("point", "vertex", "prim")


def _info(domain: str, name: str, a: np.ndarray) -> Dict[str, Any]:
    return {"name": name, "domain": domain, "type": str(a.dtype), "components": 1 if a.ndim == 1 else int(a.shape[1])}


def _table(geo: Geometry) -> Dict[tuple, np.ndarray]:
    return {(dom, name): arr for dom in DOMAINS for name, arr in geo.table(dom).items()}


def attribute_flow(out: Geometry, ins: Sequence[Optional[Geometry]]) -> Dict[str, Any]:
    ins = [g for g in ins if g is not None]
    before: Dict[tuple, np.ndarray] = {}
    for g in ins:                                           # the first input that has an attribute is the reference
        for key, arr in _table(g).items():
            before.setdefault(key, arr)
    after = _table(out)
    names_before = {}
    for (dom, name) in before:
        names_before.setdefault(name, dom)
    added, modified, removed, moved = [], [], [], []
    unchanged = 0
    for (dom, name), arr in after.items():
        if (dom, name) in before:
            if arrays_equal(np.ascontiguousarray(before[(dom, name)]), np.ascontiguousarray(arr)):
                unchanged += 1
            else:
                modified.append(_info(dom, name, arr))
        elif name in names_before:
            moved.append(dict(_info(dom, name, arr), **{"from": names_before[name]}))
        else:
            added.append(_info(dom, name, arr))
    names_after = {name for (_, name) in after}
    for (dom, name), arr in before.items():
        if name not in names_after:
            removed.append(_info(dom, name, arr))
    d_in: Dict[str, Any] = {}
    for g in ins:
        for k, v in g.detail.items():
            d_in.setdefault(k, v)
    for k, v in out.detail.items():
        rec = {"name": k, "domain": "detail"}
        if k not in d_in:
            added.append(rec)
        elif repr(d_in[k]) != repr(v):
            modified.append(rec)
        else:
            unchanged += 1
    for k in d_in:
        if k not in out.detail:
            removed.append({"name": k, "domain": "detail"})
    first = ins[0] if ins else None
    counts = {"points": [first.npoints if first else 0, out.npoints],
              "prims": [first.nprims if first else 0, out.nprims]}
    topology = "same"
    if first is None or out.topology.nprims != first.topology.nprims or not (
            arrays_equal(out.topology.vertex_point, first.topology.vertex_point)
            and arrays_equal(out.topology.prim_offsets, first.topology.prim_offsets)):
        topology = "changed" if first is not None else "new"
    for group in (added, modified, removed, moved):
        group.sort(key=lambda a: (a["domain"], a["name"]))
    return {"added": added, "modified": modified, "removed": removed, "moved": moved, "unchanged": unchanged,
            "counts": counts, "topology": topology, "inputs": len(ins)}


def summary_line(flow: Dict[str, Any]) -> str:
    """One readable line, e.g. ``+Cd ~P -N``."""
    parts: List[str] = []
    parts += ["+" + a["name"] for a in flow["added"]]
    parts += ["~" + a["name"] for a in flow["modified"]]
    parts += [f"{a['from']}>{a['domain']}:{a['name']}" for a in flow["moved"]]
    parts += ["-" + a["name"] for a in flow["removed"]]
    return " ".join(parts)
