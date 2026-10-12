"""Proxies for very large scenes: the cook level (parameters, cache keys, systems), auto refine, cancellation, the link budget,
and the geometry decimator."""
import tempfile
import threading
import time
import unittest
from pathlib import Path

import numpy as np

import minihou  # noqa: F401
from minihou import Evaluator, Geometry, Graph, proxy
from minihou.evaluator import CookCancelled
from minihou.link.session import Session
from minihou.sdk import REGISTRY, Float, Int, ParamError, Registry, node
from tests.helpers import graph_from
from tests.test_nodes import run

BIG = {"g": {"type": "grid", "params": {"columns": 400, "rows": 200}},
       "s": {"type": "scatter", "params": {"count": 100000}, "inputs": {"geo": "g"}},
       "o": {"type": "output", "inputs": {"geo": "g"}}}


class LevelScalingTests(unittest.TestCase):
    def scaled(self, name, level, **given):
        nt = REGISTRY.get(name)
        return proxy.scale_params(nt, nt.coerce_params(given), level)

    def test_kinds_scale_with_the_work(self):
        self.assertEqual(self.scaled("scatter", 0.1, count=100000)["count"], 10000)                 # count: x level
        g = self.scaled("grid", 0.25, columns=1000, rows=400)
        self.assertEqual((g["columns"], g["rows"]), (500, 200))                                     # res2: x sqrt(level)
        self.assertEqual(self.scaled("box", 0.25, divisions=40)["divisions"], 20)                  # res2 again
        self.assertAlmostEqual(self.scaled("remesh", 0.125)["voxel_size"], 0.05 * 2.0, places=6)    # step3: size / cbrt(level)
        self.assertAlmostEqual(self.scaled("resample", 0.25, length=0.1)["length"], 0.4, places=6)  # step1: size / level

    def test_full_level_and_unmarked_parameters_are_untouched(self):
        nt = REGISTRY.get("grid")
        p = nt.coerce_params({"columns": 700})
        self.assertIs(proxy.scale_params(nt, p, 1.0), p)
        g = self.scaled("grid", 0.01, columns=700, size_x=33.0)
        self.assertEqual(g["size_x"], 33.0)                                                         # a size is not a resolution
        self.assertEqual(g["columns"], 70)

    def test_a_proxy_never_goes_below_the_minimum_or_above_the_real_value(self):
        self.assertEqual(self.scaled("grid", 0.001, columns=50, rows=3)["columns"], 2)              # min=2
        self.assertEqual(self.scaled("grid", 0.001, columns=50, rows=3)["rows"], 2)
        self.assertEqual(self.scaled("scatter", 0.001, count=100)["count"], 1)                      # at least one while there is some
        self.assertEqual(self.scaled("scatter", 0.5, count=0)["count"], 0)                          # none stays none
        self.assertEqual(self.scaled("sphere", 0.5, rows=2)["rows"], 2)

    def test_level_is_clamped_and_validated(self):
        self.assertEqual(proxy.clamp(7), 1.0)
        self.assertEqual(proxy.clamp(0), proxy.MIN_LEVEL)
        with self.assertRaises(ValueError):
            proxy.clamp(float("nan"))

    def test_only_numeric_parameters_can_carry_a_proxy_kind(self):
        from minihou.sdk import Bool, Param
        with self.assertRaises(ValueError):
            Param("x", "bool", False, proxy="count")
        with self.assertRaises(ValueError):
            Int("x", 1, proxy="nonsense")

    def test_schema_tells_editors_which_parameters_scale(self):
        cols = [p for p in REGISTRY.get("grid").describe()["params"] if p["name"] == "columns"][0]
        self.assertEqual(cols["proxy"], "res2")
        seed = [p for p in REGISTRY.get("scatter").describe()["params"] if p["name"] == "seed"][0]
        self.assertNotIn("proxy", seed)


