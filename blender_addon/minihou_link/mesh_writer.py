"""Writes engine geometry into a Blender mesh with ``foreach_set``. Main thread only."""
from __future__ import annotations

import threading
import time
from typing import Dict, List, Optional, Tuple

import bpy
import numpy as np

from .protocol import GeoUpdate, Mirror, TOPO_KEYS, attr_key, split_key

_DOMAIN = {"point": "POINT", "vertex": "CORNER", "prim": "FACE"}
CUSTOM_ATTRS_KEY = "minihou_attrs"          # names of attributes we created, so we can remove stale ones

# Engine attribute names that become Blender-native data (Houdini name -> Blender meaning). Anything else is written as a
# generic attribute of the same name on POINT / CORNER / FACE.
UV_NAME, COLOR_NAME, NORMAL_NAME = "UVMap", "Color", "custom_normal"
NATIVE = {
    "uv": "UV map 'UVMap' (face corners; active for render)",
    "Cd": "color attribute 'Color' (active for render; alpha 1 unless Cd has 4 components)",
    "N": "custom normals (Blender's 'custom_normal')",
    "material_index": "per-face material index",
    "sharp_face": "flat shading per face",
}


GROUP_PREFIX = "group_"


def assert_main_thread() -> None:
    if threading.current_thread() is not threading.main_thread():
        raise RuntimeError("bpy must only be used on Blender's main thread")


def attribute_spec(arr: np.ndarray) -> Optional[Tuple[str, str, np.dtype]]:
    """(Blender data_type, RNA property for foreach_set, numpy dtype to write) or None if unsupported."""
    kind = arr.dtype.kind
    width = 1 if arr.ndim == 1 else arr.shape[1]
    if kind == "f":
        if width == 1:
            return "FLOAT", "value", np.dtype(np.float32)
        if width == 2:
            return "FLOAT2", "vector", np.dtype(np.float32)
        if width == 3:
            return "FLOAT_VECTOR", "vector", np.dtype(np.float32)
        if width == 4:
            return "FLOAT_COLOR", "color", np.dtype(np.float32)
    elif kind in "iu" and width == 1:
        return "INT", "value", np.dtype(np.int32)
    elif kind == "b" and width == 1:
        return "BOOLEAN", "value", np.dtype(np.bool_)
    return None


def set_positions(mesh, P: np.ndarray) -> str:
    """Write vertex positions. Returns which path was used.

    ``mesh.vertices.foreach_set("co", ...)`` walks the per-vertex RNA structs: ~165 ms for 1M points on
    the benchmark machine. The ``position`` attribute's ``foreach_set("vector", ...)`` is a straight
    memcpy: ~1 ms. The classic path is kept as a fallback in case a Blender version changes this.
    """
    flat = np.ascontiguousarray(P, dtype=np.float32).reshape(-1)
    try:
        mesh.attributes["position"].data.foreach_set("vector", flat)
        return "attribute"
    except (KeyError, RuntimeError, TypeError, AttributeError):
        mesh.vertices.foreach_set("co", flat)
        return "legacy"


def set_corner_verts(mesh, vertex_point: np.ndarray) -> str:
    """Write the vertex index of every face corner. The internal ``.corner_vert`` attribute is ~250x
    faster than ``loops.foreach_set("vertex_index")`` (1.3 ms vs 337 ms for 4M corners); fall back if absent."""
    vp = np.ascontiguousarray(vertex_point, dtype=np.int32)
    try:
        mesh.attributes[".corner_vert"].data.foreach_set("value", vp)
        return "attribute"
    except (KeyError, RuntimeError, TypeError, AttributeError):
        mesh.loops.foreach_set("vertex_index", vp)
        return "legacy"


def set_edge_verts(mesh, pairs: np.ndarray) -> None:
    """Write edge vertex pairs (the loose edges, added before calc_edges adds the face edges)."""
    flat = np.ascontiguousarray(pairs, dtype=np.int32).reshape(-1)
    try:
        mesh.attributes[".edge_verts"].data.foreach_set("value", flat)
    except (KeyError, RuntimeError, TypeError, AttributeError):
        mesh.edges.foreach_set("vertices", flat)


def _blender_name(engine_name: str) -> str:
    return {"uv": UV_NAME, "Cd": COLOR_NAME, "N": NORMAL_NAME}.get(engine_name, engine_name)


def write_attribute(mesh, domain: str, name: str, arr: np.ndarray) -> bool:
    """Create/replace one generic attribute and fill it. Returns False if it cannot be represented."""
    if name.startswith(GROUP_PREFIX) and arr.ndim == 1 and arr.dtype.kind in "iub":
        arr = arr != 0                                    # engine groups (group_<name> ints) become Blender booleans
    spec = attribute_spec(arr)
    if spec is None or name.startswith(".") or name == "position":
        return False
    data_type, prop, dtype = spec
    dom = _DOMAIN[domain]
    a = mesh.attributes.get(name)
    if a is not None and (a.domain != dom or a.data_type != data_type):
        mesh.attributes.remove(a)
        a = None
    if a is None:
        a = mesh.attributes.new(name, data_type, dom)
    flat = np.ascontiguousarray(arr, dtype=dtype).reshape(-1)
    if flat.size:
        a.data.foreach_set(prop, flat)
    return True


