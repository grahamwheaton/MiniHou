"""Optional Rust compute core (``minihou_rs``, built from ``rust/``; see ``rust/README.md``).

Nothing in MiniHou depends on it: when the extension is not built, :func:`available` is False and every other function
raises :class:`RuntimeError`. The Python implementations stay the reference; ``tests/test_rust_parity.py`` compares the two.

The kernels take MiniHou arrays as they are (``vertex_point`` / ``prim_offsets`` int32, ``P`` float32, C-contiguous) and pass
them to Rust without copying; anything else is converted first.
"""
from __future__ import annotations

import json
from typing import Optional

import numpy as np

try:
    import minihou_rs as _rs
except ImportError:                                   # the normal case: the extension is optional
    _rs = None


def available() -> bool:
    return _rs is not None


def version() -> Optional[str]:
    return getattr(_rs, "__version__", None)


def _need():
    if _rs is None:
        raise RuntimeError("the Rust core (minihou_rs) is not built: see rust/README.md")
    return _rs


def _i32(a) -> np.ndarray:
    return np.ascontiguousarray(a, np.int32)


def _f32x3(a) -> np.ndarray:
    return np.ascontiguousarray(np.asarray(a, np.float32).reshape(-1, 3))


def edges(geo):
    """:func:`minihou.topo.edges` computed in Rust: the same :class:`~minihou.topo.Edges`, numbered identically."""
    from .topo import Edges
    t = geo.topology
    closed = np.ascontiguousarray(t.closed_mask(), np.bool_) if t.has_open else None
    pts, of_vertex, count = _need().edges(_i32(t.vertex_point), _i32(t.prim_offsets), closed, int(geo.npoints))
    return Edges(pts, of_vertex, count)


def prim_normals(geo) -> np.ndarray:
    """:func:`minihou.nodes.normal.prim_normals` in Rust (float64, length 2 x area)."""
    t = geo.topology
    return _need().prim_normals(_f32x3(geo.P), _i32(t.vertex_point), _i32(t.prim_offsets))


def point_normals(geo) -> np.ndarray:
    """Unit point normals (float32) as the ``normal`` node makes them for ``domain='point'``."""
    t = geo.topology
    return _need().point_normals(_f32x3(geo.P), _i32(t.vertex_point), _i32(t.prim_offsets))


def fbm3(pos, seed: int, octaves: int, lacunarity: float, gain: float) -> np.ndarray:
    """:func:`minihou.nodes._util.fbm3_numpy` in Rust (float32)."""
    return _need().fbm3(_f32x3(pos), int(seed), int(octaves), float(lacunarity), float(gain))


def node_types() -> list:
    return list(_need().node_types())


def node_params(node_type: str) -> dict:
    """``{name: (default, supported)}`` as the Rust node declares them."""
    return {k: (json.loads(v), bool(s)) for k, (v, s) in _need().node_params(node_type).items()}


def cook(graph, node: Optional[str] = None):
    """Cook a graph (a dict in graph JSON form, or a :class:`~minihou.graph.Graph`) with the Rust nodes; returns a Geometry.
    Raises RuntimeError with the cook error (a node type or parameter the Rust core does not have yet, say)."""
    from .geometry import Geometry, Topology
    if hasattr(graph, "to_dict"):
        graph = graph.to_dict()
    r = _need().cook(json.dumps(graph), node)
    closed = r["closed"]
    g = Geometry(int(r["npoints"]), Topology(r["vertex_point"], r["prim_offsets"], closed))
    for domain in ("point", "vertex", "prim"):
        for name, a in r[domain].items():
            g.table(domain)[name] = a
    g.detail.update(json.loads(r["detail_json"]))
    return g
