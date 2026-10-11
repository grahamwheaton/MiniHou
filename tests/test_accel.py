"""Numba backend: parity with the numpy reference, failure fallback, thread safety, switches."""
import os
import subprocess
import sys
import threading
import unittest
from pathlib import Path

import numpy as np

from minihou import Geometry, accel
from minihou.nodes import _util
from tests import test_nodes
from tests.helpers import quad_geo
from tests.test_nodes import run

ROOT = Path(__file__).resolve().parent.parent
needs_numba = unittest.skipUnless(accel.AVAILABLE, f"numba not available ({accel.IMPORT_ERROR})")


def rand_points(n, scale=20.0, seed=0):
    return ((np.random.default_rng(seed).random((n, 3), dtype=np.float32) - 0.5) * scale).astype(np.float32)


@needs_numba
class FbmParityTests(unittest.TestCase):
    def check(self, p, **kw):
        a = _util.fbm3_numpy(p, **kw)
        b = _util.fbm3(p, **kw)
        self.assertEqual(b.dtype, np.float32)
        np.testing.assert_allclose(a, b, atol=2e-6, rtol=0)
        return a, b

    def test_kernel_is_actually_used(self):
        called = []
        k = accel.kernels()
        orig = k.fbm3
        try:
            k.fbm3 = lambda *a: called.append(1) or orig(*a)
            _util.fbm3(rand_points(10), 0, 1, 2.0, 0.5)
        finally:
            k.fbm3 = orig
        self.assertTrue(called)

    def test_matches_numpy_across_settings(self):
        p = rand_points(50_000)
        for kw in (dict(seed=0, octaves=1, lacunarity=2.0, gain=0.5),
                   dict(seed=7, octaves=4, lacunarity=2.3, gain=0.45),
                   dict(seed=12345, octaves=8, lacunarity=1.5, gain=0.9)):
            self.check(p, **kw)

    def test_negative_and_huge_coordinates_and_seeds(self):
        self.check(rand_points(20_000, scale=2000.0), seed=3, octaves=2, lacunarity=2.0, gain=0.5)
        self.check(rand_points(20_000), seed=-17, octaves=2, lacunarity=2.0, gain=0.5)
        self.check(rand_points(20_000), seed=2 ** 40 + 5, octaves=1, lacunarity=2.0, gain=0.5)

    def test_lattice_points_are_zero_and_cell_edges_agree(self):
        lattice = np.array([[0, 0, 0], [1, 2, 3], [-4, 5, -6], [0.0, -0.0, 1e-8]], np.float32)
        np.testing.assert_allclose(_util.fbm3(lattice, 1, 1, 2.0, 0.5), 0, atol=1e-6)
        # points a hair either side of a cell boundary must land in the same cell as numpy picks
        edge = np.array([[0.99999994, 0.5, 0.5], [1.0, 0.5, 0.5], [1.0000001, 0.5, 0.5], [-1e-9, 0.2, 0.3]], np.float32)
        self.check(edge, seed=0, octaves=1, lacunarity=2.0, gain=0.5)

    def test_non_contiguous_and_float64_input(self):
        p = rand_points(5000)
        wide = np.zeros((5000, 6), np.float32)
        wide[:, :3] = p
        np.testing.assert_allclose(_util.fbm3(wide[:, :3], 0, 2, 2.0, 0.5), _util.fbm3_numpy(p, 0, 2, 2.0, 0.5), atol=2e-6)
        np.testing.assert_allclose(_util.fbm3(p.astype(np.float64), 0, 2, 2.0, 0.5), _util.fbm3_numpy(p, 0, 2, 2.0, 0.5), atol=2e-6)

    def test_empty_input(self):
        self.assertEqual(_util.fbm3(np.zeros((0, 3), np.float32), 0, 1, 2.0, 0.5).shape, (0,))

    def test_noise_node_end_to_end_matches_numpy_backend(self):
        g = run("grid", columns=60, rows=60)
        for kw in (dict(), dict(octaves=4, frequency=1.7, seed=9), dict(mode="vector", amplitude=(1, 2, 3))):
            fast = run("noise", g, **kw)
            with accel.disabled():
                ref = run("noise", g, **kw)
            np.testing.assert_allclose(fast.P, ref.P, atol=5e-6)