class CookLevelTests(unittest.TestCase):
    def test_cook_at_a_level_makes_a_smaller_result_and_never_mixes_cache_entries(self):
        g = Graph.from_dict({"version": 1, "nodes": BIG, "display": "s"})
        ev = Evaluator()
        full = ev.cook(g)
        self.assertEqual(full.npoints, 100000)
        ev.proxy_level = 0.1
        small = ev.cook(g)
        self.assertEqual(small.npoints, 10000)
        ev.proxy_level = None
        self.assertEqual(ev.cook(g).npoints, 100000)
        self.assertEqual(ev.last_stats.cooked, [])                  # the full one was still cached
        with proxy.using(0.1):
            self.assertEqual(ev.cook(g).npoints, 10000)
        self.assertEqual(ev.last_stats.cooked, [])                  # and so was the small one

    def test_nodes_that_do_not_use_the_level_share_one_key(self):
        g = Graph.from_dict({"version": 1, "nodes": {"b": {"type": "box"}, "t": {"type": "transform", "inputs": {"geo": "b"}}}, "display": "t"})
        ev = Evaluator()
        order = g.upstream_order("t")
        full = ev.compute_keys(g, order)
        with proxy.using(0.1):
            small = ev.compute_keys(g, order)
        self.assertEqual(full, small)                               # nothing in this graph scales: one cached result serves both

    def test_a_proxy_parameter_changes_every_key_downstream(self):
        g = Graph.from_dict({"version": 1, "nodes": BIG, "display": "o"})
        ev = Evaluator()
        order = g.upstream_order("o")
        full = ev.compute_keys(g, order)
        with proxy.using(0.25):
            small = ev.compute_keys(g, order)
        self.assertNotEqual(full["g"], small["g"])
        self.assertNotEqual(full["o"], small["o"])

    def test_default_keys_are_unchanged_at_full_level(self):
        g = Graph.from_dict({"version": 1, "nodes": {"b": {"type": "box"}}, "display": "b"})
        ev = Evaluator()
        k1 = ev.compute_keys(g, ["b"])
        with proxy.using(1.0):
            self.assertEqual(ev.compute_keys(g, ["b"]), k1)

    def _system(self, name, exposed=()):
        import json
        import os
        import shutil
        from minihou.library import NodeLibrary
        d = Path(tempfile.mkdtemp(prefix="minihou_proxy_"))
        self.addCleanup(shutil.rmtree, d, True)
        graph = {"version": 1, "display": "out", "exposed": list(exposed),
                 "nodes": {"preview": {"type": "grid", "params": {"columns": 2, "rows": 2}},
                           "inp": {"type": "input", "inputs": {"preview": "preview"}, "params": {"name": "src"}},
                           "s": {"type": "scatter", "params": {"count": 5000}, "inputs": {"geo": "inp"}},
                           "out": {"type": "output", "inputs": {"geo": "s"}}}}
        name = f"{name}_{os.getpid()}"
        (d / f"{name}.json").write_text(json.dumps(graph), encoding="utf-8")
        lib = NodeLibrary([d])
        self.addCleanup(lib.unload)
        lib.refresh()
        return name

    def test_systems_get_the_level_in_their_key_and_scale_inside(self):
        name = self._system("dots")
        g = Graph.from_dict({"nodes": {"g": {"type": "grid"}, "d": {"type": name, "inputs": {"src": "g"}}}, "display": "d"}, check=False)
        ev = Evaluator()
        self.assertEqual(ev.cook(g).npoints, 5000)
        ev.proxy_level = 0.1
        self.assertEqual(ev.cook(g).npoints, 500)               # the scatter inside the system scaled, and the outer cache did not answer
        ev.proxy_level = None
        self.assertEqual(ev.cook(g).npoints, 5000)

    def test_exposed_system_parameters_are_not_scaled_twice(self):
        name = self._system("dots2", exposed=[{"name": "count", "node": "s", "param": "count"}])
        self.assertEqual(REGISTRY.get(name).proxy_params, ())
        g = Graph.from_dict({"nodes": {"g": {"type": "grid"}, "d": {"type": name, "inputs": {"src": "g"}, "params": {"count": 5000}}},
                             "display": "d"}, check=False)
        ev = Evaluator()
        ev.proxy_level = 0.1
        self.assertEqual(ev.cook(g).npoints, 500)               # 5000 x 0.1 once, not 5000 x 0.1 x 0.1

    def test_cancel_stops_between_nodes(self):
        g = Graph.from_dict({"version": 1, "nodes": BIG, "display": "s"})
        ev = Evaluator()
        calls = []
        ev.cancel = lambda: calls.append(1) or len(calls) > 1
        with self.assertRaises(CookCancelled):
            ev.cook(g)
        ev.cancel = None
        self.assertEqual(ev.cook(g).npoints, 100000)                # the evaluator is fine afterwards


