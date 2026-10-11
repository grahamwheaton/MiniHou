"""Readable summaries of cooked geometry, for the editor's inspector and for AI tools (JSON-friendly)."""
from __future__ import annotations

from typing import Any, Dict

import numpy as np

from .geometry import Geometry

DOMAINS = ("point", "vertex", "prim")


def _r(x: float) -> float:
    return float(f"{float(x):.6g}")


def summarize(geo: Geometry, detail: bool = False, sample: int = 3) -> Dict[str, Any]:
    """Counts, bounds and attributes. ``detail`` adds per-component min/max/mean and the first values."""
    out: Dict[str, Any] = {"points": geo.npoints, "vertices": geo.nvertices, "prims": geo.nprims}
    if geo.nprims:
        sizes = geo.topology.prim_sizes()
        out["verts_per_prim"] = [int(sizes.min()), int(sizes.max())]
    if geo.npoints:
        P = geo.P
        lo, hi = P.min(axis=0), P.max(axis=0)
        out["bounds"] = {"min": [_r(v) for v in lo], "max": [_r(v) for v in hi],
                         "size": [_r(v) for v in hi - lo], "center": [_r(v) for v in (lo + hi) / 2]}
    attrs = []
    for dom in DOMAINS:
        table = geo.table(dom)
        for name in table.names():
            a = table[name]
            info: Dict[str, Any] = {"name": name, "domain": dom, "type": str(a.dtype),
                                    "components": 1 if a.ndim == 1 else int(a.shape[1])}
            if detail and len(a) and a.dtype.kind in "fiub":
                cols = a.reshape(len(a), -1).astype(np.float64)
                info["min"] = [_r(v) for v in cols.min(axis=0)]
                info["max"] = [_r(v) for v in cols.max(axis=0)]
                info["mean"] = [_r(v) for v in cols.mean(axis=0)]
                info["first"] = [[_r(v) for v in row] if a.ndim > 1 else _r(row) for row in a[:sample]]
            attrs.append(info)
    if geo.detail:
        for name, v in geo.detail.items():
            attrs.append({"name": name, "domain": "detail", "value": v if isinstance(v, (int, float, str, bool)) else repr(v)})
    out["attributes"] = attrs
    return out