@needs_numba
class ScatterParityTests(unittest.TestCase):
    def both(self, geo, **kw):
        fast = run("scatter", geo, **kw)
        with accel.disabled():
            ref = run("scatter", geo, **kw)
        return fast, ref

    def test_same_prims_and_positions(self):
        g = run("noise", run("grid", columns=40, rows=40), amplitude=(0, 0, 2.0))
        fast, ref = self.both(g, count=20_000, seed=4)
        np.testing.assert_array_equal(fast.point["primnum"], ref.point["primnum"])
        np.testing.assert_allclose(fast.P, ref.P, atol=1e-5)
        np.testing.assert_allclose(fast.point["N"], ref.point["N"], atol=1e-5)

    def test_mixed_polygon_sizes_including_degenerate(self):
        P = np.array([[0, 0, 0], [2, 0, 0], [3, 1, 0], [1, 2, 0], [-1, 1, 0],        # pentagon
                      [5, 0, 0], [6, 0, 0], [5, 1, 0],                                 # triangle
                      [9, 9, 9], [9, 9, 9], [9, 9, 9],                                 # zero-area triangle
                      [20, 0, 0], [21, 0, 0]], np.float32)                              # 2-gon: no area
        vp = list(range(13))
        g = Geometry.from_arrays(P, vp, [0, 5, 8, 11, 13]).freeze()
        fast, ref = self.both(g, count=5000, seed=1)
        np.testing.assert_array_equal(fast.point["primnum"], ref.point["primnum"])
        np.testing.assert_allclose(fast.P, ref.P, atol=1e-5)
        self.assertEqual(set(np.unique(fast.point["primnum"])), {0, 1})

    def test_empty_cases_agree(self):
        flat = Geometry.from_arrays(np.zeros((3, 3), np.float32), [0, 1, 2], [0, 3]).freeze()
        fast, ref = self.both(flat, count=10)
        self.assertEqual((fast.npoints, ref.npoints), (0, 0))
        self.assertEqual(sorted(fast.point.names()), sorted(ref.point.names()))
        pts = Geometry.points_only(np.zeros((4, 3))).freeze()
        self.assertEqual(self.both(pts, count=10)[0].npoints, 0)

    def test_exact_cdf_boundary_draws_pick_the_same_triangle(self):
        """A random draw landing exactly on a cumulative-area boundary (measure-zero for random input, but
        r == 0.0 is a legal draw) must resolve identically: zero-area triangles are never selected."""
        from minihou.nodes import _kernels as k
        from minihou.nodes._util import fan_triangulate
        P = np.array([[9, 9, 9], [9, 9, 9], [9, 9, 9],                 # prim 0: zero-area triangle (first!)
                      [0, 0, 0], [1, 0, 0], [0, 1, 0],                 # prim 1: area 0.5
                      [5, 5, 5], [5, 5, 5], [5, 5, 5],                 # prim 2: zero-area
                      [2, 0, 0], [4, 0, 0], [2, 2, 0]], np.float32)    # prim 3: area 2
        g = Geometry.from_arrays(P, list(range(12)), [0, 3, 6, 9, 12]).freeze()
        offs = g.topology.prim_offsets
        counts = np.maximum(np.diff(offs.astype(np.int64)) - 2, 0)
        first = np.cumsum(counts) - counts
        area = np.empty(4, np.float64)
        tri_prim = np.empty(4, np.int32)
        k.tri_areas(g.P, g.topology.vertex_point, offs, counts, first, area, tri_prim)
        cdf = np.cumsum(area)
        total = cdf[-1]
        # draws hitting every boundary exactly, plus the extremes
        draws = np.array([0.0, cdf[0] / total, cdf[1] / total, cdf[2] / total, np.nextafter(1.0, 0.0), 0.25], np.float64)
        n = draws.size
        pos = np.empty((n, 3), np.float32)
        nrm = np.empty((n, 3), np.float32)
        prim = np.empty(n, np.int32)
        k.sample_surface(g.P, g.topology.vertex_point, offs, first, tri_prim, cdf, draws, np.zeros(n), np.zeros(n),
                         pos, nrm, prim)
        tris, tp = fan_triangulate(g.topology)
        expect = tp[np.minimum(np.searchsorted(cdf, draws * total, side="right"), 3)]
        np.testing.assert_array_equal(prim, expect)
        self.assertNotIn(0, prim)                                      # zero-area prims never chosen
        self.assertNotIn(2, prim)

    def test_determinism_across_runs_and_thread_counts(self):
        g = run("grid", columns=30, rows=30)
        a = run("scatter", g, count=3000, seed=2)
        b = run("scatter", g, count=3000, seed=2)
        np.testing.assert_array_equal(a.P, b.P)
        np.testing.assert_array_equal(a.point["primnum"], b.point["primnum"])