def _slow_registry(delay, scaled=True):
    """Two nodes: ``src`` (n points; ``n`` scales with the proxy level unless ``scaled`` is False) and a pass-through."""
    reg = Registry()

    @node("src", params=[Int("n", 10, proxy="count" if scaled else "")], registry=reg)
    def src(n):
        time.sleep(delay if n >= 100 else 0.0)
        return Geometry.from_arrays(np.zeros((n, 3), np.float32))

    @node("out", inputs=["geo"], registry=reg)
    def out(geo):
        return geo
    return reg


class SessionProxyTests(unittest.TestCase):
    def make(self, n=200, delay=0.0):
        reg = _slow_registry(delay)
        g = Graph.from_dict({"version": 1, "nodes": {"a": {"type": "src", "params": {"n": n}},
                                                     "o": {"type": "out", "inputs": {"geo": "a"}}}, "display": "o"}, reg)
        s = Session(g, registry=reg)
        s.start()
        self.addCleanup(s.stop)
        return s

    def test_fixed_level_cooks_the_smaller_graph_and_says_so(self):
        s = self.make(n=1000)
        self.assertTrue(s.wait_idle(5))
        self.assertEqual(s.latest_good.geo.npoints, 1000)
        s.set_proxy(level=0.1)
        self.assertTrue(s.wait_idle(5))
        self.assertEqual(s.latest_good.geo.npoints, 100)
        self.assertEqual(s.latest_good.proxy, 0.1)
        st = s.status()
        self.assertEqual((st["proxy"]["level"], st["proxy"]["cooked"], st["points"]), (0.1, 0.1, 100))
        self.assertEqual(st["cook"]["proxy"], 0.1)
        s.set_proxy(level=1.0)
        self.assertTrue(s.wait_idle(5))
        self.assertEqual((s.latest_good.geo.npoints, s.latest_good.proxy), (1000, 1.0))

    def test_inspect_and_preview_follow_the_level(self):
        s = self.make(n=1000)
        s.set_proxy(level=0.1)
        self.assertTrue(s.wait_idle(5))
        self.assertEqual(s.inspect("o")["points"], 100)
        self.assertEqual(s.preview("o")["geo"].npoints, 100)

    def test_auto_mode_publishes_the_proxy_then_the_full_result(self):
        s = self.make(n=1000, delay=0.0)
        self.assertTrue(s.wait_idle(5))
        seen = []

        class Listener:
            def on_result(self, res):
                seen.append((res.proxy, res.geo.npoints))
        s.add_listener(Listener())
        s.set_proxy(level=0.1, auto=True)               # no full cook has been slow yet: _full_ms is small, so it cooks full only
        self.assertTrue(s.wait_idle(5))
        # force the slow path: pretend the full cook was slow
        s._full_ms = 5000.0
        s.set_proxy(level=0.2)
        s.set_proxy(level=0.1)
        self.assertTrue(s.wait_idle(10))
        seen.clear()
        s.set_proxy(level=1.0, auto=True)
        s._full_ms = 5000.0
        s._last_key = None
        s._rev += 1
        s._wake.set()
        self.assertTrue(s.wait_idle(10))
        self.assertEqual(seen[0], (proxy.AUTO_LEVEL, 100))                  # the quick one first
        self.assertEqual(seen[-1], (1.0, 1000))                             # then the real one
        self.assertFalse(s.status()["proxy"]["refining"])
        self.assertEqual(s.latest_good.proxy, 1.0)

    def test_auto_mode_skips_the_proxy_when_nothing_depends_on_the_level(self):
        reg = _slow_registry(0.0, scaled=False)
        g = Graph.from_dict({"version": 1, "nodes": {"a": {"type": "src", "params": {"n": 5}},
                                                     "o": {"type": "out", "inputs": {"geo": "a"}}}, "display": "o"}, reg)
        s = Session(g, registry=reg)
        s.start()
        self.addCleanup(s.stop)
        self.assertTrue(s.wait_idle(5))
        s._full_ms = 5000.0
        seen = []

        class Listener:
            def on_result(self, res):
                seen.append(res.proxy)
        s.add_listener(Listener())
        s.set_proxy(level=0.1, auto=True)
        self.assertTrue(s.wait_idle(10))
        self.assertTrue(all(p == 1.0 for p in seen), seen)                  # the graph has no level-dependent node: no wasted passes

    def test_a_new_edit_during_the_pause_skips_the_refine(self):
        s = self.make(n=1000)
        self.assertTrue(s.wait_idle(5))
        s.set_proxy(level=0.1, auto=True)
        s._full_ms = 5000.0
        s.edit([{"op": "set", "id": "a", "params": {"n": 2000}}])
        s.edit([{"op": "set", "id": "a", "params": {"n": 3000}}])
        self.assertTrue(s.wait_idle(10))
        self.assertEqual(s.latest_good.geo.npoints, 3000)                   # the last edit won, at full resolution
        self.assertEqual(s.latest_good.proxy, 1.0)

    def test_link_budget_reduces_what_blender_receives(self):
        reg = Registry()
        s = Session(Graph.from_dict({"version": 1, "nodes": {"g": {"type": "grid", "params": {"columns": 100, "rows": 100}}}, "display": "g"}))
        s.start()
        self.addCleanup(s.stop)
        self.assertTrue(s.wait_idle(5))
        good = s.latest_good
        self.assertIs(s.link_geometry(good), good.geo)                      # off by default
        s.set_proxy(link_tris=2000)
        self.assertTrue(s.wait_idle(5))
        small = s.link_geometry(s.latest_good)
        self.assertLessEqual(small.nprims, 2000)
        self.assertGreater(small.nprims, 500)
        self.assertIs(s.link_geometry(s.latest_good), small)                # remembered per result
        self.assertEqual(s.latest_good.geo.nprims, 99 * 99)                 # the cooked geometry itself is untouched

    def test_settings_validate(self):
        s = self.make()
        self.assertEqual(s.set_proxy(level=5)["level"], 1.0)
        self.assertEqual(s.set_proxy(level=0)["level"], proxy.MIN_LEVEL)
        with self.assertRaises(ValueError):
            s.set_proxy(level=float("inf"))


