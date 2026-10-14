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
    """Whether a node type shows handles or guides: a built-in provider and/or ``Handle`` / ``Guide`` specs declared with
    ``@node(handles=..., guides=...)``."""
    return node_type in PROVIDERS or bool(getattr(nt, "handles", ())) or bool(getattr(nt, "guides", ()))


def handles_for(node_type: str, inputs: list, output, params: dict, nt=None) -> List[dict]:
    fn = PROVIDERS.get(node_type)
    specs = tuple(getattr(nt, "handles", ()) or ())
    gspecs = tuple(getattr(nt, "guides", ()) or ())
    if (fn is None and not specs and not gspecs) or output is None:
        return []
    params = {**params, "_inputs": len(inputs)}               # for `when="_inputs=2"`: how many inputs are connected
    if node_type == "group_create" and len(inputs) > 1 and inputs[1] is not None:       # the bounding object replaces the box parameters
        from .nodes.group import region_from
        params["box_center"], params["box_size"] = region_from(inputs[1], params.get("box_center"), params.get("box_size"))
    out: List[dict] = []
    makers = ([lambda: fn(inputs, output, params)] if fn else []) + ([lambda: declared(specs, output, params)] if specs else [])
    for make in makers:
        try:                                      # each source on its own: a failing one just shows nothing
            out += [_clean(h) for h in make()]
        except Exception:  # noqa: BLE001 - a gizmo must never break the preview
            continue
    if gspecs:
        try:
            out += declared_guides(gspecs, output, params)
        except Exception:  # noqa: BLE001 - nor a helper
            pass
    return out


# -- guides: outlines of the regions a node works on ---------------------------------------------------------------------
GUIDE_COLOURS = {"x": "#ff5d5d", "y": "#4cc38a", "z": "#4aa3ff"}
MAX_GUIDE_POINTS = 4000


def guide(id: str, label: str, lines, color: str = "") -> dict:
    """A guide item for the viewer: ``lines`` are polylines of world positions (each an (n, 3) array or list)."""
    return {"id": "guide:" + id, "label": label, "kind": "guide", "color": color, "lines": [np.asarray(l, np.float64).reshape(-1, 3) for l in lines],
            "param": "", "comp": None, "origin": np.zeros(3), "dir": AXES[0]}


def _basis(n):
    n = np.asarray(n, np.float64)
    n = n / max(float(np.linalg.norm(n)), 1e-12)
    a = AXES[0] if abs(n[0]) < 0.9 else AXES[1]
    u = np.cross(a, n)
    u /= max(float(np.linalg.norm(u)), 1e-12)
    return u, np.cross(n, u), n


def box_lines(center, size) -> list:
    c, h = np.asarray(center, np.float64), np.abs(np.asarray(size, np.float64)) / 2.0
    corners = np.array([[sx, sy, sz] for sz in (-1, 1) for sy in (-1, 1) for sx in (-1, 1)], np.float64) * h + c
    return [corners[[a, b]] for a, b in ((0, 1), (2, 3), (4, 5), (6, 7), (0, 2), (1, 3), (4, 6), (5, 7), (0, 4), (1, 5), (2, 6), (3, 7))]


def circle_lines(center, normal, radius, steps: int = 64) -> list:
    u, v, _ = _basis(normal)
    t = np.linspace(0.0, 2.0 * np.pi, steps + 1)
    return [np.asarray(center, np.float64) + (np.cos(t)[:, None] * u + np.sin(t)[:, None] * v) * float(radius)]


def sphere_lines(center, radii) -> list:
    """Three great circles of a sphere or ellipsoid (radii: a number or one per axis)."""
    c = np.asarray(center, np.float64)
    r = np.broadcast_to(np.asarray(radii, np.float64), (3,))
    t = np.linspace(0.0, 2.0 * np.pi, 65)
    co, si = np.cos(t), np.sin(t)
    z = np.zeros_like(t)
    return [c + np.stack([co * r[0], si * r[1], z], 1), c + np.stack([co * r[0], z, si * r[2]], 1), c + np.stack([z, co * r[1], si * r[2]], 1)]


def plane_lines(origin, normal, size) -> list:
    u, v, n = _basis(normal)
    o, h = np.asarray(origin, np.float64), float(size) / 2.0
    ring = np.array([o + (-u - v) * h, o + (u - v) * h, o + (u + v) * h, o + (-u + v) * h, o + (-u - v) * h])
    return [ring, np.array([o, o + n * h * 0.5])]


