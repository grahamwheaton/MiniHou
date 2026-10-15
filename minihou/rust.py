"""Optional Rust compute core (``minihou_rs``, built from ``rust/``; see ``rust/README.md``).

Nothing in MiniHou depends on it: when the extension is not built, :func:`available` is False and every other function
raises :class:`RuntimeError`. The Python implementations stay the reference; ``tests/test_rust_parity.py`` compares the two.

The kernels take MiniHou arrays as they are (``vertex_point`` / ``prim_offsets`` int32, ``P`` float32, C-contiguous) and pass
them to Rust without copying; anything else is converted first.
"""
from __future__ import annotations

import contextlib
import json
import os
import threading
from typing import Optional

import numpy as np

_off_by_env = os.environ.get("MINIHOU_NO_RUST", "").strip().lower() in ("1", "true", "yes", "on")
try:
    if _off_by_env:
        raise ImportError("disabled by MINIHOU_NO_RUST")
    import minihou_rs as _rs
except ImportError:                                   # the normal case: the extension is optional
    _rs = None

_lock = threading.Lock()
_forced_off = 0                                       # > 0 inside `disabled()`


def available() -> bool:
    return _rs is not None


def enabled() -> bool:
    """The engine's fast paths use Rust: the extension is installed, MINIHOU_NO_RUST is not set, and no :func:`disabled` is active.
    The engine prefers Rust, then numba, then numpy; all three give the same results (tests/test_rust_parity.py)."""
    return _rs is not None and _forced_off == 0


@contextlib.contextmanager
def disabled():
    """Use the Python / numba paths inside this block (the parity tests use it to get the reference results)."""
    global _forced_off
    with _lock:
        _forced_off += 1
    try:
        yield
    finally:
        with _lock:
            _forced_off -= 1


# Node types whose Python function calls a Rust kernel. The evaluator records which backend each node used on its last cook
# (CookStats.backend) so the editor can show it: "rust" (a kernel ran), "fallback" (this node type has a Rust path, but this
# cook ran the Python / numba one: unsupported settings or no points) and "python" (no Rust path).
NODE_TYPES = frozenset({"grid", "box", "transform", "normal", "noise"})
_tls = threading.local()


def calls() -> int:
    """How many Rust kernels this thread has called so far (the evaluator reads it before and after each node)."""
    return getattr(_tls, "calls", 0)


def backend_of(node_type: str, calls_made: int) -> Optional[str]:
    """``"rust"``, ``"fallback"`` or ``"python"`` for a node that has just cooked; None when Rust is not in use."""
    if not enabled():
        return None
    if node_type in NODE_TYPES:
        return "rust" if calls_made > 0 else "fallback"
    return "python"


def status() -> str:
    if _rs is None:
        return "off (MINIHOU_NO_RUST)" if _off_by_env else "not installed"
    return f"minihou_rs {_rs.__version__}" + ("" if _forced_off == 0 else " (disabled)")


def version() -> Optional[str]:
    return getattr(_rs, "__version__", None)


def _need():
    if _rs is None:
        raise RuntimeError("the Rust core (minihou_rs) is not built: see rust/README.md")
    _tls.calls = getattr(_tls, "calls", 0) + 1
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


def point_prims(topology, npoints: int):
    """``(start, prims)``: the primitives around each point (CSR, in primitive order). It depends only on the topology, so it is
    cached on a frozen (cooked) topology: a deformer downstream shares its input's topology and reuses it."""
    t = topology
    hit = getattr(t, "_point_prims", None)
    if hit is not None and hit[0] == npoints:
        return hit[1]
    table = _need().point_prims(_i32(t.vertex_point), _i32(t.prim_offsets), int(npoints))
    if not t.vertex_point.flags.writeable:
        t._point_prims = (npoints, table)
    return table


def point_normals(geo) -> np.ndarray:
    """Unit point normals (float32) as the ``normal`` node makes them for ``domain='point'``."""
    t = geo.topology
    return _need().point_normals(_f32x3(geo.P), _i32(t.vertex_point), _i32(t.prim_offsets), point_prims(t, geo.npoints))


def fbm3(pos, seed: int, octaves: int, lacunarity: float, gain: float) -> np.ndarray:
    """:func:`minihou.nodes._util.fbm3_numpy` in Rust (float32)."""
    return _need().fbm3(_f32x3(pos), int(seed), int(octaves), float(lacunarity), float(gain))


def noise_vec(P, frequency, offset, amplitude, seed, octaves, lacunarity, gain, vector: bool, add: bool) -> np.ndarray:
    """The ``noise`` node's vector cases in one pass: ``P + noise`` (``add``) or the noise alone, float32 (n, 3)."""
    return _need().noise_vec(_f32x3(P), float(np.float32(frequency)), [float(x) for x in np.asarray(offset, np.float32)],
                             [float(x) for x in np.asarray(amplitude, np.float32)], int(seed) & 0xFFFFFFFF, int(octaves),
                             float(lacunarity), float(gain), bool(vector), bool(add))


def affine(P, M, pivot, shift) -> np.ndarray:
    """``(P - pivot) @ M.T + shift`` in float32 (the transform node), one parallel pass."""
    m = [[float(x) for x in row] for row in np.asarray(M, np.float32)]
    return _need().affine(_f32x3(P), m, [float(x) for x in np.asarray(pivot, np.float32)], [float(x) for x in np.asarray(shift, np.float32)])


