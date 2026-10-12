"""Output groups: the groups a modelling node can create from what it just made, switched on and named in its parameters.

Houdini's modelling nodes end with an "Output Groups" section (PolyBevel: Point Fillet Polygons, Offset Edges...; PolyExtrude:
Front, Back, Side): a checkbox and a name per group. Here every one is a pair of parameters, ``outgrp_<key>`` (on or off) and
``outgrp_<key>_name``, declared with :func:`params` and written with :func:`write`, so any node (library nodes too) gets
the same section, the same naming and the same behaviour:

    OUT = OutGroups(OutGroup("front", "Front", "extrudefront", "prim", default=True), ...)

    @node("mynode", params=[..., *OUT.params()])
    def mynode(geo, ..., **og):
        ...
        return OUT.write(result, og, "front", is_front_mask)

``domain`` is ``"prim"``, ``"point"`` or ``"edge"`` (edge groups are the ``edge_group_<name>`` vertex attributes of
:mod:`minihou.edges`; give the mask over ``topo.edges(result)`` or point pairs with :func:`edge_mask`). A group that is
switched off is not written, an empty name is ignored, and an existing group of the same name is replaced.
"""
from __future__ import annotations

from dataclasses import dataclass
from typing import Dict, Iterable, List, Sequence

import numpy as np

from . import edges, groups, topo
from .sdk import Bool, Param, Str

DOMAINS = ("prim", "point", "edge")


@dataclass(frozen=True)
class OutGroup:
    key: str                  # parameter stem: outgrp_<key>, outgrp_<key>_name
    label: str                # shown next to the checkbox
    name: str                 # default group name
    domain: str = "prim"      # prim | point | edge
    default: bool = False     # on by default (only where a node always made the group before)
    doc: str = ""


class OutGroups:
    def __init__(self, *items: OutGroup):
        for g in items:
            if g.domain not in DOMAINS:
                raise ValueError(f"output group {g.key!r}: domain must be one of {DOMAINS}")
        keys = [g.key for g in items]
        if len(set(keys)) != len(keys):
            raise ValueError(f"duplicate output group keys in {keys}")
        self.items = items
        self._by_key = {g.key: g for g in items}

    def params(self) -> List[Param]:
        """The parameter pairs, for the node's ``params=[..., *OUT.params()]``."""
        out: List[Param] = []
        for g in self.items:
            doc = g.doc or f"Make a {g.domain} group of the {g.label.lower()}"
            out.append(Bool(f"outgrp_{g.key}", g.default, label=g.label, doc=doc))
            out.append(Str(f"outgrp_{g.key}_name", g.name, label=f"{g.label} name", doc=f"Name of the {g.label.lower()} group"))
        for p in out:
            object.__setattr__(p, "ui", "outgroup" if p.kind == "bool" else "outgroup_name")
        return out

    def settings(self, og: Dict, key: str):
        """(enabled, name) for one group from the node's keyword arguments."""
        g = self._by_key[key]
        return bool(og.get(f"outgrp_{key}", g.default)), str(og.get(f"outgrp_{key}_name", g.name) or "").strip()

    def enabled(self, og: Dict, key: str) -> bool:
        on, name = self.settings(og, key)
        return on and bool(name)

    def any_enabled(self, og: Dict, *keys: str) -> bool:
        return any(self.enabled(og, k) for k in (keys or self._by_key))

    def write(self, geo, og: Dict, key: str, members):
        """``geo`` with the group written when it is switched on (else ``geo`` itself). ``members`` is a bool array over the
        elements of the group's domain (for edges: over ``topo.edges(geo)``), or a callable returning one (so an unused group costs nothing)."""
        on, name = self.settings(og, key)
        if not on or not name:
            return geo
        m = np.asarray(members() if callable(members) else members, bool)
        domain = self._by_key[key].domain
        if domain == "edge":
            return edges.write(geo, edges.PREFIX + _checked(name), m.astype(np.int32))
        return groups.write(geo, domain, name, m)


def _checked(name: str) -> str:
    groups.attr_name(name)                                   # raises a clear error for an invalid name
    return name


def edge_mask(geo, pairs) -> np.ndarray:
    """Bool per derived edge of ``geo`` (``topo.edges`` order): which of the point pairs ``(a, b)`` exist as edges."""
    pairs = np.asarray(pairs, np.int64).reshape(-1, 2)
    e = topo.edges(geo)
    if not len(pairs) or not len(e.points):
        return np.zeros(len(e.points), bool)
    n = max(geo.npoints, 1)
    want = np.minimum(pairs[:, 0], pairs[:, 1]) * n + np.maximum(pairs[:, 0], pairs[:, 1])
    return np.isin(e.points[:, 0] * n + e.points[:, 1], want)


def names_of(geo) -> Dict[str, List[str]]:
    """Every group name on the geometry by domain (for tests and tools)."""
    return {"prim": groups.names(geo, "prim"), "point": groups.names(geo, "point"),
            "edge": sorted(n[len(edges.PREFIX):] for n in geo.vertex if n.startswith(edges.PREFIX))}
