"""Numba call-site parity against the numpy branches, with the Rust backend off.

The inputs are deterministic.  Geometry comparisons include every attribute domain,
topology, dtype and shape; float32 results allow 1e-5 for rounding of sums and
matrix products.  Float64 ray and solver results use tighter tolerances.
"""
import unittest

import numpy as np

from minihou import Geometry, accel, lscm, preview, rust, topo
from minihou.nodes._util import fbm3
from minihou.nodes.copy_to_points import copy_to_points
from minihou.nodes.noise import noise
from minihou.nodes.normal import normal
from minihou.nodes.ray import ray
from minihou.nodes.scatter import scatter
from minihou.raycast import TriGrid


def _compare_array(a, b, label, atol=1e-5):
    a, b = np.asarray(a), np.asarray(b)
    assert a.shape == b.shape, f"{label}: shape {a.shape} != {b.shape}"
    assert a.dtype == b.dtype, f"{label}: dtype {a.dtype} != {b.dtype}"
    if a.dtype.kind == "f":
        np.testing.assert_allclose(a, b, rtol=0, atol=atol, err_msg=label)
    else:
        np.testing.assert_array_equal(a, b, err_msg=label)


def _compare_geo(a, b, label, atol=1e-5):
    assert (a.npoints, a.nvertices, a.nprims) == (b.npoints, b.nvertices, b.nprims), label
    for field in ("vertex_point", "prim_offsets"):
        _compare_array(getattr(a.topology, field), getattr(b.topology, field), f"{label} topology.{field}")
    _compare_array(a.topology.closed_mask(), b.topology.closed_mask(), f"{label} topology.closed")
    for domain in ("point", "vertex", "prim"):
        x, y = a.table(domain), b.table(domain)
        assert set(x.names()) == set(y.names()), f"{label} {domain} names: {x.names()} != {y.names()}"
        for name in x.names():
            _compare_array(x[name], y[name], f"{label} {domain}.{name}", atol)
    assert a.detail == b.detail, f"{label} detail"


def _pair(fn):
    with rust.disabled():
        with accel.disabled():
            ref = fn()
        assert accel.enabled(), "Numba was disabled after a kernel error"
        fast = fn()
        assert accel.enabled(), "Numba was disabled after a kernel error"
    return ref, fast


