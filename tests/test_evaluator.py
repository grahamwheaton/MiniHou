import unittest

import numpy as np

from minihou import Cache, Evaluator, Geometry, NodeError
from minihou.sdk import Float, Input, Registry, Str, node
from tests.helpers import counting_registry, graph_from


class EvaluatorTests(unittest.TestCase):
    def chain(self, reg):
        return graph_from({
            "s": {"type": "src", "params": {"x": 1.0}},
            "a": {"type": "add", "params": {"dx": 10.0}, "inputs": {"geo": "s"}},
            "b": {"type": "add", "params": {"dx": 100.0}, "inputs": {"geo": "a"}},
        }, display="b", registry=reg)

    def test_cooks_in_dependency_order(self):
        reg, calls = counting_registry()
        ev = Evaluator(reg)
        out = ev.cook(self.chain(reg))
        self.assertEqual(out.P[0, 0], 111.0)
        self.assertEqual(ev.last_stats.cooked, ["s", "a", "b"])

    def test_second_cook_is_fully_cached_and_skips_upstream(self):
        reg, calls = counting_registry()
        ev = Evaluator(reg)
        g = self.chain(reg)
        ev.cook(g)
        n = len(calls)
        ev.cook(g)
        self.assertEqual(len(calls), n)
        self.assertEqual(ev.last_stats.cooked, [])
        self.assertEqual(ev.last_stats.cached, ["b"])          # upstream never even visited

    def test_changing_a_parameter_recooks_only_downstream(self):
        reg, calls = counting_registry()
        ev = Evaluator(reg)
        g = self.chain(reg)
        ev.cook(g)
        g.set_param("a", "dx", 20.0)
        out = ev.cook(g)
        self.assertEqual(out.P[0, 0], 121.0)
        self.assertEqual(ev.last_stats.cooked, ["a", "b"])
        self.assertEqual(ev.last_stats.cached, ["s"])
        g.set_param("s", "x", 2.0)
        ev.cook(g)
        self.assertEqual(ev.last_stats.cooked, ["s", "a", "b"])

    def test_untouched_sibling_branch_stays_cached(self):
        reg, calls = counting_registry()
        g = graph_from({
            "s1": {"type": "src", "params": {"x": 1.0}}, "s2": {"type": "src", "params": {"x": 2.0}},
            "a1": {"type": "add", "inputs": {"geo": "s1"}}, "a2": {"type": "add", "inputs": {"geo": "s2"}},
            "j": {"type": "join", "inputs": {"a": "a1", "b": "a2"}},
        }, display="j", registry=reg)
        ev = Evaluator(reg)
        ev.cook(g)
        g.set_param("s2", "x", 5.0)
        ev.cook(g)
        self.assertEqual(sorted(ev.last_stats.cooked), ["a2", "j", "s2"])
        self.assertEqual(sorted(ev.last_stats.cached), ["a1"])           # s1 behind a1 never visited

    def test_diamond_cooks_shared_node_once(self):
        reg, calls = counting_registry()
        g = graph_from({
            "s": {"type": "src"},
            "l": {"type": "add", "params": {"dx": 1.0}, "inputs": {"geo": "s"}},
            "r": {"type": "add", "params": {"dx": 2.0}, "inputs": {"geo": "s"}},
            "j": {"type": "join", "inputs": {"a": "l", "b": "r"}},
        }, display="j", registry=reg)
        ev = Evaluator(reg)
        out = ev.cook(g)
        self.assertEqual([c[0] for c in calls].count("src"), 1)
        self.assertEqual(out.P[:, 0].tolist(), [1.0, 2.0])

    def test_cache_is_content_addressed_not_id_addressed(self):
        reg, calls = counting_registry()
        g = graph_from({
            "s1": {"type": "src", "params": {"x": 3.0}}, "s2": {"type": "src", "params": {"x": 3.0}},
            "j": {"type": "join", "inputs": {"a": "s1", "b": "s2"}},
        }, display="j", registry=reg)
        ev = Evaluator(reg)
        ev.cook(g)
        self.assertEqual([c[0] for c in calls].count("src"), 1)         # identical node reused

    def test_going_back_to_an_old_value_hits_the_cache(self):
        reg, calls = counting_registry()
        g = self.chain(reg)
        ev = Evaluator(reg)
        ev.cook(g)
        g.set_param("a", "dx", 20.0)
        ev.cook(g)
        g.set_param("a", "dx", 10.0)
        ev.cook(g)
        self.assertEqual(ev.last_stats.cooked, [])

    def test_node_version_busts_cache(self):
        reg = Registry()

        def make(version):
            node("v", params=[Float("x", 0.0)], version=version, registry=reg, replace=True)(
                lambda x: Geometry.from_arrays(np.zeros((1, 3), np.float32)))

        make(1)
        g = graph_from({"v": {"type": "v"}}, registry=reg)
        ev = Evaluator(reg)
        ev.cook(g, "v")
        make(1)
        ev.cook(g, "v")
        self.assertEqual(ev.last_stats.cooked, [])
        make(2)
        ev.cook(g, "v")
        self.assertEqual(ev.last_stats.cooked, ["v"])

    def test_outputs_are_frozen_so_nodes_cannot_corrupt_the_cache(self):
        reg = Registry()

        @node("src", registry=reg)
        def src():
            return Geometry.from_arrays(np.zeros((2, 3), np.float32))

        @node("evil", inputs=["geo"], registry=reg)
        def evil(geo):
            geo.P[0, 0] = 99.0                    # mutates its (frozen) input in place
            return geo

        g = graph_from({"s": {"type": "src"}, "e": {"type": "evil", "inputs": {"geo": "s"}}}, registry=reg)
        with self.assertRaises(NodeError) as cm:
            Evaluator(reg).cook(g, "e")
        self.assertEqual(cm.exception.node_id, "e")

    def test_node_errors_carry_context_and_are_not_cached(self):
        reg = Registry()
        state = {"fail": True}

        @node("flaky", params=[Float("x", 0.0)], registry=reg)
        def flaky(x):
            if state["fail"]:
                raise RuntimeError("boom")
            return Geometry.from_arrays(np.zeros((1, 3), np.float32))

        g = graph_from({"f": {"type": "flaky"}}, registry=reg)
        ev = Evaluator(reg)
        with self.assertRaises(NodeError) as cm:
            ev.cook(g, "f")
        self.assertIn("boom", str(cm.exception))
        self.assertEqual(cm.exception.node_type, "flaky")
        state["fail"] = False
        ev.cook(g, "f")                           # recovers: failure was not cached
        self.assertEqual(ev.last_stats.cooked, ["f"])

    def test_node_returning_garbage_is_an_error(self):
        reg = Registry()
        node("bad", registry=reg)(lambda: 42)
        node("bad2", registry=reg)(lambda: Geometry(3))             # no P
        for t in ("bad", "bad2"):
            with self.assertRaises(NodeError):
                Evaluator(reg).cook(graph_from({"x": {"type": t}}, registry=reg), "x")

    def test_retain_drops_stale_entries(self):
        reg, calls = counting_registry()
        g = self.chain(reg)
        ev = Evaluator(reg)
        ev.cook(g)
        for dx in (1.0, 2.0, 3.0):
            g.set_param("a", "dx", dx)
            ev.cook(g)
        self.assertGreater(len(ev.cache), 3)
        dropped = ev.cache.retain(ev.live_keys(g))
        self.assertGreater(dropped, 0)
        self.assertEqual(len(ev.cache), 3)

    def test_lru_eviction_respects_byte_budget(self):
        reg = Registry()

        @node("big", params=[Float("x", 0.0)], registry=reg)
        def big(x):
            return Geometry.from_arrays(np.full((1000, 3), x, np.float32))      # 12 kB

        ev = Evaluator(reg, Cache(max_bytes=30_000))
        g = graph_from({"b": {"type": "big"}}, display="b", registry=reg)
        for x in range(5):
            g.set_param("b", "x", float(x))
            ev.cook(g)
        self.assertLessEqual(ev.cache.total_bytes, 30_000)
        self.assertGreaterEqual(len(ev.cache), 1)

    def test_shared_arrays_are_counted_once(self):
        reg, calls = counting_registry()
        ev = Evaluator(reg)
        g = graph_from({"g": {"type": "grid", "params": {"columns": 50, "rows": 50}},
                        "t": {"type": "transform", "inputs": {"geo": "g"}},
                        "o": {"type": "output", "inputs": {"geo": "t"}}}, display="o")
        ev = Evaluator()
        out = ev.cook(g)
        # topology, uv and (via passthrough) output share arrays; total must be < sum of entries.
        naive = sum(e.geo.nbytes for e in ev.cache._entries.values())
        self.assertLess(ev.cache.total_bytes, naive)

    def test_missing_input_target_and_default_output(self):
        g = graph_from({"g": {"type": "grid"}, "o": {"type": "output", "inputs": {"geo": "g"}}})
        ev = Evaluator()
        ev.cook(g)                                 # no display set: uses the output node
        self.assertEqual(ev.last_stats.target, "o")


if __name__ == "__main__":
    unittest.main()
