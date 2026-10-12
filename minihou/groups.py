"""Groups: named selections of points or primitives, as in Houdini.

A group is an int attribute ``group_<name>`` (1 = member) on the point or primitive domain, so wrangles read it as
``@group_top`` exactly like VEX, it survives every node that carries attributes, and it reaches Blender as a boolean
attribute of the same name (usable in Geometry Nodes with *Named Attribute*).

Group fields on nodes take a **pattern**, like Houdini's group parameter:

* ``top side``      -- union of groups (``*`` and ``?`` wildcards work: ``rock_*``)
* ``^side``         -- remove a group from what came before; a pattern that *starts* with ``^`` starts from everything
  (``^side`` = everything except ``side``)
* ``0-9 20``        -- element numbers and inclusive ranges (``0-99:2`` = every second one)
* ``@P.z > 0``      -- an ad-hoc group: a wrangle expression over the domain (the whole field)
* empty             -- everything (the node acts on all elements); comma/whitespace-only fields are also empty

Commas separate pattern tokens; repeated separators are ignored. Range strides must be positive integers.
Attribute expressions may be parenthesized, e.g. ``(@group_top)`` or ``((@P.z))``.

A pattern naming a group of the *other* domain converts it: points of a primitive group are the points its primitives
use; primitives of a point group are those whose points are *all* in it. If Merge supplies the same name on both
domains, ordinary selection prefers the requested domain. Merge preserves both attributes; Group Promote explicitly
converts the opposite (source) domain, replaces the destination group, and removes the source group.
Blast treats an empty field (including separators only) as selecting nothing rather than everything.
"""
from __future__ import annotations

import fnmatch
import re
from typing import Optional

import numpy as np

PREFIX = "group_"
_RANGE = re.compile(r"^(\d+)(?:-(\d+))?(?::(.*))?$")
_NAME = re.compile(r"^[A-Za-z_][A-Za-z0-9_]*$")


class GroupError(ValueError):
    pass


def attr_name(group: str) -> str:
    if not _NAME.match(group or ""):
        raise GroupError(f"invalid group name {group!r} (letters, digits and _, not starting with a digit)")
    return PREFIX + group


def names(geo, domain: str):
    """Group names on a domain (without the prefix)."""
    return sorted(n[len(PREFIX):] for n in geo.table(domain).names() if n.startswith(PREFIX))


def member(geo, domain: str, group: str) -> Optional[np.ndarray]:
    """Bool membership of a group on ``domain`` (converting from the other domain), or None if there is no such group."""
    key = PREFIX + group
    for src in (domain, "prim" if domain == "point" else "point"):
        a = geo.table(src).get(key)
        if a is not None:
            return convert(geo, (a if a.ndim == 1 else a[:, 0]) != 0, src, domain)
    return None


def convert(geo, mask: np.ndarray, src: str, dst: str) -> np.ndarray:
    """Convert a membership mask between the point and primitive domains."""
    if src == dst:
        return mask
    topo = geo.topology
    prim_of_v = np.repeat(np.arange(topo.nprims), topo.prim_sizes())
    if src == "prim":                                       # points used by selected prims
        out = np.zeros(geo.npoints, bool)
        out[topo.vertex_point[mask[prim_of_v]]] = True
        return out
    missing = np.bincount(prim_of_v, weights=(~mask[topo.vertex_point]).astype(np.float64), minlength=topo.nprims)
    return (missing == 0) & (topo.prim_sizes() > 0)        # prims whose points are all selected


def mask(geo, domain: str, pattern: str) -> Optional[np.ndarray]:
    """Elements of ``domain`` ("point" or "prim") selected by a group pattern; None means *all* (empty pattern)."""
    if domain not in ("point", "prim"):
        raise GroupError(f"groups live on points or primitives, not {domain!r}")
    pattern = (pattern or "").strip()
    if not pattern:
        return None
    n = geo.npoints if domain == "point" else geo.nprims
    if re.match(r"^(?:\(\s*)*@", pattern) or any(op in pattern for op in ("<", ">", "==", "!=")):
        return expression_mask(geo, domain, pattern)
    tokens = pattern.replace(",", " ").split()
    if not tokens:
        return None
    sel = np.full(n, tokens[0].startswith("^"))           # a pattern that starts by excluding starts from everything
    known = set(names(geo, "point")) | set(names(geo, "prim"))
    for tok in tokens:
        sub = tok.startswith("^")
        tok = tok[1:] if sub else tok
        if not tok:
            continue
        m = _token(geo, domain, tok, n, known)
        sel = (sel & ~m) if sub else (sel | m)
    return sel


