"""The geometry spreadsheet: a window of rows of one domain (points, vertices, primitives, edges, detail), for the editor's spreadsheet
panel. Rows are element numbers as every node and the viewer number them; values are rounded for display (6 significant digits)."""
from __future__ import annotations

import re
import threading
import weakref
from typing import Any, Dict, List, Optional, Sequence

import numpy as np

from . import topo

DOMAINS = ("point", "vertex", "prim", "edge", "detail")
MAX_ROWS = 500


def _num(x) -> Any:
    if isinstance(x, (bool, np.bool_)):
        return int(x)
    if isinstance(x, (int, np.integer)):
        return int(x)
    f = float(x)
    return f if f != f or f in (float("inf"), float("-inf")) else float(f"{f:.6g}")


def _cell(a: np.ndarray, i: int) -> Any:
    v = a[i]
    return [_num(x) for x in v] if getattr(v, "ndim", 0) else _num(v)


def _col(name: str, a: np.ndarray, computed: bool = False) -> dict:
    return {"name": name, "comps": 1 if a.ndim == 1 else int(a.reshape(len(a), -1).shape[1]), "dtype": str(a.dtype), "computed": computed}


def _tables(geo, domain: str) -> Dict[str, np.ndarray]:
    """Name -> array of the columns of ``domain`` (built-in ones first)."""
    t = geo.topology
    out: Dict[str, np.ndarray] = {}
    if domain == "point":
        out["P"] = geo.P
        out.update({n: geo.point[n] for n in geo.point.names() if n != "P"})
    elif domain == "vertex":
        out["point"] = t.vertex_point
        out["prim"] = topo.prim_of_vertex(geo)
        out.update({n: geo.vertex[n] for n in geo.vertex.names()})
    elif domain == "prim":
        out["size"] = t.prim_sizes()
        out["closed"] = t.closed_mask().astype(np.uint8)
        out.update({n: geo.prim[n] for n in geo.prim.names()})
    elif domain == "edge":
        from . import edges
        e = topo.edges(geo)
        out["a"], out["b"] = e.points[:, 0], e.points[:, 1]
        out["faces"] = np.asarray(e.count)
        for n in geo.vertex.names():
            if n.startswith(edges.PREFIX):
                try:
                    out[n[len(edges.PREFIX):]] = edges.values(geo, n)
                except ValueError:
                    pass
    return out


def size(geo, domain: str) -> int:
    if domain == "point":
        return geo.npoints
    if domain == "vertex":
        return geo.nvertices
    if domain == "prim":
        return geo.nprims
    if domain == "edge":
        return len(topo.edges(geo).points)
    return len(geo.detail)


class FilterError(ValueError):
    """A filter or sort that cannot be applied to this geometry (unknown column or group, unreadable expression); the editor shows the message."""


_COMPS = {"x": 0, "y": 1, "z": 2, "0": 0, "1": 1, "2": 2, "len": -1}
_CMP = re.compile(r"^\s*([A-Za-z_][A-Za-z0-9_]*)(?:\.(x|y|z|0|1|2|len))?\s*(<=|>=|==|!=|<|>)\s*(-?(?:\d+\.?\d*|\.\d+)(?:[eE][-+]?\d+)?)\s*$")
_OPS = {"<": np.less, "<=": np.less_equal, ">": np.greater, ">=": np.greater_equal, "==": np.equal, "!=": np.not_equal}


def _numbers(a: np.ndarray, comp: int) -> np.ndarray:
    """One float64 number per element of a column: the first component, component ``comp``, or the length (``comp`` = -1) of a vector."""
    a = np.asarray(a)
    if a.dtype.kind not in "fiub":
        raise FilterError("only numeric columns can be sorted or compared")
    a = a.reshape(len(a), -1).astype(np.float64)
    if comp == -1:
        return np.linalg.norm(a, axis=1)
    if comp >= a.shape[1]:
        raise FilterError(f"the column has {a.shape[1]} component(s), not {comp + 1}")
    return a[:, comp]


def _compare(cols: Dict[str, np.ndarray], n: int, text: str) -> np.ndarray:
    """A filter for the domains without group syntax (vertices, edges): comparisons of a column with a number, joined by ``and`` / ``&&``,
    e.g. ``faces == 1 and a < 10`` or ``P.z > 0.5``."""
    keep = np.ones(n, bool)
    for term in re.split(r"\s*(?:&&|\band\b)\s*", text.strip()):
        m = _CMP.match(term)
        if not m:
            raise FilterError(f"cannot read {term!r}: write column <op> number (<, <=, >, >=, ==, !=), joined by 'and', e.g. faces == 1 and a < 10")
        name, comp, op, num = m.groups()
        if name not in cols:
            raise FilterError(f"no column {name!r}; columns: {', '.join(cols)}")
        keep &= _OPS[op](_numbers(cols[name], _COMPS.get(comp, 0)), float(num))
    return keep