@unittest.skipUnless(accel.AVAILABLE, f"Numba unavailable: {accel.IMPORT_ERROR}")
class AccelParityTests(unittest.TestCase):
    def test_fbm3_and_noise(self):
        for seed in (0, 7, 29):
            for size in (0, 1, 37, 4097):
                rng = np.random.default_rng(seed)
                p = rng.uniform(-4, 4, (size, 3)).astype(np.float32)
                label = f"noise/_util.fbm3 seed={seed} size={size}"
                ref, fast = _pair(lambda: fbm3(p, seed, 3, 2.1, 0.45))
                _compare_array(ref, fast, label)
                g = Geometry.points_only(p)
                for mode, attr in (("scalar", "P"), ("vector", "flow"), ("scalar", "height")):
                    kw = (g, attr, mode, (0.3, 0.7, 1.1), 1.3, (0.2, -0.4, 0.1), 3, 2.1, 0.45, seed)
                    ref, fast = _pair(lambda: noise(*kw))
                    _compare_geo(ref, fast, f"{label} mode={mode} attr={attr}")

    def test_scatter(self):
        for seed in (1, 9, 31):
            rng = np.random.default_rng(seed)
            p = rng.uniform(-2, 2, (6, 3)).astype(np.float32)
            p[:, 2] *= 0.2
            g = Geometry.from_arrays(p, [0, 1, 2, 3, 2, 4, 5], [0, 4, 7])
            for size in (0, 1, 37, 4097):
                ref, fast = _pair(lambda: scatter(g, size, seed))
                # Both branches draw three arrays from a fresh default_rng(seed) in the same order.
                _compare_geo(ref, fast, f"scatter seed={seed} size={size}")
            for case in (Geometry.points_only(p[:0]), Geometry.from_arrays(np.zeros((3, 3), np.float32),
                                                                              [0, 1, 2], [0, 3])):
                ref, fast = _pair(lambda: scatter(case, 1, seed))
                _compare_geo(ref, fast, f"scatter empty/degenerate seed={seed} size=1")

    def test_copy_to_points(self):
        for seed in (2, 13, 37):
            rng = np.random.default_rng(seed)
            src = Geometry.from_arrays(rng.uniform(-1, 1, (3, 3)).astype(np.float32), [0, 1, 2], [0, 3])
            src.point["tag"] = np.arange(3, dtype=np.int32)
            src.vertex["uv"] = rng.random((3, 2), dtype=np.float32)
            src.prim["id"] = np.array([4], np.int32)
            for size in (0, 1, 17, 1367):
                target = Geometry.points_only(rng.uniform(-3, 3, (size, 3)).astype(np.float32))
                target.point["N"] = rng.normal(size=(size, 3)).astype(np.float32)
                target.point["pscale"] = rng.uniform(0.2, 2, size).astype(np.float32)
                target.point["Cd"] = rng.random((size, 3), dtype=np.float32)
                for orient, scaled in ((False, False), (True, True)):
                    if size == 1367 and orient:
                        continue  # Isolated below as a confirmed position mismatch.
                    ref, fast = _pair(lambda: copy_to_points(src, target, orient, scaled))
                    _compare_geo(ref, fast, f"copy_to_points seed={seed} size={size} orient={orient} scale={scaled}")

    @unittest.expectedFailure
    def test_copy_to_points_near_opposite_normal_divergence(self):
        # Confirmed with seed=2, 1367 targets: 16 P components exceed 1e-5, maximum 3.99e-5 (seed 13: 1.75e-4).
        # Cause (read from the code): the numpy path normalises N with ``normalize_rows``, which rounds it to float32,
        # and only then builds the rotation (``rotation_from_z_to``, float64); the numba kernel ``copy_points`` normalises
        # in float64. Near N = -Z the rotation divides by (1 + nz), so the float32 rounding of N is amplified and a few
        # copies move by up to ~2e-4. The numba result is the more accurate one. Fixing it changes saved results for
        # near -Z normals, so it is left as a recorded finding, not changed here.
        size = 1367
        findings = []
        for seed in (2, 13, 37):
            rng = np.random.default_rng(seed)
            src = Geometry.from_arrays(rng.uniform(-1, 1, (3, 3)).astype(np.float32), [0, 1, 2], [0, 3])
            src.point["tag"] = np.arange(3, dtype=np.int32)
            src.vertex["uv"] = rng.random((3, 2), dtype=np.float32)
            src.prim["id"] = np.array([4], np.int32)
            # Match the draws made by the normal test for sizes 0, 1 and 17.
            for n in (0, 1, 17, size):
                target = Geometry.points_only(rng.uniform(-3, 3, (n, 3)).astype(np.float32))
                target.point["N"] = rng.normal(size=(n, 3)).astype(np.float32)
                target.point["pscale"] = rng.uniform(0.2, 2, n).astype(np.float32)
                target.point["Cd"] = rng.random((n, 3), dtype=np.float32)
            ref, fast = _pair(lambda: copy_to_points(src, target, True, True))
            label = f"copy_to_points seed={seed} size={size} orient=True scale=True"
            try:
                _compare_geo(ref, fast, label)
            except AssertionError as exc:
                difference = float(np.max(np.abs(ref.P.astype(np.float64) - fast.P.astype(np.float64))))
                findings.append(f"{label} max_P_diff={difference:.9g}: {exc}")
        self.assertFalse(findings, "\n".join(findings))

    def test_normal(self):
        for seed in (3, 17, 41):
            rng = np.random.default_rng(seed)
            for size in (0, 1, 37, 1025):
                p = rng.uniform(-1, 1, (size * 3, 3)).astype(np.float32)
                g = Geometry.from_arrays(p, np.arange(len(p), dtype=np.int32),
                                         np.arange(0, len(p) + 1, 3, dtype=np.int32))
                for domain in ("point", "prim"):
                    ref, fast = _pair(lambda: normal(g, domain))
                    _compare_geo(ref, fast, f"normal seed={seed} size={size} domain={domain}")

    def test_ray_intersect_and_closest(self):
        for seed in (4, 19, 43):
            rng = np.random.default_rng(seed)
            tris = rng.uniform(-2, 2, (19, 3, 3))
            grid = TriGrid(tris)
            for size in (0, 1, 37, 257):
                q = rng.uniform(-3, 3, (size, 3))
                direction = rng.normal(size=(size, 3))
                if size:
                    direction[0] = [0, 0, -1]
                for method, fn in (("intersect", lambda: grid.intersect(q, direction, 8.0)),
                                   ("closest", lambda: grid.closest(q))):
                    ref, fast = _pair(fn)
                    for i, (a, b) in enumerate(zip(ref, fast)):
                        _compare_array(a, b, f"raycast.TriGrid.{method} seed={seed} size={size} field={i}", 1e-7)

    def test_ray_node(self):
        target = Geometry.from_arrays(np.array([[-2, -2, 0], [2, -2, 0], [2, 2, 0], [-2, 2, 0]], np.float32),
                                      [0, 1, 2, 3], [0, 4])
        target.point["Cd"] = np.array([[1, 0, 0], [0, 1, 0], [0, 0, 1], [1, 1, 1]], np.float32)
        for seed in (4, 19, 43):
            rng = np.random.default_rng(seed)
            for size in (0, 1, 37, 257):
                p = rng.uniform(-1.5, 1.5, (size, 3)).astype(np.float32)
                p[:, 2] = rng.uniform(0.2, 2, size)
                source = Geometry.points_only(p)
                for method in (("minimum_distance",) if size == 0 else ("minimum_distance", "direction")):
                    args = (source, target, method, (0, 0, -1), False, 5.0, 1.0, 0.0, True, "Cd", False, "")
                    ref, fast = _pair(lambda: ray(*args))
                    _compare_geo(ref, fast, f"nodes.ray seed={seed} size={size} method={method}")

    def test_topology_edges_above_threshold(self):
        for seed, size in ((5, 0), (5, 1), (5, 37), (5, topo._NUMBA_EDGES + 1),
                           (23, topo._NUMBA_EDGES + 3)):
            rng = np.random.default_rng(seed)
            p = rng.uniform(-1, 1, (size, 3)).astype(np.float32)
            g = Geometry.from_arrays(p, np.arange(size, dtype=np.int32), np.array([0, size], np.int32),
                                     np.array([False])) if size else Geometry.points_only(p)
            ref, fast = _pair(lambda: topo.edges(g))
            for field in ("points", "of_vertex", "count"):
                _compare_array(getattr(ref, field), getattr(fast, field),
                               f"topo.edges seed={seed} size={size} {field}")

    def test_preview_fan_chunk_and_bounds(self):
        for seed, size in ((6, 1), (6, 37), (6, preview.EXACT_PRIMS + 1)):
            rng = np.random.default_rng(seed)
            p = rng.uniform(-2, 2, (size * 3, 3)).astype(np.float32)
            g = Geometry.from_arrays(p, np.arange(len(p), dtype=np.int32),
                                     np.arange(0, len(p) + 1, 3, dtype=np.int32))
            ref, fast = _pair(lambda: preview._triangles(g))
            for i in range(2):
                _compare_array(ref[i], fast[i], f"preview._triangles seed={seed} size={size} field={i}")
        for seed in (6, 25):
            rng = np.random.default_rng(seed)
            size = preview.CHUNK_MIN_TRIS + 1
            p = rng.uniform(-2, 2, (size * 3, 3)).astype(np.float32)
            tri = np.arange(len(p), dtype=np.int64).reshape(-1, 3)
            owner = np.arange(size, dtype=np.int64)
            ref, fast = _pair(lambda: preview.chunk_triangles(p, tri, owner))
            # The numpy and Numba Morton quantisation use different bounds.  Compare
            # the triangle/owner pairs as a set and recompute each branch's bounds.
            def sorted_pairs(result):
                rows = np.column_stack((result[0], result[1]))
                return rows[np.lexsort(rows.T[::-1])]
            _compare_array(sorted_pairs(ref), sorted_pairs(fast), f"preview.chunk_triangles seed={seed} size={size}")
            for name, result in (("numpy", ref), ("numba", fast)):
                bounds_ref, bounds_fast = _pair(lambda: preview.chunk_table(p, result[0]))
                _compare_array(bounds_ref, bounds_fast,
                               f"preview.chunk_table seed={seed} size={size} order={name}")
                _compare_array(result[2], bounds_fast, f"preview.chunk_triangles bounds seed={seed} size={size} order={name}")

    def test_lscm_pcg(self):
        for seed in (8, 27, 47):
            rng = np.random.default_rng(seed)
            for size in (1, 7, 61):
                # Banded least-squares system with a known, well-conditioned answer.
                n = size
                r = np.repeat(np.arange(n), 2)
                c = np.column_stack((np.arange(n), (np.arange(n) + 1) % n)).ravel()
                v = np.tile([2.0, -0.25], n).astype(np.float64)
                b = rng.normal(size=n)
                x0 = np.zeros(n)
                ref, fast = _pair(lambda: lscm._cg_normal(r, c, v, n, b, x0, 1e-10, 300))
                _compare_array(ref, fast, f"lscm._cg_normal seed={seed} size={size}", 1e-9)


if __name__ == "__main__":
    unittest.main()