def _vector(v, params, default=None):
    if isinstance(v, str):
        if v.startswith("@"):
            return params.get(v[1:])
        return default
    return v


def declared_guides(specs, out, params) -> List[dict]:
    """The guide items for the ``Guide`` specs of a node type, resolved against the current parameters and the node's output."""
    from .sdk import HANDLE_AXES, holds
    c, lo, hi = _bounds(out)
    extent = max(float(np.linalg.norm(hi - lo) / 2.0), 0.05)
    res = []
    for g in specs:
        if g.when and not holds(g.when, params):
            continue
        a = g.at
        if isinstance(a, str):
            if a == "center":
                o = c
            elif a == "origin":
                o = np.zeros(3)
            elif a == "top":
                o = np.array([c[0], c[1], hi[2]])
            elif a == "bottom":
                o = np.array([c[0], c[1], lo[2]])
            else:
                o = np.asarray(params[a[1:]], np.float64).reshape(3)
        else:
            o = np.asarray(a, np.float64).reshape(3)
        ax = g.axis
        if isinstance(ax, str):
            d = np.asarray(params[ax[1:]], np.float64).reshape(3) if ax.startswith("@") else np.asarray(HANDLE_AXES[ax.lstrip("-")], np.float64) * (-1.0 if ax.startswith("-") else 1.0)
        else:
            d = np.asarray(ax, np.float64).reshape(3)
        if float(np.linalg.norm(d)) < 1e-12:
            d = AXES[2]
        size = _vector(g.size, params)
        if g.shape == "box":
            s = np.broadcast_to(np.asarray(size if size is not None else (hi - lo), np.float64), (3,))
            lines = box_lines(o, s)
        elif g.shape == "bounds":
            lines = box_lines(c, hi - lo)
        elif g.shape == "sphere":
            if size is None:
                r = extent
            else:
                sz = np.asarray(size, np.float64)
                r = sz if sz.ndim == 0 else np.abs(sz) / 2.0
            lines = sphere_lines(o, r)
        elif g.shape == "circle":
            lines = circle_lines(o, d, float(size) if size is not None else extent)
        elif g.shape == "plane":
            lines = plane_lines(o, d, float(size) if size is not None else extent * 2.0)
        elif g.shape == "line":
            n = d / np.linalg.norm(d)
            L = float(size) if size is not None else extent * 2.0
            lines = [np.array([o, o + n * L])]
        else:                                                   # axes
            L = float(size) if size is not None else extent * 0.5
            lines = [np.array([o, o + AXES[i] * L]) for i in range(3)]
        col = GUIDE_COLOURS.get(g.color, g.color)
        res.append(guide(g.key(), g.label or g.key().replace("_", " "), lines, col))
    return [_clean(h) for h in res]


def _bounds(geo):
    if not geo.npoints:
        return np.zeros(3), np.zeros(3), np.zeros(3)
    P = geo.P.astype(np.float64)
    lo, hi = P.min(0), P.max(0)
    return (lo + hi) / 2.0, lo, hi


def declared(specs, out, params) -> List[dict]:
    """Handles for the ``Handle`` specs of a node type (see :class:`minihou.sdk.Handle`), resolved against the current
    parameters and the node's output."""
    from .sdk import HANDLE_AXES, holds
    c, lo, hi = _bounds(out)
    extent = float(np.linalg.norm(hi - lo) / 2.0)

    def vec(v, sign=1.0):
        return sign * np.asarray(v, np.float64).reshape(3)

    res = []
    for h in specs:
        if h.when and not holds(h.when, params):
            continue
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
    if h.get("kind") == "guide":
        lines = [[[float(x) for x in p] for p in np.asarray(l, np.float64).reshape(-1, 3)[:MAX_GUIDE_POINTS]] for l in h["lines"][:64]]
        return {"id": str(h["id"]), "label": str(h.get("label", "")), "kind": "guide", "color": str(h.get("color", "")), "lines": lines,
                "param": "", "comp": None, "origin": [0.0, 0.0, 0.0], "dir": [1.0, 0.0, 0.0], "scale": 1.0}
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
    normal = _mean_normal(src, prims)
    corner = src.P.astype(float)[pts[0]]
    inward = origin - corner
    if np.linalg.norm(inward) < 1e-9:
        inward = AXES[0]
    inward /= np.linalg.norm(inward)
    return [{"id": "distance", "label": "distance", "param": "distance", "comp": None, "origin": origin,
             "dir": normal},
            {"id": "inset", "label": "Inset / Outset amount", "param": "inset", "comp": None,
             "origin": corner + normal * float(p.get("distance",0)), "dir": inward, "kind": "cage"}]