def _filter_mask(geo, domain: str, cols: Dict[str, np.ndarray], n: int, text: str) -> np.ndarray:
    if domain in ("point", "prim"):
        from . import groups
        try:
            m = groups.mask(geo, domain, text)                # the group syntax: names, ranges, ^, and @expressions such as @P.z > 0
        except Exception as e:                                # GroupError, WrangleError...: the text is what is wrong
            raise FilterError(str(e)) from None
        return np.ones(n, bool) if m is None else np.asarray(m, bool)
    return _compare(cols, n, text)


_ORDERS: "weakref.WeakKeyDictionary" = weakref.WeakKeyDictionary()           # geometry -> {(domain, sort, filter): row order}
_ORDERS_LOCK = threading.Lock()


def _row_order(geo, domain: str, cols: Dict[str, np.ndarray], n: int, sort: Optional[dict], text: str) -> Optional[np.ndarray]:
    """The element numbers in display order after filtering and sorting, remembered per (frozen) geometry; None = all, in order."""
    if not sort and not text:
        return None
    key = (domain, (sort["column"], sort["comp"], sort["desc"]) if sort else None, text)
    with _ORDERS_LOCK:
        hit = _ORDERS.get(geo, {}).get(key)
    if hit is not None:
        return hit
    idx = np.arange(n)
    if text:
        idx = idx[_filter_mask(geo, domain, cols, n, text)]
    if sort:
        if sort["column"] not in cols:
            raise FilterError(f"no column {sort['column']!r}; columns: {', '.join(cols)}")
        v = _numbers(cols[sort["column"]], sort["comp"])[idx]
        nan = np.isnan(v)
        good, bad = idx[~nan], idx[nan]                                   # NaN last, either way
        key_ = v[~nan]
        order = np.argsort(-key_ if sort["desc"] else key_, kind="stable")      # ties keep ascending element order
        idx = np.concatenate([good[order], bad])
    if geo.frozen:
        with _ORDERS_LOCK:
            slot = _ORDERS.setdefault(geo, {})
            if len(slot) > 8:
                slot.clear()
            slot[key] = idx
    return idx


def _sort_spec(sort) -> Optional[dict]:
    if not sort:
        return None
    if not isinstance(sort, dict) or not isinstance(sort.get("column"), str):
        raise ValueError('"sort" must be {column, comp?, desc?}')
    comp = sort.get("comp", 0)
    comp = _COMPS.get(str(comp), None) if not isinstance(comp, int) or isinstance(comp, bool) else comp
    if comp is None or comp < -1 or comp > 2:
        raise ValueError('sort "comp" must be 0, 1, 2 or "len"')
    return {"column": sort["column"], "comp": int(comp), "desc": bool(sort.get("desc"))}


def page(geo, domain: str, start: int = 0, count: int = 100, ids: Optional[Sequence[int]] = None, sort=None, filter: str = "") -> Dict[str, Any]:  # noqa: A002
    """``{"domain", "total", "start", "columns", "rows"}``: rows are ``[element number, value, value, ...]``. ``ids`` restricts the rows
    to those element numbers (the viewport's selection); ``start`` and ``count`` then page through them. ``filter`` keeps the rows that match
    (points and primitives: the group syntax, e.g. ``@w > 0.5`` or ``0-99``; vertices and edges: ``column <op> number [and ...]``) and ``sort``
    (``{column, comp?, desc?}``) orders them; ``total`` counts the rows left."""
    if domain not in DOMAINS:
        raise ValueError(f"domain must be one of {', '.join(DOMAINS)}")
    count = max(0, min(int(count), MAX_ROWS))
    start = max(0, int(start))
    if domain == "detail":
        items = list(geo.detail.items())
        rows = [[i, k, v if isinstance(v, (int, float, str, bool)) else repr(v)] for i, (k, v) in enumerate(items)][start:start + count]
        return {"domain": domain, "total": len(items), "start": start, "columns": [{"name": "name", "comps": 1, "dtype": "str"},
                                                                                   {"name": "value", "comps": 1, "dtype": "str"}], "rows": rows}
    cols = _tables(geo, domain)
    n = size(geo, domain)
    order = _row_order(geo, domain, cols, n, _sort_spec(sort), str(filter or "").strip())
    if ids is not None:
        pick = np.unique(np.asarray(list(ids), np.int64))
        pick = pick[(pick >= 0) & (pick < n)]
        if order is not None:                                              # the selection, in the table's order
            chosen = np.zeros(n, bool)
            chosen[pick] = True
            pick = order[chosen[order]]
        total = len(pick)
        rows_idx = pick[start:start + count]
    elif order is not None:
        total = len(order)
        rows_idx = order[start:start + count]
    else:
        total = n
        rows_idx = np.arange(start, min(start + count, n))
    columns = [_col(k, np.asarray(a), computed=k in ("P", "point", "prim", "size", "closed", "a", "b", "faces")) for k, a in cols.items()]
    rows: List[list] = []
    arrays = [np.asarray(a) for a in cols.values()]
    for i in rows_idx.tolist():
        rows.append([i] + [_cell(a, i) for a in arrays])
    return {"domain": domain, "total": total, "start": start, "columns": columns, "rows": rows}
