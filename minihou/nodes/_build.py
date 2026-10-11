"""Building new geometry from old: gathering and interpolating attribute tables (used by the modelling nodes).

Interpolation rules, the same everywhere: floats blend linearly; ints and bools (ids, materials, groups) do not blend --
a blended element takes the value of its first source, and an average over several sources takes their minimum (so a
new point between group members is a member, one next to a non-member is not).
"""
from __future__ import annotations

from typing import Dict, Optional

import numpy as np

from ..geometry import Geometry, Topology


def gather(table, index: np.ndarray) -> Dict[str, np.ndarray]:
    return {name: a[index] for name, a in table.items()}


def lerp(table, ia: np.ndarray, ib: np.ndarray, t: np.ndarray) -> Dict[str, np.ndarray]:
    """Blend rows ia -> ib by t (floats); ints/bools take row ia."""
    out = {}
    for name, a in table.items():
        if a.dtype.kind == "f":
            tt = t.reshape((-1,) + (1,) * (a.ndim - 1)).astype(np.float32)
            out[name] = (a[ia] * (1 - tt) + a[ib] * tt).astype(np.float32)
        else:
            out[name] = a[ia]
    return out


def mean_by(table, values_index: np.ndarray, groups: np.ndarray, n: int) -> Dict[str, np.ndarray]:
    """Per-bucket combination of rows ``table[values_index]`` grouped by ``groups`` (n buckets): floats average,
    ints/bools take the minimum."""
    out = {}
    counts = np.maximum(np.bincount(groups, minlength=n), 1).astype(np.float64)
    for name, a in table.items():
        v = a[values_index]
        if a.dtype.kind == "f":
            flat = v.reshape(len(v), -1).astype(np.float64)
            s = np.stack([np.bincount(groups, weights=flat[:, c], minlength=n) for c in range(flat.shape[1])], 1)
            r = (s / counts[:, None]).astype(np.float32)
            out[name] = r[:, 0] if a.ndim == 1 else r
        else:                                              # sort + reduceat: much faster than np.minimum.at
            flat = v.reshape(len(v), -1)
            r = np.zeros((n, flat.shape[1]), a.dtype)
            if len(flat):
                order = np.argsort(groups, kind="stable")
                g = groups[order]
                starts = np.flatnonzero(np.r_[True, g[1:] != g[:-1]])
                r[g[starts]] = np.minimum.reduceat(flat[order], starts, axis=0)
            out[name] = r[:, 0] if a.ndim == 1 else r
    return out


def concat(parts) -> Dict[str, np.ndarray]:
    """Concatenate attribute dicts that share names (names missing from a part are skipped for all)."""
    parts = [p for p in parts if p is not None]
    if not parts:
        return {}
    names = set(parts[0])
    for p in parts[1:]:
        names &= set(p)
    return {n: np.concatenate([p[n] for p in parts]) for n in parts[0] if n in names}


def make(P: np.ndarray, vertex_point: np.ndarray, prim_offsets: np.ndarray, point: Optional[dict] = None,
         vertex: Optional[dict] = None, prim: Optional[dict] = None, detail: Optional[dict] = None,
         closed: Optional[np.ndarray] = None) -> Geometry:
    """A Geometry from raw arrays; ``point`` may contain P (it wins over the ``P`` argument if both given).
    ``closed`` (bool per primitive) marks open curves; None = all closed polygons."""
    g = Geometry(len(P), Topology(np.asarray(vertex_point, np.int32), np.asarray(prim_offsets, np.int32), closed))
    g.point["P"] = np.asarray(P, np.float32)
    for name, a in (point or {}).items():
        if name != "P":
            g.point[name] = a
    for name, a in (vertex or {}).items():
        g.vertex[name] = a
    for name, a in (prim or {}).items():
        g.prim[name] = a
    g.detail = dict(detail or {})
    return g


def closed_subset(geo, prim_index) -> Optional[np.ndarray]:
    """The closed flags of the primitives ``prim_index`` (None when the geometry has no open curves)."""
    c = geo.topology.closed
    return None if c is None else c[np.asarray(prim_index)]


def require_polygons(geo, what: str) -> None:
    """Raise a clear error for nodes that only work on polygons when the input has open curves."""
    if geo.topology.has_open:
        n = int((~geo.topology.closed).sum())
        raise ValueError(f"{what} works on polygons, but the input has {n} open curve(s); remove them first, e.g. "
                         f"blast with group '@closed == 0'")


def offsets_from_sizes(sizes) -> np.ndarray:
    return np.concatenate([[0], np.cumsum(np.asarray(sizes, np.int64))]).astype(np.int32)