@provider("pushpull")
def _pushpull(ins, out, p):
    return _polyextrude(ins,out,p)[:1]


@provider("inset")
def _inset(ins, out, p):
    result = _polyextrude(ins,out,{**p,"distance":0})[1:]
    for h in result:
        h.update(id="amount",param="amount",label="Inset amount",min=0.)
    return result


@provider("outset")
def _outset(ins, out, p):
    result = _inset(ins,out,p)
    for h in result:
        h.update(label="Outset amount",dir=-h["dir"])
    return result


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


@provider("solidify")
def _solidify(ins, out, p):
    from . import groups
    src = ins[0] if ins else None
    if src is None or not src.nprims:
        return []
    selected = groups.mask(src, "prim", p.get("group", ""))
    prims = np.arange(src.nprims) if selected is None else np.flatnonzero(selected)
    if not len(prims):
        return []
    from .nodes.normal import prim_normals
    face = prims[0]
    offsets = src.topology.prim_offsets
    points = src.topology.vertex_point[offsets[face]:offsets[face+1]]
    origin = src.P[points].mean(axis=0).astype(float)
    normal = prim_normals(src)[face].astype(float)
    normal /= max(np.linalg.norm(normal), 1e-12)
    if np.linalg.norm(normal) < 1e-9:
        return []
    offset = float(p.get("offset", -1))
    coefficient = (offset + (1 if offset >= 0 else -1)) / 2
    thickness = float(p.get("thickness", .1))
    return [{"id": "thickness", "label": "Shell thickness", "param": "thickness", "comp": None,
             "origin": origin + normal*coefficient*thickness,
             "dir": normal*np.sign(coefficient), "scale": 1/abs(coefficient)}]


@provider("ray")
def _ray(ins, out, p):
    hit = out.point.get("hitprim")
    normals = out.point.get("hitN")
    if hit is None or normals is None:
        return []
    ids = np.flatnonzero(hit >= 0)
    if not len(ids):
        return []
    return [{"id": "lift", "label": "Surface lift", "param": "lift", "comp": None,
             "origin": out.P[ids[0]], "dir": normals[ids[0]]}]


@provider("sphere")
def _sphere(ins, out, p):
    c = np.asarray(p.get("center", (0, 0, 0)), np.float64)
    r = float(p.get("radius", 1.0))
    return [{"id": "radius", "label": "radius", "param": "radius", "comp": None, "origin": c + [r, 0, 0], "dir": AXES[0],
             "min": 0.0}] + _center_handles(c)


def _center_handles(center):
    return [{"id": f"center_{axis}", "label": f"Center {axis.upper()}", "param": "center", "comp": i,
             "origin": center, "dir": AXES[i], "tool": "move"} for i, axis in enumerate("xyz")]


@provider("box")
def _box(ins, out, p):
    c = np.asarray(p.get("center", (0, 0, 0)), np.float64)
    s = np.asarray(p.get("size", (1, 1, 1)), np.float64)
    return [{"id": f"size_{'xyz'[i]}", "label": f"size {'xyz'[i]}", "param": "size", "comp": i, "origin": c + AXES[i] * s[i] / 2,
             "dir": AXES[i], "scale": 2.0, "min": 0.0} for i in range(3)] + _center_handles(c)


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
                               "dir": sign * AXES[i], "scale": 1.0 if p.get("translate", True) else 2.0, "min": 0.0001,
                               "cage_sign": sign if p.get("translate", True) else 0, "kind": "cage"})
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

    result += [{"id": f"control_plane_{j}", "label": f"Control {j} view plane",
                "param": "offsets", "comp": None, "origin": point, "dir": AXES[2],
                "kind": "plane", "json_array": out.npoints * 3, "json_point": j}
               for j, point in enumerate(out.P)]

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
