"""Viewport handles (gizmos): draggable arrows in the 3-D view that change a node's parameters.

A provider is a function ``(inputs, output, params) -> [handle, ...]`` for one node type, run when a node is previewed.
A handle is ``{id, label, param, comp, origin, dir, scale, min, max, kind, tool, radius}``. The default ``kind`` "arrow" runs
from ``origin`` along the unit vector ``dir``; dragging it ``d`` world units along ``dir`` sets the parameter to
``value + d * scale`` (``comp`` picks the component of a vector parameter, None for a plain number). Other kinds:
"ring" (a circle of ``radius`` around ``origin`` in the plane normal to ``dir``; turning it by ``a`` radians sets
``value + a * scale``), "scale" (an arrow ending in a cube; dragging multiplies the value by ``exp(pixels / 64)``)
and "uniform" (a square at ``origin``, same multiplication). ``tool`` ("move", "rotate", "scale") groups handles
under the viewer's tool buttons; without it a handle is always shown. Providers only read geometry, never change
anything, and any failure just means no handles: they must never get in the way of a preview.
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


def has_handles(node_type: str, nt=None) -> bool:
    """Whether a node type shows handles: a built-in provider and/or ``Handle`` specs declared with ``@node(handles=...)``."""
    return node_type in PROVIDERS or bool(getattr(nt, "handles", ()))


def handles_for(node_type: str, inputs: list, output, params: dict, nt=None) -> List[dict]:
    fn = PROVIDERS.get(node_type)
    specs = tuple(getattr(nt, "handles", ()) or ())
    if (fn is None and not specs) or output is None:
        return []
    out: List[dict] = []
    for make in ([lambda: fn(inputs, output, params)] if fn else []) + ([lambda: declared(specs, output, params)] if specs else []):
        try:                                      # each source on its own: a failing one just shows nothing
            out += [_clean(h) for h in make()]
        except Exception:  # noqa: BLE001 - a gizmo must never break the preview
            continue
    return out


def _bounds(geo):
    if not geo.npoints:
        return np.zeros(3), np.zeros(3), np.zeros(3)
    P = geo.P.astype(np.float64)
    lo, hi = P.min(0), P.max(0)
    return (lo + hi) / 2.0, lo, hi


def declared(specs, out, params) -> List[dict]:
    """Handles for the ``Handle`` specs of a node type (see :class:`minihou.sdk.Handle`), resolved against the current
    parameters and the node's output."""
    from .sdk import HANDLE_AXES
    c, lo, hi = _bounds(out)
    extent = float(np.linalg.norm(hi - lo) / 2.0)

    def vec(v, sign=1.0):
        return sign * np.asarray(v, np.float64).reshape(3)

    res = []
    for h in specs:
        value = params.get(h.param)
        if h.comp is not None:
            value = value[h.comp] if value is not None else None
        a = h.axis
        if isinstance(a, str):
            d = vec(params[a[1:]]) if a.startswith("@") else vec(HANDLE_AXES[a.lstrip("-")], -1.0 if a.startswith("-") else 1.0)
        else:
            d = vec(a)
        if float(np.linalg.norm(d)) < 1e-12:
            continue
        d = d / np.linalg.norm(d)
        at = h.at
        if isinstance(at, str):
            if at == "center":
                o = c
            elif at == "origin":
                o = np.zeros(3)
            elif at == "top":
                o = np.array([c[0], c[1], hi[2]])
            elif at == "bottom":
                o = np.array([c[0], c[1], lo[2]])
            else:
                o = vec(params[at[1:]])                 # "@name": a Vec3 parameter
        else:
            o = vec(at)
        if h.reach and h.kind == "arrow" and value is not None and h.scale:
            o = o + d * (float(value) / h.scale)
        item = {"id": h.key(), "label": h.label or h.key().replace("_", " "), "param": h.param, "comp": h.comp, "origin": o, "dir": d,
                "scale": h.scale, "kind": h.kind}
        if h.kind == "ring":
            item["radius"] = float(h.radius) if h.radius else max(extent * 0.6, 0.05)
        for k in ("min", "max"):
            if getattr(h, k) is not None:
                item[k] = getattr(h, k)
        if h.tool:
            item["tool"] = h.tool
        res.append(item)
    return res


def _clean(h: dict) -> dict:
    out = dict(h)
    out["origin"] = [float(x) for x in np.asarray(h["origin"], np.float64).reshape(3)]
    d = np.asarray(h["dir"], np.float64).reshape(3)
    n = float(np.linalg.norm(d))
    out["dir"] = [float(x) for x in (d / n if n > 0 else AXES[0])]
    out["scale"] = float(h.get("scale", 1.0))
    out["comp"] = None if h.get("comp") is None else int(h["comp"])
    out["kind"] = str(h.get("kind", "arrow"))
    if h.get("tool"):
        out["tool"] = str(h["tool"])
    if h.get("radius") is not None:
        out["radius"] = float(h["radius"])
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


def _extent(geo) -> float:
    """Half the diagonal of the geometry's bounds: a size for gizmos that must fit around it."""
    if not geo.npoints:
        return 1.0
    P = geo.P.astype(np.float64)
    return float(np.linalg.norm(P.max(0) - P.min(0)) / 2.0)