def _to_corners(domain: str, arr: np.ndarray, mirror: Mirror) -> Optional[np.ndarray]:
    """Values of any domain expressed per face corner (UV maps must be on corners; colors on points or corners)."""
    if domain == "vertex":
        return arr
    if domain == "point":
        return arr[mirror.vertex_point]
    if domain == "prim":
        sizes = np.diff(mirror.prim_offsets)
        return np.repeat(arr, sizes, axis=0)
    return None


def write_native(mesh, mirror: Mirror, domain: str, name: str, arr: np.ndarray) -> Optional[str]:
    """Write one engine attribute the way Blender understands it natively. Returns the Blender attribute name written,
    or None if ``name`` has no native meaning (or cannot be represented: then a warning is the caller's)."""
    width = 1 if arr.ndim == 1 else arr.shape[1]
    if name == "uv":
        if width < 2:
            return None
        data = _to_corners(domain, np.asarray(arr, np.float32).reshape(len(arr), -1)[:, :2], mirror)
        write_attribute(mesh, "vertex", UV_NAME, data)
        layer = mesh.uv_layers.get(UV_NAME)
        if layer is not None:
            mesh.uv_layers.active = layer
            layer.active_render = True
        return UV_NAME
    if name == "Cd":
        if width not in (3, 4):
            return None
        rgba = np.ones((len(arr), 4), np.float32)
        rgba[:, :width] = arr
        if domain == "prim":                                 # color attributes live on points or corners
            rgba, domain = _to_corners("prim", rgba, mirror), "vertex"
        write_attribute(mesh, domain, COLOR_NAME, rgba)
        ca = mesh.color_attributes.get(COLOR_NAME)
        if ca is not None:
            mesh.color_attributes.active_color = ca
            try:
                mesh.color_attributes.render_color_index = list(mesh.color_attributes).index(ca)
            except (AttributeError, ValueError, TypeError):
                pass
        return COLOR_NAME
    if name == "N":
        if width != 3:
            return None
        return NORMAL_NAME if set_custom_normals(mesh, domain, arr, mirror) else None
    if name == "material_index":
        if domain != "prim" or width != 1:
            return None
        write_attribute(mesh, "prim", "material_index", np.asarray(arr).astype(np.int32))
        return "material_index"
    if name == "sharp_face":
        if domain != "prim" or width != 1:
            return None
        write_attribute(mesh, "prim", "sharp_face", np.asarray(arr) != 0)
        return "sharp_face"
    return None


def set_custom_normals(mesh, domain: str, N: np.ndarray, mirror: Mirror) -> bool:
    """Custom normals from N. Blender 4.5+ reads a FLOAT_VECTOR 'custom_normal' attribute on POINT/CORNER/FACE ("free"
    normals: a memcpy, 7 ms for 1M points); older versions get the (much slower) normals_split_custom_set API."""
    N = np.ascontiguousarray(N, np.float32)
    a = mesh.attributes.get(NORMAL_NAME)
    if a is not None and a.data_type != "FLOAT_VECTOR":
        mesh.attributes.remove(a)
    try:
        write_attribute(mesh, domain, NORMAL_NAME, N)
        if getattr(mesh, "has_custom_normals", False):
            return True
    except (RuntimeError, TypeError, ValueError):
        pass
    a = mesh.attributes.get(NORMAL_NAME)                     # not honoured by this Blender: use the classic API
    if a is not None:
        mesh.attributes.remove(a)
    if domain == "point":
        mesh.normals_split_custom_set_from_vertices(N.tolist())
    else:
        mesh.normals_split_custom_set(_to_corners(domain, N, mirror).tolist())
    return True


