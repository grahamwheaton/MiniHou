"""The geometry spreadsheet: a window of rows of one domain (points, vertices, primitives, edges, detail), for the editor's spreadsheet
panel. Rows are element numbers as every node and the viewer number them; values are rounded for display (6 significant digits)."""
from __future__ import annotations

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


def page(geo, domain: str, start: int = 0, count: int = 100, ids: Optional[Sequence[int]] = None) -> Dict[str, Any]:
    """``{"domain", "total", "start", "columns", "rows"}``: rows are ``[element number, value, value, ...]``. ``ids`` restricts the rows
    to those element numbers (the viewport's selection); ``start`` and ``count`` then page through them."""
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
    if ids is not None:
        pick = np.unique(np.asarray(list(ids), np.int64))
        pick = pick[(pick >= 0) & (pick < n)]
        total = len(pick)
        rows_idx = pick[start:start + count]
    else:
        total = n
        rows_idx = np.arange(start, min(start + count, n))
    columns = [_col(k, np.asarray(a), computed=k in ("P", "point", "prim", "size", "closed", "a", "b", "faces")) for k, a in cols.items()]
    rows: List[list] = []
    arrays = [np.asarray(a) for a in cols.values()]
    for i in rows_idx.tolist():
        rows.append([i] + [_cell(a, i) for a in arrays])
    return {"domain": domain, "total": total, "start": start, "columns": columns, "rows": rows}