class DecimateTests(unittest.TestCase):
    def test_within_budget_returns_the_same_geometry(self):
        g = run("sphere", rows=8, columns=10)
        r = proxy.decimate(g, 10_000, 10_000)
        self.assertFalse(r.reduced)
        self.assertIs(r.geo, g)

    def test_meshes_keep_their_shape_and_land_under_the_budget(self):
        for name, p in (("sphere", dict(rows=120, columns=240)), ("torus", dict(rows=80, columns=160)), ("box", dict(divisions=40))):
            g = run(name, **p)
            for budget in (3000, 600, 120):
                r = proxy.decimate(g, budget, 10_000)
                r.geo.validate()
                self.assertTrue(r.reduced, (name, budget))
                self.assertLessEqual(r.geo.nprims, budget, (name, budget))
                self.assertGreater(r.geo.nprims, budget * 0.25, (name, budget))
                # clusters are averages of surface points, so the result stays inside the original bounds
                lo, hi = g.P.min(0), g.P.max(0)
                self.assertTrue((r.geo.P.min(0) >= lo - 1e-4).all() and (r.geo.P.max(0) <= hi + 1e-4).all())
                span = (hi - lo).max()
                self.assertGreater((r.geo.P.max(0) - r.geo.P.min(0)).max(), span * (0.6 if budget >= 600 else 0.3))

    def test_attributes_follow_their_points_prims_and_corners(self):
        g = run("grid", columns=60, rows=60).copy()
        g.point["Cd"] = np.stack([g.point["uv"][:, 0], g.point["uv"][:, 1], np.zeros(g.npoints, np.float32)], 1)
        g.point["id"] = np.arange(g.npoints, dtype=np.int32)
        g.prim["tag"] = np.arange(g.nprims, dtype=np.int32)
        g.vertex["w"] = np.arange(g.nvertices, dtype=np.float32)
        g.freeze()
        r = proxy.decimate(g, 400, 1000)
        o = r.geo
        self.assertEqual(len(r.point_src), o.npoints)
        self.assertEqual(len(r.prim_src), o.nprims)
        # floats are cluster averages (a smooth gradient stays inside 0..1), ints are real ids of a member
        self.assertTrue(o.point["Cd"].min() >= 0 and o.point["Cd"].max() <= 1.0)
        self.assertTrue(np.isin(o.point["id"], g.point["id"]).all())
        np.testing.assert_array_equal(o.point["id"], g.point["id"][r.point_src])
        np.testing.assert_array_equal(o.prim["tag"], g.prim["tag"][r.prim_src])
        self.assertEqual(o.vertex["w"].shape, (o.nvertices,))
        self.assertEqual(set(g.point.names()), set(o.point.names()))

    def test_normals_are_renormalised(self):
        g = run("sphere", rows=60, columns=120)
        from minihou.nodes.normal import normal
        g = REGISTRY.get("normal").fn(g, **REGISTRY.get("normal").coerce_params({}))
        self.assertIn("N", g.point)
        r = proxy.decimate(g, 300, 1000)
        n = np.linalg.norm(r.geo.point["N"], axis=1)
        np.testing.assert_allclose(n, 1.0, atol=1e-4)

    def test_loose_points_are_sampled_and_curves_thinned(self):
        pts = Geometry.points_only(np.random.default_rng(1).random((5000, 3)).astype(np.float32))
        r = proxy.decimate(pts, 1000, 500)
        self.assertEqual(r.geo.npoints, 500)
        self.assertEqual(len(np.unique(r.point_src)), 500)
        # curves: 50 polylines of 20 points; budget for 200 points keeps every 5th curve
        P = np.random.default_rng(2).random((1000, 3)).astype(np.float32)
        curves = Geometry.from_arrays(P, np.arange(1000), np.arange(0, 1001, 20), closed=np.zeros(50, bool))
        r = proxy.decimate(curves, 1000, 200)
        self.assertTrue(r.reduced)
        self.assertLessEqual(r.geo.nprims, 10)
        self.assertEqual(r.geo.npoints, r.geo.nprims * 20)
        self.assertFalse(r.geo.topology.closed_mask().any())

    def test_a_mesh_with_curves_and_loose_points_keeps_all_three(self):
        mesh = run("grid", columns=40, rows=40)
        from minihou.nodes.merge import merge
        helix = run("helix", points=100)
        loose = Geometry.points_only(np.random.default_rng(3).random((400, 3)).astype(np.float32))
        both = merge([mesh, helix, loose])
        r = proxy.decimate(both, 300, 150)
        r.geo.validate()
        closed = r.geo.topology.closed_mask()
        self.assertTrue(closed.any() and (~closed).any())
        used = np.zeros(r.geo.npoints, bool)
        used[r.geo.topology.vertex_point] = True
        self.assertTrue((~used).any())                                      # loose points survived

    def test_degenerate_input_does_not_crash(self):
        empty = Geometry.points_only(np.zeros((0, 3), np.float32))
        self.assertFalse(proxy.decimate(empty, 10, 10).reduced)
        flat = Geometry.from_arrays(np.zeros((12, 3), np.float32), np.arange(12), np.arange(0, 13, 3))   # all points identical
        r = proxy.decimate(flat, 1, 100)
        r.geo.validate()
        self.assertEqual(r.geo.nprims, 0)


if __name__ == "__main__":
    unittest.main()
