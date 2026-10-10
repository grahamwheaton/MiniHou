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


def write_attribute(mesh, domain: str, name: str, arr: np.ndarray) -> bool:
    """Create/replace one generic attribute and fill it. Returns False if it cannot be represented."""
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


def apply_update(mesh, mirror: Mirror, u: GeoUpdate) -> Dict[str, object]:
    """Apply an already-mirrored update to ``mesh``. Rebuilds topology only when it changed (or the
    counts no longer match the mesh); otherwise rewrites just the arrays that arrived.

    Returns timings in ms plus ``rebuilt`` and ``warnings``.
    """
    assert_main_thread()
    pc = time.perf_counter
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
    wanted = set()
    for key in sorted(written - {P_key}):
        domain, name = split_key(key)
        if write_attribute(mesh, domain, name, mirror.arrays[key]):
            wanted.add(name)
        else:
            out["warnings"].append(f"attribute {key!r} cannot be represented in Blender; skipped")
    live_names = {split_key(k)[1] for k in mirror.arrays if k not in TOPO_KEYS}
    for name in ours:                                    # attributes the engine no longer produces
        if name not in live_names and name in mesh.attributes:
            mesh.attributes.remove(mesh.attributes[name])
    mesh[CUSTOM_ATTRS_KEY] = sorted((set(ours) & live_names) | wanted)
    out["attributes_ms"] = (pc() - t) * 1000

    t = pc()
    mesh.update(calc_edges=rebuild)
    out["update_ms"] = (pc() - t) * 1000
    return out