@needs_numba
class CopyParityTests(unittest.TestCase):
    def test_matches_numpy_with_orient_scale_and_attributes(self):
        g = run("noise", run("grid", columns=30, rows=30), amplitude=(0, 0, 1.5))
        pts = run("scatter", g, count=2000, seed=5)
        pts = pts.copy()
        pts.point["pscale"] = np.random.default_rng(1).uniform(0.2, 2.0, pts.npoints).astype(np.float32)
        pts.point["Cd"] = np.random.default_rng(2).random((pts.npoints, 3), dtype=np.float32)
        pts.freeze()
        src = run("transform", run("grid", columns=3, rows=3), rotate=(20, 30, 40), translate=(0, 0, 1))
        for kw in (dict(), dict(orient=False), dict(use_pscale=False), dict(orient=False, use_pscale=False)):
            fast = run("copy_to_points", src, pts, **kw)
            with accel.disabled():
                ref = run("copy_to_points", src, pts, **kw)
            np.testing.assert_allclose(fast.P, ref.P, atol=2e-5)
            np.testing.assert_array_equal(fast.topology.vertex_point, ref.topology.vertex_point)

    def test_special_normals(self):
        src = Geometry.points_only(np.array([[0.3, 0.2, 1.0]], np.float32)).freeze()
        N = np.array([[0, 0, 1], [0, 0, -1], [1, 0, 0], [0, -1, 0], [0, 0, 0], [3, 0, 0]], np.float32)
        t = Geometry.points_only(np.zeros((6, 3), np.float32))
        t.point["N"] = N
        t.freeze()
        fast = run("copy_to_points", src, t)
        with accel.disabled():
            ref = run("copy_to_points", src, t)
        np.testing.assert_allclose(fast.P, ref.P, atol=1e-6)


@needs_numba
class NormalParityTests(unittest.TestCase):
    def test_matches_numpy_point_and_prim(self):
        from tests.test_nodes_m2 import cube
        geos = [run("noise", run("grid", columns=40, rows=30), amplitude=(0, 0, 2), frequency=0.5), cube().freeze(),
                Geometry.from_arrays(np.array([[0, 0, 0], [2, 0, 0], [3, 1, 0], [1, 2, 0], [-1, 1, 0], [5, 5, 5], [5, 5, 5],
                                               [5, 5, 5], [9, 9, 9]], np.float32), [0, 1, 2, 3, 4, 5, 6, 7], [0, 5, 8]).freeze()]
        for g in geos:
            for domain, table in (("point", "point"), ("prim", "prim")):
                fast = run("normal", g, domain=domain).table(table)["N"]
                with accel.disabled():
                    ref = run("normal", g, domain=domain).table(table)["N"]
                np.testing.assert_allclose(fast, ref, atol=1e-6)


