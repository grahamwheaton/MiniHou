"""Writes engine geometry into a Blender mesh with ``foreach_set``. Main thread only."""
from __future__ import annotations

import threading
import time
from typing import Dict, List, Optional, Tuple

import bpy
import numpy as np

from . import material_slots
from .protocol import GeoUpdate, Mirror, TOPO_EDGE_PAIRS, TOPO_KEYS, attr_key, split_key

_DOMAIN = {"point": "POINT", "vertex": "CORNER", "edge": "EDGE", "prim": "FACE"}
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
    elif kind in "iu" and width == 2:
        return "INT32_2D", "value", np.dtype(np.int32)
    elif kind == "b" and width == 1:
        return "BOOLEAN", "value", np.dtype(np.bool_)
    return None


MADE_KEY = "minihou_made"                    # custom property on a material MiniHou created because the graph named it


def material_for(name: str):
    """The material called ``name``; one is made (and marked) when the file has none, so a name in a graph is never silently
    ignored. Returns ``(material, made)``."""
    mat = bpy.data.materials.get(name)
    if mat is not None:
        return mat, False
    mat = bpy.data.materials.new(name)
    mat[MADE_KEY] = 1
    return mat, True


def apply_material_names(mesh, names) -> List[str]:
    """Fill the mesh's material slots from the engine's name list (see :mod:`material_slots`). Existing slots are never removed.
    Returns warnings (a material that had to be created)."""
    assert_main_thread()
    names = material_slots.usable(names)
    if not names:
        return []
    current = [m.name if m is not None else None for m in mesh.materials]
    wanted = material_slots.plan(current, names)
    warnings: List[str] = []
    for i, name in enumerate(wanted):
        if i < len(current) and current[i] == name:
            continue
        mat = None
        if name:
            mat, made = material_for(name)
            if made:
                warnings.append(f"material {name!r} is not in this file; made an empty one")
        if i < len(current):
            mesh.materials[i] = mat
        else:
            mesh.materials.append(mat)
    return warnings


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


DETAIL_PREFIX = "mh_"                        # detail attribute "iteration" -> custom property "mh_iteration" on the output object
DETAIL_KEYS = "minihou_detail_keys"          # which custom properties we wrote, so stale ones can go
DETAIL_INTERNAL = {"instance_objects", "materials"}       # detail data the add-on consumes itself


def _detail_value(v):
    """A custom-property value for a detail attribute, or None if it cannot be one (numbers, text, short number lists)."""
    if isinstance(v, bool):
        return int(v)
    if isinstance(v, (int, float, str)):
        return v
    if isinstance(v, (list, tuple)) and 0 < len(v) <= 16 and all(isinstance(x, (int, float)) and not isinstance(x, bool) for x in v):
        return [int(x) for x in v] if all(isinstance(x, int) for x in v) else [float(x) for x in v]
    return None


def write_detail(obj, detail: Dict[str, object]) -> List[str]:
    """Publish the engine's detail attributes as custom properties ``mh_<name>`` on the output object, so drivers,
    other add-ons and scripts can read per-geometry values (counts, measurements, an iteration number). Only changed values
    are written. Returns warnings for values that could not be represented."""
    assert_main_thread()
    warnings: List[str] = []
    wanted = {}
    for name, v in detail.items():
        if name in DETAIL_INTERNAL:
            continue
        key = DETAIL_PREFIX + str(name)
        val = _detail_value(v)
        if val is None or len(key.encode("utf-8")) > 63:
            warnings.append(f"detail attribute {name!r} cannot be a Blender custom property; skipped")
            continue
        wanted[key] = val
    for key in list(obj.get(DETAIL_KEYS, [])):
        if key not in wanted and key in obj:
            del obj[key]
    for key, val in wanted.items():
        cur = obj.get(key)
        if hasattr(cur, "to_list"):
            cur = cur.to_list()
        if cur is None or type(cur) is not type(val) or cur != val:
            obj[key] = val
    keys = sorted(wanted)
    if list(obj.get(DETAIL_KEYS, [])) != keys:
        obj[DETAIL_KEYS] = keys
    return warnings


EXTRA_MARK = "minihou_extra"                 # custom property on objects the add-on created for extra outputs