def _token(geo, domain, tok, n, known) -> np.ndarray:
    r = _RANGE.match(tok)
    if r:
        lo = int(r.group(1))
        hi = int(r.group(2)) if r.group(2) is not None else lo
        stride = r.group(3)
        if stride is not None and (not re.fullmatch(r"\d+", stride) or int(stride) <= 0):
            raise GroupError(f"invalid range stride in {tok!r}; expected a positive integer")
        step = int(stride) if stride is not None else 1
        m = np.zeros(n, bool)
        if lo < n:
            m[lo:min(hi, n - 1) + 1:step] = True
        return m
    if tok == "*":
        return np.ones(n, bool)
    if any(c in tok for c in "*?["):
        hits = [g for g in sorted(known) if fnmatch.fnmatchcase(g, tok)]
        m = np.zeros(n, bool)
        for g in hits:
            m |= member(geo, domain, g)
        return m
    got = member(geo, domain, tok)
    if got is None:
        have = sorted(known)
        raise GroupError(f"no group {tok!r}" + (f"; groups: {', '.join(have)}" if have else " (this geometry has no groups)"))
    return got


def expression_mask(geo, domain: str, expr: str) -> np.ndarray:
    """An ad-hoc group: a wrangle expression (``@P.z > 0``) over points or primitives."""
    from .nodes.wrangle import run_snippet
    if ";" in expr or "\n" in expr:
        raise GroupError("a group expression must be a single expression, e.g. @P.z > 0")
    flag = "__minihou_group"
    out = run_snippet(geo, f"i@{flag} = 1 if ({expr}) else 0", domain)
    return out[flag].astype(bool)


def write(geo, domain: str, group: str, members: np.ndarray, combine: str = "replace"):
    """Return a copy of ``geo`` with the group set (``combine``: replace, union, intersect, subtract)."""
    key = attr_name(group)
    old = member(geo, domain, group) if key in geo.table(domain) else None
    m = np.asarray(members, bool)
    if old is not None:
        if combine == "union":
            m = m | old
        elif combine == "intersect":
            m = m & old
        elif combine == "subtract":
            m = old & ~m
    elif combine in ("intersect", "subtract"):
        m = np.zeros_like(m)
    out = geo.copy()
    other = "prim" if domain == "point" else "point"
    if key in out.table(other):                             # one domain per name, like any attribute
        del out.table(other)[key]
    out.table(domain)[key] = m.astype(np.int32)
    return out


def restrict(before, after, domain: str, pattern: str):
    """Limit a node's changes to a group: attributes of ``domain`` that the node replaced keep their old values outside
    the selection (attributes it created are zero there). Topology must be unchanged. Returns ``after`` if no group."""
    sel = mask(before, domain, pattern)
    if sel is None:
        return after
    old_table, new_table = before.table(domain), after.table(domain)
    out = after.copy()
    for name in new_table.names():
        new = new_table[name]
        old = old_table.get(name)
        if old is new:
            continue
        if old is None or old.shape != new.shape:
            old = np.zeros_like(new)
        out.table(domain)[name] = np.where(sel.reshape((-1,) + (1,) * (new.ndim - 1)), new, old.astype(new.dtype))
    return out


def parse_edges(text: str, npoints: int):
    """An edge selection written as point pairs: ``"12-13 40-41, 7-8"`` -> (k, 2) int64 array of point numbers (each pair
    sorted). Empty text gives no edges. Raises :class:`GroupError` for anything that is not ``a-b`` or an unknown point."""
    out = []
    for tok in (text or "").replace(",", " ").split():
        m = re.match(r"^(\d+)-(\d+)$", tok)
        if not m:
            raise GroupError(f"bad edge {tok!r}: write point pairs like 12-13")
        a, b = int(m.group(1)), int(m.group(2))
        if a >= npoints or b >= npoints:
            raise GroupError(f"edge {tok!r}: the geometry has only {npoints} points")
        if a != b:
            out.append((min(a, b), max(a, b)))
    return np.array(sorted(set(out)), np.int64).reshape(-1, 2)


def format_edges(pairs) -> str:
    """The inverse of :func:`parse_edges` (what the editor writes when it makes a selection)."""
    return " ".join(f"{min(int(a), int(b))}-{max(int(a), int(b))}" for a, b in pairs)
