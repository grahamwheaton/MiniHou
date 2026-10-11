"""Viewport handles (gizmos): draggable arrows in the 3-D view that change a node's parameters.

A provider is a function ``(inputs, output, params) -> [handle, ...]`` for one node type, run when a node is previewed.
A handle is ``{id, label, param, comp, origin, dir, scale, min, max}``: an arrow from ``origin`` along the unit vector
``dir``; dragging it ``d`` world units along ``dir`` sets the parameter to ``value + d * scale`` (``comp`` picks the
component of a vector parameter, None for a plain number). Providers only read geometry, never change anything, and
any failure just means no handles: they must never get in the way of a preview.
"""
from __future__ import annotations

from typing import Callable, Dict, List

import numpy as np

AXES = np.eye(3)
Provider = Callable[[list, object, dict], List[dict]]
PROVIDERS: Dict[str, Provider] = {}


def provider(node_type: str):
    def deco(fn: Provider) -> Provider:
        PROVIDERS[node_type] = fn
        return fn
    return deco


def handles_for(node_type: str, inputs: list, output, params: dict) -> List[dict]:
    fn = PROVIDERS.get(node_type)
    if fn is None or output is None:
        return []
    try:
        return [_clean(h) for h in fn(inputs, output, params)]
    except Exception:  # noqa: BLE001 - a gizmo must never break the preview
        return []


def _clean(h: dict) -> dict:
    out = dict(h)
    out["origin"] = [float(x) for x in np.asarray(h["origin"], np.float64).reshape(3)]
    d = np.asarray(h["dir"], np.float64).reshape(3)
    n = float(np.linalg.norm(d))
    out["dir"] = [float(x) for x in (d / n if n > 0 else AXES[0])]
    out["scale"] = float(h.get("scale", 1.0))
    out["comp"] = None if h.get("comp") is None else int(h["comp"])
    for k in ("min", "max"):
        if h.get(k) is not None:
            out[k] = float(h[k])
    return out


def _center(geo) -> np.ndarray:
    if not geo.npoints:
        return np.zeros(3)
    P = geo.P.astype(np.float64)
    return (P.min(0) + P.max(0)) / 2.0


def _mean_normal(geo, prims) -> np.ndarray:
    from .nodes.normal import prim_normals
    n = prim_normals(geo).astype(np.float64)[prims].sum(0)
    ln = float(np.linalg.norm(n))
    return n / ln if ln > 0 else AXES[2]


@provider("transform")
def _transform(ins, out, p):
    c = _center(out)
    return [{"id": f"translate_{'xyz'[i]}", "label": f"move {'xyz'[i]}", "param": "translate", "comp": i, "origin": c,
             "dir": AXES[i]} for i in range(3)]


@provider("polyextrude")
def _polyextrude(ins, out, p):
    from . import groups
    src = ins[0] if ins else None
    if src is None or not src.nprims:
        return []
    sel = groups.mask(src, "prim", p.get("group", ""))
    prims = np.flatnonzero(sel) if sel is not None else np.arange(src.nprims)
    if not len(prims):
        return []
    t = src.topology
    vp = t.vertex_point.astype(np.int64)
    offs = t.prim_offsets.astype(np.int64)
    pts = np.unique(np.concatenate([vp[offs[q]:offs[q + 1]] for q in prims.tolist()]))
    origin = src.P.astype(np.float64)[pts].mean(0)
    return [{"id": "distance", "label": "distance", "param": "distance", "comp": None, "origin": origin,
             "dir": _mean_normal(src, prims)}]


@provider("polybevel")
def _polybevel(ins, out, p):
    from . import groups
    from .nodes.bevel import twins
    src = ins[0] if ins else None
    if src is None or not src.nprims:
        return []
    picked = groups.parse_edges(p.get("edges", ""), src.npoints)
    if not len(picked):
        return []
    P = src.P.astype(np.float64)
    origin = P[picked.reshape(-1)].mean(0)
    a, b = picked[0]
    t = src.topology
    vp = t.vertex_point.astype(np.int64)
    offs = t.prim_offsets.astype(np.int64)
    # the first face that has this edge: the arrow runs from the edge into that face
    face = None
    for q in range(t.nprims):
        v = vp[offs[q]:offs[q + 1]]
        if a in v and b in v:
            face = q
            break
    if face is None:
        return []
    inside = P[vp[offs[face]:offs[face + 1]]].mean(0) - (P[a] + P[b]) / 2.0
    return [{"id": "distance", "label": "distance", "param": "distance", "comp": None, "origin": origin, "dir": inside,
             "min": 0.0}]


@provider("sphere")
def _sphere(ins, out, p):
    c = np.asarray(p.get("center", (0, 0, 0)), np.float64)
    r = float(p.get("radius", 1.0))
    return [{"id": "radius", "label": "radius", "param": "radius", "comp": None, "origin": c + [r, 0, 0], "dir": AXES[0],
             "min": 0.0}]


@provider("box")
def _box(ins, out, p):
    c = np.asarray(p.get("center", (0, 0, 0)), np.float64)
    s = np.asarray(p.get("size", (1, 1, 1)), np.float64)
    return [{"id": f"size_{'xyz'[i]}", "label": f"size {'xyz'[i]}", "param": "size", "comp": i, "origin": c + AXES[i] * s[i] / 2,
             "dir": AXES[i], "scale": 2.0, "min": 0.0} for i in range(3)]