@provider("transform")
def _transform(ins, out, p):
    from . import groups
    from .nodes._util import euler_xyz_matrix
    c = _center(out)
    group = str(p.get("group", "") or "")
    if group:                                      # a grouped transform: the arrows stand on the points it moves (their bounds' centre)
        src = ins[0] if ins else out
        try:
            m = groups.mask(src, "point", group)
            if m is not None and m.any():
                P = out.P.astype(np.float64)[m]
                c = (P.min(0) + P.max(0)) / 2.0
        except Exception:  # noqa: BLE001 - a bad group just leaves the arrows at the middle of everything
            pass
    B = euler_xyz_matrix(p.get("axis_rotation", (0, 0, 0)))
    hs = [{"id": f"translate_{'xyz'[i]}", "label": f"move {'xyz'[i]}", "param": "translate", "comp": i, "origin": c,
           "dir": B[:, i], "tool": "move"} for i in range(3)]
    hs += [{"id": f"translate_plane_{i}", "label": f"move {'YZ XZ XY'.split()[i]}", "param": "translate", "comp": None, "origin": c, "dir": B[:, i], "axis_rotation": p.get("axis_rotation", (0,0,0)), "plane_axis": i, "kind": "plane", "tool": "move"} for i in range(3)]
    # rotate and scale sit on the fixed point of the transform (pivot + translate), so they do not run away while dragged
    rot = np.asarray(p.get("rotate", (0, 0, 0)), np.float64)
    R = euler_xyz_matrix(rot)
    anchor = np.asarray(p.get("pivot", (0, 0, 0)), np.float64) + B @ np.asarray(p.get("translate", (0, 0, 0)), np.float64)
    Rz = euler_xyz_matrix([0, 0, rot[2]])
    Ryz = euler_xyz_matrix([0, rot[1], rot[2]])
    rings = [B @ Ryz @ AXES[0], B @ Rz @ AXES[1], B[:, 2]]       # the axis each Euler angle turns about
    radius = max(_extent(out) * 0.6, 0.05)
    hs += [{"id": f"rotate_{'xyz'[i]}", "label": f"rotate {'xyz'[i]}", "param": "rotate", "comp": i, "origin": anchor, "dir": rings[i],
            "kind": "ring", "radius": radius, "scale": 180.0 / np.pi, "tool": "rotate"} for i in range(3)]
    hs += [{"id": f"scale_{'xyz'[i]}", "label": f"scale {'xyz'[i]}", "param": "scale", "comp": i, "origin": anchor, "dir": B @ R[:, i],
            "kind": "scale", "tool": "scale"} for i in range(3)]
    hs.append({"id": "uniform_scale", "label": "uniform scale", "param": "uniform_scale", "comp": None, "origin": anchor, "dir": AXES[2],
               "kind": "uniform", "tool": "scale"})
    return hs


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


@provider("match_size")
def _match_size(ins, out, p):
    """Target-box face controls; enabled operations only, matching evaluation."""
    center = np.asarray(p.get("center", (0, 0, 0)), dtype=float)
    size = np.asarray(p.get("size", (1, 1, 1)), dtype=float)
    result = []
    if p.get("translate", True):
        result += [{"id": f"center_{i}", "label": f"Target center {'XYZ'[i]}",
                    "param": "center", "comp": i, "origin": center, "dir": AXES[i], "tool": "move"} for i in range(3)]
    if p.get("scale", False):
        box_center = center.copy() if p.get("translate", True) else _bounds(ins[0])[0]
        if p.get("translate", True):
            box_center[2] += {"min": 0.5, "max": -0.5}.get(p.get("justify_z"), 0) * size[2]
        for i in range(3):
            for sign in (-1, 1):
                result.append({"id": f"target_size_{i}_{sign}", "label": f"Target size {'XYZ'[i]}",
                               "param": "size", "comp": i, "origin": box_center + sign * AXES[i] * size[i] / 2,
                               "dir": sign * AXES[i], "scale": 2.0, "min": 0.0001})
    if result and p.get("scale", False):
        corners = [box_center + np.asarray(v) * size / 2 for v in __import__('itertools').product((-1, 1), repeat=3)]
        result[0]['guide_lines'] = [[a.tolist(), b.tolist()] for j, a in enumerate(corners) for b in corners[j+1:] if np.count_nonzero(a != b) == 1]
    return result


@provider("lattice_grid")
def _lattice_grid(ins, out, p):
    # Dense cages remain editable numerically; avoid thousands of DOM handles.
    if out.npoints > 216:
        return []
    result = [{"id": f"control_{j}_{i}", "label": f"Control {j} {'XYZ'[i]}",
             "param": "offsets", "comp": j * 3 + i, "origin": point, "dir": AXES[i],
             "json_array": out.npoints * 3, "axis_color": i}
            for j, point in enumerate(out.P) for i in range(3)]

    nx, ny, nz = (int(p.get(k, 2)) for k in ('columns', 'rows', 'layers'))
    lines = []
    for z in range(nz):
        for y in range(ny):
            for x in range(nx):
                a = x + nx * (y + ny * z)
                for ok, step in ((x + 1 < nx, 1), (y + 1 < ny, nx), (z + 1 < nz, nx * ny)):
                    if ok:
                        lines.append([out.P[a].tolist(), out.P[a+step].tolist()])
    if result:
        result[0]['guide_lines'] = lines
    return result


@provider("lattice")
def _lattice(ins, out, p):
    if len(ins) < 3 or ins[2] is None:
        return []
    dims = ins[2].detail.get('lattice_dimensions')
    if dims is None:
        return []
    result = _lattice_grid([], ins[2], dict(zip(('columns', 'rows', 'layers'), dims)))
    for handle in result:
        handle['input_target'] = 'deformed'
    return result