@needs_numba
class TriGridParityTests(unittest.TestCase):
    """The compiled ray and closest-point traversals agree with the numpy ones (and so with brute force)."""

    def layouts(self):
        rng = np.random.default_rng(3)
        th = rng.random((300, 3)) * np.pi * 2
        soup = np.stack([np.c_[np.cos(th[:, i]) * 2, np.sin(th[:, i]) * 2, rng.normal(0, 1, 300)] for i in range(3)], 1)
        plane = np.array([[[-9, -9, -3], [9, -9, -3], [9, 9, -3]], [[-9, -9, -3], [9, 9, -3], [-9, 9, -3]]], float)
        tiny = rng.random((150, 3, 3)) * 0.1 + rng.random((150, 1, 3)) * 6 - 3
        line = np.array([[[0, 0, z], [0.4, 0, z], [0, 0.4, z + 0.1]] for z in np.linspace(0, 5, 40)], float)
        return {"soup": soup, "plane": plane, "mixed": np.concatenate([soup, plane, tiny]), "stack": line,
                "single": soup[:1]}

    def test_intersect_matches_numpy(self):
        from minihou.raycast import TriGrid
        rng = np.random.default_rng(4)
        for name, tris in self.layouts().items():
            g = TriGrid(tris)
            o, d = rng.normal(0, 4, (400, 3)), rng.normal(0, 1, (400, 3))
            d[::7, 1:] = 0.0                                              # axis-parallel rays: zero direction parts
            for md in (np.inf, 3.0):
                with accel.disabled():
                    ref = g.intersect(o, d, md)
                got = g.intersect(o, d, md)
                self.assertTrue(hasattr(g, "_karr"), "the compiled kernel did not run")
                np.testing.assert_array_equal(np.isinf(got[0]), np.isinf(ref[0]), err_msg=name)
                hit = np.isfinite(ref[0])
                np.testing.assert_allclose(got[0][hit], ref[0][hit], atol=1e-9, err_msg=name)
                np.testing.assert_allclose(got[2][hit], ref[2][hit], atol=1e-7, err_msg=name)

    def test_closest_matches_numpy(self):
        from minihou.raycast import TriGrid, _closest_on_tri
        rng = np.random.default_rng(5)
        for name, tris in self.layouts().items():
            g = TriGrid(tris)
            q = rng.normal(0, 5, (300, 3))
            with accel.disabled():
                ref_p, ref_d, _ = g.closest(q)
            p, d, tri = g.closest(q)
            self.assertTrue(hasattr(g, "_karr"), "the compiled kernel did not run")
            np.testing.assert_allclose(d, ref_d, atol=1e-9, err_msg=name)
            np.testing.assert_allclose(p, ref_p, atol=1e-7, err_msg=name)      # equal distances may name another triangle
            np.testing.assert_allclose(_closest_on_tri(q, tris[tri]), p, atol=1e-9, err_msg=name)

    def test_inside_matches_numpy(self):
        from minihou.raycast import TriGrid, inside
        P = np.array([[0, 0, 0], [1, 0, 0], [1, 1, 0], [0, 1, 0], [0, 0, 1], [1, 0, 1], [1, 1, 1], [0, 1, 1]], float)
        F = [(0, 3, 2), (0, 2, 1), (4, 5, 6), (4, 6, 7), (0, 1, 5), (0, 5, 4), (2, 3, 7), (2, 7, 6), (1, 2, 6),
             (1, 6, 5), (3, 0, 4), (3, 4, 7)]
        g = TriGrid(P[np.array(F)])
        q = np.random.default_rng(6).uniform(-0.5, 1.5, (500, 3))
        with accel.disabled():
            ref = inside(g, q)
        np.testing.assert_array_equal(inside(g, q), ref)
        np.testing.assert_array_equal(ref, np.all((q > 0) & (q < 1), axis=1))

    def test_empty_input(self):
        from minihou.raycast import TriGrid
        g = TriGrid(self.layouts()["soup"])
        self.assertEqual(g.closest(np.zeros((0, 3)))[1].shape, (0,))
        self.assertEqual(g.intersect(np.zeros((0, 3)), np.zeros((0, 3)))[0].shape, (0,))

    def test_failing_kernels_fall_back_to_numpy(self):
        from minihou.raycast import TriGrid
        tris = self.layouts()["mixed"]
        q = np.random.default_rng(7).normal(0, 4, (50, 3))
        d = np.random.default_rng(8).normal(0, 1, (50, 3))
        k = accel.kernels()
        with accel.disabled():
            ref_c, ref_t = TriGrid(tris).closest(q), TriGrid(tris).intersect(q, d)
        for name, call, ref in (("grid_closest", lambda g: g.closest(q), ref_c),
                                ("grid_intersect", lambda g: g.intersect(q, d), ref_t)):
            orig = getattr(k, name)
            try:
                setattr(k, name, lambda *a: (_ for _ in ()).throw(RuntimeError("boom")))
                with self.assertLogs("minihou.accel", level="WARNING"):
                    out = call(TriGrid(tris))
                np.testing.assert_allclose(out[1] if name == "grid_closest" else out[0],
                                           ref[1] if name == "grid_closest" else ref[0], atol=1e-9)
            finally:
                setattr(k, name, orig)
                accel._disabled_reason = None