class PolygonView:
    """A Mirror-like view with only the polygons (closed primitives) and their vertices, plus the open curves as
    loose edges. Blender faces need 3+ corners; curves become edges, and their primitive/vertex attributes have no
    Blender domain to live in (their point attributes do)."""

    def __init__(self, mirror: Mirror, closed: np.ndarray):
        offs = mirror.prim_offsets.astype(np.int64)
        sizes = np.diff(offs)
        prim_of_v = np.repeat(np.arange(len(sizes)), sizes)
        vmask = closed[prim_of_v]
        self.vertex_point = np.ascontiguousarray(mirror.vertex_point[vmask])
        self.prim_offsets = np.concatenate([[0], np.cumsum(sizes[closed])]).astype(np.int32)
        self.counts = {"point": mirror.counts["point"], "vertex": int(vmask.sum()), "prim": int(closed.sum())}
        self.arrays = {}
        for k, a in mirror.arrays.items():
            if k in TOPO_KEYS:
                continue
            dom = k.split("/", 1)[0]
            self.arrays[k] = a[vmask] if dom == "vertex" else a[closed] if dom == "prim" else a
        # loose edges: consecutive vertices of each open curve
        open_v = ~vmask
        nxt_same = np.r_[prim_of_v[1:] == prim_of_v[:-1], False]
        starts = np.flatnonzero(open_v & nxt_same)
        self.edges = np.stack([mirror.vertex_point[starts], mirror.vertex_point[starts + 1]], 1).astype(np.int32)

    def attributes(self):
        for k, a in self.arrays.items():
            dom, name = split_key(k)
            yield dom, name, a


def apply_update(mesh, mirror: Mirror, u: GeoUpdate) -> Dict[str, object]:
    """Apply an already-mirrored update to ``mesh``. Rebuilds topology only when it changed (or the
    counts no longer match the mesh); otherwise rewrites just the arrays that arrived.

    Returns timings in ms plus ``rebuilt`` and ``warnings``.
    """
    assert_main_thread()
    pc = time.perf_counter
    closed = mirror.prim_closed
    view = PolygonView(mirror, closed) if closed is not None else None
    loose = view.edges if view is not None else np.zeros((0, 2), np.int32)
    if view is not None:
        mirror = view                                    # faces only from here on; curves are the loose edges
    c = mirror.counts
    npts, nv, npr = c["point"], c["vertex"], c["prim"]
    rebuild = (u.full or u.topology_changed or len(mesh.vertices) != npts
               or len(mesh.loops) != nv or len(mesh.polygons) != npr)
    out: Dict[str, object] = {"rebuilt": rebuild, "warnings": [], "topology_ms": 0.0, "positions_ms": 0.0,
                              "attributes_ms": 0.0, "update_ms": 0.0}
    P_key = attr_key("point", "P")

    t = pc()
    if rebuild:
        mesh.clear_geometry()
        if npts:
            mesh.vertices.add(npts)
        if nv:
            mesh.loops.add(nv)
            out["corner_path"] = set_corner_verts(mesh, mirror.vertex_point)
        if npr:
            mesh.polygons.add(npr)
            mesh.polygons.foreach_set("loop_start", mirror.prim_offsets[:-1])
        if len(loose):
            mesh.edges.add(len(loose))
            set_edge_verts(mesh, loose)
        out["loose_edges"] = int(len(loose))
        written = set(mirror.arrays) - set(TOPO_KEYS)
    else:
        written = set(u.arrays) - set(TOPO_KEYS)
    out["topology_ms"] = (pc() - t) * 1000

    t = pc()
    if P_key in written and npts:
        out["position_path"] = set_positions(mesh, mirror.arrays[P_key])
    out["positions_ms"] = (pc() - t) * 1000

    t = pc()
    ours: List[str] = list(mesh.get(CUSTOM_ATTRS_KEY, []))
    reserved = {UV_NAME, COLOR_NAME, NORMAL_NAME}
    wanted = set()
    live_names = set()
    for key in sorted(k for k in mirror.arrays if k not in TOPO_KEYS and k != P_key):
        domain, name = split_key(key)
        live_names.add(_blender_name(name))
        if key not in written:
            continue
        arr = mirror.arrays[key]
        if name in NATIVE:
            done = write_native(mesh, mirror, domain, name, arr)
            if done:
                wanted.add(done)
            else:
                out["warnings"].append(f"attribute {key!r} {arr.shape} cannot be Blender's {NATIVE[name]}; skipped")
        elif name in reserved:
            out["warnings"].append(f"attribute {key!r}: the name {name!r} is used for a native Blender attribute "
                                   f"(see uv / Cd / N); rename it")
        elif write_attribute(mesh, domain, name, arr):
            wanted.add(name)
        else:
            out["warnings"].append(f"attribute {key!r} cannot be represented in Blender; skipped")
    for name in ours:                                    # attributes the engine no longer produces
        if name not in live_names and name in mesh.attributes:
            mesh.attributes.remove(mesh.attributes[name])
        elif name == NORMAL_NAME and name not in live_names and getattr(mesh, "has_custom_normals", False):
            mesh.normals_split_custom_set([(0.0, 0.0, 0.0)] * len(mesh.loops))   # old Blender: back to auto normals
    mesh[CUSTOM_ATTRS_KEY] = sorted((set(ours) & live_names) | wanted)
    out["attributes_ms"] = (pc() - t) * 1000

    t = pc()
    mesh.update(calc_edges=rebuild)
    out["update_ms"] = (pc() - t) * 1000
    return out
