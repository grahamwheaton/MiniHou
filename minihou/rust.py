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
NODE_TYPES = frozenset({"grid", "box", "point", "curve", "line", "circle", "sphere", "tube", "torus", "bound", "delete", "transform", "normal", "noise", "heightfield_erode",
                        "match_size", "remesh", "points_to_surface", "sdf_boolean", "sdf_shape"})
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


def _sdf_array(values, dtype):
    a = np.asarray(values, dtype=dtype)
    if a.ndim != 3:
        raise ValueError("SDF values must have three dimensions")
    return np.ascontiguousarray(a).reshape(-1), a.shape


def sdf_from_mesh(tris, h, band_voxels=3.0, pad_voxels=2, grid=None):
    from .sdf import Volume, grid_for
    tris = np.asarray(tris, np.float64)
    if tris.shape != (len(tris), 3, 3):
        raise ValueError("triangles must have shape (T, 3, 3)")
    band = band_voxels * h
    if grid is not None:
        lo, dims = np.asarray(grid[0], np.float64), np.asarray(grid[1], np.int64)
    elif len(tris):
        lo, dims = grid_for(tris.reshape(-1, 3).min(0), tris.reshape(-1, 3).max(0), h, band + pad_voxels * h)
    else:
        lo, dims = np.zeros(3), np.array([2, 2, 2], np.int64)
    flat_tris = np.ascontiguousarray(tris).reshape(-1)
    values, ambiguous = _need().sdf_from_mesh(flat_tris, lo.tolist(), tuple(int(x) for x in dims), float(h), float(band))
    if len(ambiguous):
        # SciPy's nearest-corner tie order is part of the reference sign convention.
        # Only wall voxels whose vertex seed survived the native triangle search need it.
        from .spatial import nearest
        ny, nz = dims[1:]
        idx = np.stack([ambiguous // (ny * nz), (ambiguous // nz) % ny, ambiguous % nz], 1)
        points = lo + idx * h
        seeds, distances = nearest(tris.reshape(-1, 3), points)
        values[ambiguous] = _need().sdf_resolve_seed_ties(flat_tris, np.ascontiguousarray(points).reshape(-1),
                                                       np.ascontiguousarray(seeds, np.int64),
                                                       np.ascontiguousarray(distances, np.float64), float(band))
    return Volume(lo, float(h), values.reshape(dims))


def sdf_flood(free):
    values, dims = _sdf_array(free, np.bool_)
    if 0 in dims:
        raise ValueError("cannot flood an empty grid axis")
    return _need().sdf_flood(values, dims).reshape(dims)


def sdf_from_points(P, radius, h, blend=1.0):
    from .sdf import Volume
    P = np.asarray(P, np.float64).reshape(-1, 3)
    radius = np.broadcast_to(np.asarray(radius, np.float64), (len(P),))
    lo, dims, values = _need().sdf_from_points(np.ascontiguousarray(P).reshape(-1),
                                             np.ascontiguousarray(radius), float(h), float(blend))
    return Volume(np.asarray(lo, np.float64), float(h), values.reshape(dims))


def sdf_blur(vol, iterations):
    from .sdf import Volume
    values, dims = _sdf_array(vol.values, np.float32)
    if iterations > 0 and 0 in dims:
        raise ValueError("can't extend empty axis using modes other than 'constant' or 'empty'")
    out = _need().sdf_blur(values, dims, max(0, int(iterations)))
    return Volume(vol.origin, vol.h, out.reshape(dims))


def sdf_resample_to(vol, origin, dims, h, fill):
    off = np.round((np.asarray(origin) - vol.origin) / h).astype(np.int64)
    values, src_dims = _sdf_array(vol.values, np.float32)
    dims = tuple(int(x) for x in dims)
    out = _need().sdf_resample_to(values, src_dims, off.tolist(), dims, float(fill))
    return out.reshape(dims)


def sdf_smooth_min(a, b, k):
    # Resolve dtype with NumPy's scalar-promotion rules using empty arrays; no field arithmetic here.
    base_dtype = dtype = np.result_type(a, b)
    if not k <= 0:
        dtype = (np.empty(0, dtype=dtype) / k).dtype
    if dtype not in (np.dtype("float32"), np.dtype("float64")):
        from . import sdf
        with disabled():
            return sdf.smooth_min(a, b, k)
    a, b = np.broadcast_arrays(np.asarray(a, dtype=dtype), np.asarray(b, dtype=dtype))
    if base_dtype == np.dtype("float32") and dtype == np.dtype("float64"):
        fn = _need().sdf_smooth_min_f32_k64
    else:
        fn = _need().sdf_smooth_min_f32 if dtype == np.dtype("float32") else _need().sdf_smooth_min_f64
    result = fn(np.ascontiguousarray(a).reshape(-1), np.ascontiguousarray(b).reshape(-1), float(k))
    return result.reshape(a.shape)


def sdf_resolve_face_saddles(v, max_rounds=16):
    dtype = np.asarray(v).dtype
    if dtype not in (np.dtype("float32"), np.dtype("float64")):
        from . import sdf
        with disabled():
            return sdf._resolve_face_saddles(v, max_rounds)
    values, dims = _sdf_array(v, dtype)
    fn = _need().sdf_resolve_face_saddles_f32 if dtype == np.dtype("float32") else _need().sdf_resolve_face_saddles
    return fn(values, dims, max(0, int(max_rounds))).reshape(dims)


def sdf_surface_nets(vol):
    values, dims = _sdf_array(vol.values, np.float64)
    return _need().sdf_surface_nets(values, dims, np.asarray(vol.origin, np.float64).tolist(), float(vol.h))


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


def match_size(P, target, translate, center, justify_z, scale, size, uniform, N=None):
    """The ``match_size`` node body in float64: ``(P (n, 3) float32, N (n, 3) float32 or None)``. ``target`` is the second
    input's nonempty points and ``N`` the point normals; N is rescaled only when the scale is not uniform."""
    return _need().match_size(_f32x3(P), None if target is None else _f32x3(target), bool(translate),
                              [float(x) for x in center], str(justify_z), bool(scale), [float(x) for x in size], bool(uniform),
                              None if N is None else _f32x3(N))


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

def _f64x2(a) -> np.ndarray:
    return np.ascontiguousarray(np.asarray(a, np.float64))


def _f64x1(a) -> np.ndarray:
    return np.ascontiguousarray(np.asarray(a, np.float64).reshape(-1))


def hydraulic_state(H: np.ndarray):
    """A state object in the Rust core for the hydraulic erosion loop."""
    return _need().Hydraulic(_f64x2(H))


def thermal(H: np.ndarray, cell, talus_deg: float, rate: float, iterations: int) -> np.ndarray:
    """:func:`minihou.nodes.heightfield.thermal` computed in the Rust core."""
    _NEIGHBOURS = [(-1, -1), (-1, 0), (-1, 1), (0, -1), (0, 1), (1, -1), (1, 0), (1, 1)]
    tan = float(np.tan(np.radians(talus_deg)))
    dist = np.ascontiguousarray([np.hypot(dy * cell[0], dx * cell[1]) for dy, dx in _NEIGHBOURS], np.float64)
    return _need().thermal(_f64x2(H), tan, dist, float(rate), int(iterations))


def hydraulic(H: np.ndarray, cell, iterations: int, rain: float, capacity: float, erode_rate: float,
              deposit_rate: float, evaporation: float, droplets: int = 4096, lifetime: int = 48,
              inertia: float = 0.1, gravity: float = 4.0, min_slope: float = 0.01, seed: int = 0):
    """:func:`minihou.nodes.heightfield.hydraulic` computed in the Rust core with draws from Python."""
    h, w = H.shape
    state = hydraulic_state(H)
    rng = np.random.default_rng(int(seed) & 0xFFFFFFFF)
    w_bound = float(w - 1.001)
    h_bound = float(h - 1.001)
    for _ in range(iterations):
        x = np.ascontiguousarray(rng.random(droplets) * w_bound, np.float64)
        y = np.ascontiguousarray(rng.random(droplets) * h_bound, np.float64)
        state.step(x, y, (float(cell[0]), float(cell[1])), float(rain), float(capacity),
                   float(erode_rate), float(deposit_rate), float(evaporation), int(lifetime),
                   float(inertia), float(gravity), float(min_slope))
    return state.finish()


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
            # Native Rust viewport hint; Python geometry has no equivalent attribute.
            if domain == "prim" and name == "__viewport_curve":
                continue
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