class FallbackAndSafetyTests(unittest.TestCase):
    def test_numpy_path_works_with_acceleration_disabled(self):
        with accel.disabled():
            self.assertFalse(accel.enabled())
            self.assertEqual(run("scatter", run("grid"), count=10).npoints, 10)
            self.assertEqual(run("noise", run("grid")).npoints, 100)
        if accel.AVAILABLE:
            self.assertTrue(accel.enabled())

    def _failing_kernel_case(self, kernel_name, node_kwargs):
        k = accel.kernels()
        orig = getattr(k, kernel_name)
        g = run("grid", columns=20, rows=20)
        was_enabled = accel.enabled()
        try:
            def boom(*a):
                raise RuntimeError("simulated JIT failure")
            setattr(k, kernel_name, boom)
            with self.assertLogs("minihou.accel", level="WARNING") as cm:
                out = run("noise", g, **node_kwargs)              # must still cook
            self.assertTrue(any("simulated JIT failure" in m for m in cm.output))
            self.assertFalse(accel.enabled())                      # off for the rest of the process
            self.assertIn("disabled after error", accel.status())
            with accel.disabled():
                ref = run("noise", g, **node_kwargs)
            np.testing.assert_array_equal(out.point[node_kwargs.get("attrib", "P")],
                                          ref.point[node_kwargs.get("attrib", "P")])   # identical to numpy
        finally:
            setattr(k, kernel_name, orig)
            accel._disabled_reason = None
        self.assertEqual(accel.enabled(), was_enabled)

    @needs_numba
    def test_a_failing_fused_kernel_falls_back_to_numpy_and_disables_acceleration(self):
        self._failing_kernel_case("noise_vec", {})

    @needs_numba
    def test_a_failing_fbm_kernel_falls_back_too(self):
        self._failing_kernel_case("fbm3", {"attrib": "mask"})       # scalar attribute path uses fbm3

    @needs_numba
    def test_scatter_and_copy_kernel_failures_fall_back(self):
        k = accel.kernels()
        g = run("grid", columns=15, rows=15)
        pts = run("scatter", g, count=50, seed=1)
        for name, call in (("tri_areas", lambda: run("scatter", g, count=50, seed=1)),
                           ("sample_surface", lambda: run("scatter", g, count=50, seed=1)),
                           ("copy_points", lambda: run("copy_to_points", g, pts))):
            orig = getattr(k, name)
            try:
                setattr(k, name, lambda *a: (_ for _ in ()).throw(RuntimeError("boom")))
                with self.assertLogs("minihou.accel", level="WARNING"):
                    out = call()
                with accel.disabled():
                    ref = call()
                np.testing.assert_allclose(out.P, ref.P, atol=1e-5)
            finally:
                setattr(k, name, orig)
                accel._disabled_reason = None

    @needs_numba
    def test_concurrent_cooks_from_many_threads_do_not_crash(self):
        """numba's default workqueue layer aborts the process on concurrent parallel launches."""
        g = run("grid", columns=200, rows=200)
        errors = []

        def work(i):
            try:
                for _ in range(4):
                    run("noise", g, seed=i, octaves=2)
                    run("scatter", g, count=20000, seed=i)
            except BaseException as e:  # noqa: BLE001
                errors.append(e)

        ts = [threading.Thread(target=work, args=(i,)) for i in range(6)]
        [t.start() for t in ts]
        [t.join(120) for t in ts]
        self.assertEqual(errors, [])
        self.assertFalse(any(t.is_alive() for t in ts))

    def test_env_switch_forces_numpy(self):
        out = subprocess.run([sys.executable, "-c", "from minihou import accel; print(accel.AVAILABLE, accel.enabled(), accel.status())"],
                             cwd=str(ROOT), env={**os.environ, "MINIHOU_NO_NUMBA": "1"}, capture_output=True, text=True, encoding="utf-8")
        self.assertEqual(out.returncode, 0, out.stderr)
        self.assertTrue(out.stdout.startswith("False False numpy only"), out.stdout)

    def test_broken_numba_import_leaves_engine_usable(self):
        """If importing numba blows up (version clash etc.) the engine must still import and cook."""
        code = ("import sys, types\n"
                "class Bad:\n"
                "    def find_spec(self, name, path, target=None):\n"
                "        if name == 'numba': raise RuntimeError('simulated numba/numpy mismatch')\n"
                "sys.meta_path.insert(0, Bad())\n"
                "import minihou\n"
                "from minihou import accel, Graph, Evaluator\n"
                "g = Graph.from_dict({'version':1,'nodes':{'g':{'type':'grid'},'n':{'type':'noise','inputs':{'geo':'g'}},"
                "'s':{'type':'scatter','inputs':{'geo':'n'}},'o':{'type':'output','inputs':{'geo':'s'}}}})\n"
                "print(accel.AVAILABLE, Evaluator().cook(g).npoints, accel.IMPORT_ERROR)\n")
        env = {k: v for k, v in os.environ.items() if k != "MINIHOU_NO_NUMBA"}      # the import must be attempted
        out = subprocess.run([sys.executable, "-c", code], cwd=str(ROOT), capture_output=True, text=True,
                             encoding="utf-8", env=env)
        self.assertEqual(out.returncode, 0, out.stderr)
        self.assertTrue(out.stdout.startswith("False 1000 "), out.stdout)
        self.assertIn("simulated numba/numpy mismatch", out.stdout)


# Re-run the node test classes against the numpy implementation even when numba is installed, so the
# fallback path never rots unnoticed.
class _NumpyBackend:
    def setUp(self):
        self._ctx = accel.disabled()
        self._ctx.__enter__()
        super().setUp()

    def tearDown(self):
        super().tearDown()
        self._ctx.__exit__(None, None, None)


@needs_numba
class NoiseNumpyBackend(_NumpyBackend, test_nodes.NoiseTests):
    pass


@needs_numba
class ScatterNumpyBackend(_NumpyBackend, test_nodes.ScatterTests):
    pass


@needs_numba
class CopyNumpyBackend(_NumpyBackend, test_nodes.CopyToPointsTests):
    pass


if __name__ == "__main__":
    unittest.main()