def ensure_extra_object(name: str):
    """The mesh object for an extra output, created if missing. Returns None when ``name`` belongs to an object the
    add-on did not create (it never takes over someone's object)."""
    assert_main_thread()
    obj = bpy.data.objects.get(name)
    if obj is not None:
        return obj if obj.get(EXTRA_MARK) and obj.type == "MESH" else None
    mesh = bpy.data.meshes.new(name)
    obj = bpy.data.objects.new(name, mesh)
    obj[EXTRA_MARK] = True
    scene = bpy.context.scene
    (scene.collection if scene is not None else bpy.data.collections[0]).objects.link(obj)
    return obj


def remove_extra_objects(keep) -> List[str]:
    """Delete extra-output objects the add-on created that the engine no longer publishes. Returns their names."""
    assert_main_thread()
    keep = set(keep)
    gone = [o.name for o in bpy.data.objects if o.get(EXTRA_MARK) and o.name not in keep]
    for name in gone:
        obj = bpy.data.objects.get(name)
        mesh = obj.data if obj is not None and obj.type == "MESH" else None
        if obj is not None:
            bpy.data.objects.remove(obj, do_unlink=True)
        if mesh is not None and mesh.users == 0:
            bpy.data.meshes.remove(mesh)
    return gone


def _blender_name(engine_name: str, domain: str = "") -> str:
    if domain == "edge" and engine_name == "seam":
        return "uv_seam"
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


def edge_values(mesh, pairs: np.ndarray, values: np.ndarray) -> np.ndarray:
    """Reorder edge rows by their sorted point pair; missing mesh edges retain zero."""
    if pairs.shape != (len(values), 2):
        raise ValueError("edge values need one point pair per row")
    source = {tuple(sorted(map(int, pair))): i for i, pair in enumerate(pairs)}
    raw = np.empty(len(mesh.edges) * 2, np.int32)
    if len(mesh.edges):
        mesh.edges.foreach_get("vertices", raw)
    out = np.zeros((len(mesh.edges),) + values.shape[1:], values.dtype)
    for i, pair in enumerate(raw.reshape(-1, 2)):
        found = source.get(tuple(sorted(map(int, pair))))
        if found is not None:
            out[i] = values[found]
    return out


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
    if domain == "edge":
        if name not in ("sharp_edge", "seam") or width != 1:
            return None
        blender_name = _blender_name(name, domain)
        write_attribute(mesh, "edge", blender_name, np.asarray(arr) != 0)
        return blender_name
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
        if TOPO_EDGE_PAIRS in mirror.arrays:
            self.arrays[TOPO_EDGE_PAIRS] = mirror.arrays[TOPO_EDGE_PAIRS]
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
        if any(k.startswith("edge/") for k in mirror.arrays):
            mesh.update(calc_edges=True)             # populate polygon edges before matching pairs
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
        live_names.add(_blender_name(name, domain))
        if key not in written:
            continue
        arr = mirror.arrays[key]
        if domain == "edge":
            pairs = mirror.arrays.get(TOPO_EDGE_PAIRS)
            if pairs is None:
                out["warnings"].append(f"attribute {key!r} has no topo/edge_pairs; skipped")
                continue
            arr = edge_values(mesh, pairs, arr)
        if name in NATIVE or (domain == "edge" and name in ("sharp_edge", "seam")):
            done = write_native(mesh, mirror, domain, name, arr)
            if done:
                wanted.add(done)
            else:
                meaning = NATIVE.get(name, "edge flag")
                out["warnings"].append(f"attribute {key!r} {arr.shape} cannot be Blender's {meaning}; skipped")
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

    out["warnings"].extend(apply_material_names(mesh, (getattr(mirror, "detail", None) or u.detail).get("materials")))

    t = pc()
    mesh.update(calc_edges=rebuild)
    out["update_ms"] = (pc() - t) * 1000
    return out


def modal_ui_active():
    """Modal tools retain RNA pointers; do not rebuild modifier data underneath them."""
    wm = getattr(bpy.context, "window_manager", None)
    return bool(wm and any(tuple(getattr(window, "modal_operators", ())) for window in wm.windows))