def linear_normalized(V, M) -> np.ndarray:
    """``normalize_rows(V @ M.T)`` in float32 (normals through the transform node)."""
    m = [[float(x) for x in row] for row in np.asarray(M, np.float32)]
    return _need().linear_normalized(_f32x3(V), m)


def preview_layout(P, topology, max_tris: int, max_edges: int, chunk_min: int, chunk: int):
    """``(tri, tri_prim, edge, table, truncated)`` for :func:`minihou.preview._arrays_for`: fan triangles, Morton-sorted into runs
    with a bounds table (None below ``chunk_min`` triangles), and the edges; uint32 arrays, table float32 (rows, 8)."""
    t = topology
    closed = np.ascontiguousarray(t.closed_mask(), np.bool_) if t.has_open else None
    tri, tri_prim, edge, table, truncated = _need().preview_layout(_f32x3(P), _i32(t.vertex_point), _i32(t.prim_offsets), closed,
                                                                   int(max_tris), int(max_edges), int(chunk_min), int(chunk))
    return tri, tri_prim, edge, (None if table is None else table.reshape(-1, 8)), bool(truncated)


def draw_stats(topology, npoints: int):
    """:func:`minihou.proxy.draw_stats` computed in Rust (parallel): ``(fan triangles, open-curve vertices, loose points)``."""
    t = topology
    closed = np.ascontiguousarray(t.closed_mask(), np.bool_) if t.has_open else None
    return _need().draw_stats(_i32(t.vertex_point), _i32(t.prim_offsets), closed, int(npoints))


def bounds(P):
    """``(P.min(0), P.max(0))`` as float32 arrays of 3, in one parallel pass."""
    return _need().bounds(_f32x3(P))


def chunk_bounds(P, tri, chunk: int) -> np.ndarray:
    """:func:`minihou.preview.chunk_table` for triangles already in order: float32 (rows, 8)."""
    tri = np.ascontiguousarray(np.asarray(tri).reshape(-1), np.uint32)
    return _need().chunk_bounds(_f32x3(P), tri, int(chunk)).reshape(-1, 8)


def cook_node(node_type: str, **params):
    """One input-less node (``grid``, ``box``) cooked in Rust, as a Geometry."""
    try:
        return cook({"version": 1, "nodes": {"n": {"type": node_type, "params": params}}, "display": "n"})
    except RuntimeError as exc:
        # the core words errors as "node 'n' (type): ..." for the one-node graph above; the evaluator adds the real node
        # context, so the message is the same text the Python body raises
        message, prefix = str(exc), f"node 'n' ({node_type}): "
        raise ValueError(message[len(prefix):] if message.startswith(prefix) else message) from exc


def node_types() -> list:
    return list(_need().node_types())


def node_params(node_type: str) -> dict:
    """``{name: (default, supported)}`` as the Rust node declares them."""
    return {k: (json.loads(v), bool(s)) for k, (v, s) in _need().node_params(node_type).items()}


def _geometry(r):
    """A MiniHou Geometry from the dict ``minihou_rs.cook`` / ``Session.cook`` return."""
    from .geometry import Geometry, Topology
    g = Geometry(int(r["npoints"]), Topology(r["vertex_point"], r["prim_offsets"], r["closed"]))
    for domain in ("point", "vertex", "prim"):
        for name, a in r[domain].items():
            g.table(domain)[name] = a
    g.detail.update(json.loads(r["detail_json"]))
    return g


class Session:
    """An incremental evaluator in the Rust core: edit a parameter, cook again, and only the nodes whose recipe changed run
    (see ``rust/crates/minihou-core/src/session.rs``). ``cook`` gives the same result as :func:`cook` on the edited graph.

    ``last_stats`` is ``{"cooked": [...], "cached": [...], "node_ms": {...}}`` for the last cook. The returned Geometry owns
    copies of the arrays (the cache keeps its own).
    """

    def __init__(self, graph):
        if hasattr(graph, "to_dict"):
            graph = graph.to_dict()
        self._s = _need().Session(graph if isinstance(graph, str) else json.dumps(graph))

    def set_param(self, node: str, name: str, value) -> None:
        self._s.set_param(node, name, json.dumps(value))

    def set_bypass(self, node: str, flag: bool) -> None:
        self._s.set_bypass(node, bool(flag))

    def cook(self, target: Optional[str] = None):
        return _geometry(self._s.cook(target))

    @property
    def last_stats(self) -> dict:
        return self._s.stats()

    def cache_bytes(self) -> int:
        return int(self._s.cache_bytes())

    def set_budget_bytes(self, n: Optional[int]) -> None:
        self._s.set_budget_bytes(None if not n else int(n))

    def clear(self) -> None:
        self._s.clear()

    def set_keep_versions(self, n: int) -> None:
        """Results kept per node (default 2: the current one and the previous one)."""
        self._s.set_keep_versions(int(n))


def cook(graph, node: Optional[str] = None):
    """Cook a graph (a dict in graph JSON form, its JSON text, or a :class:`~minihou.graph.Graph`) with the Rust nodes; returns a Geometry.
    Raises RuntimeError with the cook error (a node type or parameter the Rust core does not have yet, say)."""
    if hasattr(graph, "to_dict"):
        graph = graph.to_dict()
    return _geometry(_need().cook(graph if isinstance(graph, str) else json.dumps(graph), node))
