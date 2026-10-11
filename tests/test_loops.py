"""Repeat regions: feedback, validation, deterministic keys and existing graph contracts."""
import tempfile
import unittest
from dataclasses import replace
from pathlib import Path
from unittest.mock import patch

import numpy as np

from minihou import Evaluator, Graph, GraphError
from minihou.edit import EditError, apply_ops
from minihou.library import NodeLibrary
from minihou.link.session import Session
from minihou.sdk import REGISTRY, ParamError
from tests.helpers import quad_geo

ROOT = Path(__file__).resolve().parents[1]


def repeat_graph(count=3):
    graph = Graph.load(ROOT / "examples/graphs/repeat_transform.json")
    graph.set_param("end", "iterations", count)
    return graph


class RepeatTests(unittest.TestCase):
    def test_zero_one_many_feedback_and_input_immutable(self):
        ev = Evaluator()
        initial = ev.cook(repeat_graph(), "box")
        original = initial.P.copy()
        for count in (0, 1, 7):
            result = ev.cook(repeat_graph(count))
            np.testing.assert_allclose(result.P, original + [count * .5, 0, 0])
            self.assertTrue(result.frozen)
            self.assertNotIn("iteration", result.detail)
        np.testing.assert_array_equal(initial.P, original)
        self.assertNotIn("begin", ev.last_stats.cooked)

    def test_metadata_inside_body_and_restored_after(self):
        g = repeat_graph(4)
        g.nodes["step"].type = "wrangle"
        g.nodes["step"].params = {"snippet": "@P.x += @iteration + 1; @P.y += @numiterations;"}
        incoming = quad_geo()
        incoming.detail.update(iteration=90, numiterations=100, keep="hello")
        out = Evaluator().cook(g, feeds={"box": ("initial", incoming.freeze())})
        np.testing.assert_allclose(out.P, incoming.P + [10, 16, 0])
        self.assertEqual(dict(out.detail), dict(incoming.detail))

    def test_nested_feedback_and_outer_metadata(self):
        g = repeat_graph(3)
        g.nodes["step"].type = "wrangle"
        g.nodes["step"].params = {"snippet": "@P.x += @iteration + 1;"}
        g.add_node("inner_begin", "loop_begin", inputs={"geo": "step"})
        g.add_node("inner_step", "wrangle", params={"snippet": "@P.y += @iteration + 1;"}, inputs={"geo": "inner_begin"})
        g.add_node("inner_end", "loop_end", params={"iterations": 2}, inputs={"geo": "inner_step", "begin": "inner_begin"})
        g.add_node("after", "wrangle", params={"snippet": "@P.z += @iteration + @numiterations;"}, inputs={"geo": "inner_end"})
        g.connect("end", "geo", "after")
        g.exposed = []
        g.check()
        ev = Evaluator()
        initial = ev.cook(g, "box")
        out = ev.cook(g)
        np.testing.assert_allclose(out.P, initial.P + [6, 9, 12])
        self.assertNotIn("iteration", out.detail)
        g.set_param("inner_end", "iterations", 0)
        np.testing.assert_allclose(ev.cook(g).P, initial.P + [6, 0, 12])

    def test_external_inputs_are_constant_and_cached(self):
        g = repeat_graph(3)
        g.add_node("external", "box")
        g.add_node("join", "merge", inputs={"geos": ["step", "external"]})
        g.connect("end", "geo", "join")
        ev = Evaluator()
        out = ev.cook(g)
        self.assertEqual(out.npoints, 32)
        self.assertEqual(ev.last_stats.cooked.count("external"), 0)  # same content as initial box
        before = ev.last_stats.key
        g.set_param("external", "size", [2, 2, 2])
        self.assertEqual(ev.cook(g).npoints, 32)
        self.assertNotEqual(before, ev.last_stats.key)

    def test_cache_invalidation_body_count_input_rewire_and_version(self):
        g = repeat_graph(3)
        ev = Evaluator()
        first = ev.cook(g)
        self.assertIs(first, ev.cook(g))
        self.assertEqual(ev.last_stats.cooked, [])
        key = ev.last_stats.key
        g.nodes["step"].meta["pos"] = [100, 200]
        self.assertIs(first, ev.cook(g))
        g.set_param("step", "translate", [2, 0, 0])
        np.testing.assert_allclose(ev.cook(g).P, first.P + [4.5, 0, 0])
        self.assertNotEqual(key, ev.last_stats.key)
        g.set_param("end", "iterations", 1)
        np.testing.assert_allclose(ev.cook(g).P, first.P + [.5, 0, 0])
        g.set_param("box", "size", [4, 4, 4])
        self.assertEqual(float(np.ptp(ev.cook(g).P[:, 0])), 4)
        g.connect("end", "geo", "begin")
        np.testing.assert_allclose(ev.cook(g).P, ev.cook(g, "box").P)
        nt = REGISTRY.get("loop_end")
        REGISTRY.unregister("loop_end")
        REGISTRY.register(replace(nt, version=nt.version + 1))
        self.addCleanup(lambda: (REGISTRY.unregister("loop_end"), REGISTRY.register(nt)))
        ev.cook(g)
        self.assertIn("end", ev.last_stats.cooked)

    def test_iteration_keys_do_not_alias_preview_or_other_instances(self):
        g = repeat_graph(2)
        ev = Evaluator()
        a, b = quad_geo(), quad_geo()
        b.P = b.P + [10, 0, 0]
        first = ev.cook(g, feeds={"box": ("a", a.freeze())})
        second = ev.cook(g, feeds={"box": ("b", b.freeze())})
        np.testing.assert_allclose(second.P, first.P + [10, 0, 0])
        self.assertIs(first, ev.cook(g, feeds={"box": ("a", a)}))
        with self.assertRaisesRegex(GraphError, "display.*End"):
            ev.cook(g, "step")

    def test_system_feed_still_prunes_unfinished_preview_branch(self):
        g = repeat_graph(2)
        g.nodes["box"].type = "input"
        g.nodes["box"].inputs = {"preview": "not_connected_yet"}
        incoming = quad_geo().freeze()
        out = Evaluator().cook(g, feeds={"box": ("supplied-system-input", incoming)})
        np.testing.assert_allclose(out.P, incoming.P + [1, 0, 0])

    def test_count_rejects_negative_fractional_and_runaway(self):
        g = repeat_graph()
        for value in (-1, 1001, 1.5, True):
            with self.subTest(value=value), self.assertRaises(ParamError):
                g.set_param("end", "iterations", value)
        with patch("minihou.loops.MAX_TOTAL_ITERATIONS", 2):
            with self.assertRaisesRegex(GraphError, "budget exceeded"):
                Evaluator().cook(g)
        self.assertIsNotNone(Evaluator().cook(g))  # failed cook resets the context-local budget

    def test_nesting_depth_and_nested_total_work_are_bounded(self):
        g = Graph.from_dict({"nodes": {"seed": {"type": "box"}}, "display": "seed"})
        src = "seed"
        for i in range(17):
            g.add_node(f"begin{i}", "loop_begin", inputs={"geo": src})
            src = f"begin{i}"
        for i in reversed(range(17)):
            g.add_node(f"end{i}", "loop_end", params={"iterations": 1}, inputs={"geo": src, "begin": f"begin{i}"})
            src = f"end{i}"
        g.display = src
        with self.assertRaisesRegex(GraphError, "nesting exceeds"):
            Evaluator().cook(g)
        g.connect("end15", "geo", "begin15")
        g.remove_node("end16")
        g.remove_node("begin16")
        g.check()
        self.assertEqual(Evaluator().cook(g).npoints, 8)
        g.set_param("end0", "iterations", 1000)
        g.set_param("end1", "iterations", 1000)
        with patch("minihou.loops.MAX_TOTAL_ITERATIONS", 25):
            with self.assertRaisesRegex(GraphError, "budget exceeded"):
                Evaluator().cook(g)

    def test_body_error_skipped_only_for_zero_and_library_version_invalidates(self):
        g = repeat_graph(0)
        g.nodes["step"].type = "wrangle"
        g.nodes["step"].params = {"snippet": "@P.x += does_not_exist;"}
        ev = Evaluator()
        self.assertEqual(ev.cook(g).npoints, 8)
        g.set_param("end", "iterations", 1)
        with self.assertRaisesRegex(RuntimeError, "does_not_exist"):
            ev.cook(g)
        g = repeat_graph(2)
        first = ev.cook(g)
        old = REGISTRY.get("transform")
        def changed(geo, **params):
            params["translate"] = [1, 0, 0]
            return old.fn(geo, **params)
        REGISTRY.unregister("transform")
        REGISTRY.register(replace(old, fn=changed, code_hash="edited-test-body"))
        try:
            np.testing.assert_allclose(ev.cook(g).P, first.P + [1, 0, 0])
        finally:
            REGISTRY.unregister("transform")
            REGISTRY.register(old)

    def test_bad_pairing_and_cycles_are_errors_even_zero_count(self):
        for case in ("wrong_type", "missing_begin", "multiple", "unpaired", "disconnected", "cycle", "crossed", "multi_body", "multi_initial"):
            g = repeat_graph(0)
            if case == "wrong_type":
                g.connect("end", "begin", "box")
            elif case == "missing_begin":
                del g.nodes["end"].inputs["begin"]
            elif case == "multiple":
                g.add_node("other", "loop_end", inputs={"begin": "begin", "geo": "step"})
                g.add_node("both", "merge", inputs={"geos": ["end", "other"]})
                g.connect("out", "geo", "both")
            elif case == "unpaired":
                g.remove_node("end")
                g.connect("out", "geo", "step")
            elif case == "disconnected":
                g.connect("end", "geo", "box")
            elif case == "cycle":
                g.connect("begin", "geo", "end")
            elif case == "crossed":
                g.add_node("inner", "loop_begin", inputs={"geo": "step"})
                g.connect("end", "geo", "inner")
            elif case == "multi_body":
                g.nodes["end"].inputs["geo"] = ["step"]
            elif case == "multi_initial":
                g.nodes["begin"].inputs["geo"] = ["box"]
            with self.subTest(case=case):
                self.assertTrue(g.validate())
                with self.assertRaises(GraphError):
                    Evaluator().cook(g)

    def test_sibling_blocks_cannot_mix_unclosed_bodies(self):
        g = repeat_graph()
        g.add_node("b2", "loop_begin", inputs={"geo": "box"})
        g.add_node("join", "merge", inputs={"geos": ["step", "b2"]})
        g.connect("end", "geo", "join")
        with self.assertRaisesRegex(GraphError, "crossed"):
            Evaluator().cook(g)

    def test_unfinished_unrelated_branch_does_not_block_cook(self):
        g = repeat_graph()
        g.add_node("unfinished", "loop_begin")
        self.assertTrue(g.validate())
        self.assertEqual(Evaluator().cook(g).npoints, 8)

    def test_roundtrip_rename_insert_undo_and_redo(self):
        g = Graph.from_dict({"nodes": {"box": {"type": "box"}}, "display": "box"})
        session = Session(g)
        result = session.edit([{"op": "loop", "from": "box", "iterations": 5, "pos": [10, 20]}])
        pair = result["results"][0]
        session.edit([{"op": "display", "id": pair["end"]}, {"op": "rename", "id": pair["begin"], "to": "renamed"}])
        self.assertEqual(session.graph.nodes[pair["end"]].inputs["begin"], "renamed")
        self.assertEqual(session.graph.to_dict(), Graph.from_json(session.graph.to_json()).to_dict())
        session.undo()
        self.assertIn(pair["begin"], session.graph.nodes)
        session.undo()
        self.assertEqual(list(session.graph.nodes), ["box"])
        session.redo()
        self.assertIn(pair["end"], session.graph.nodes)
        with self.assertRaises(EditError):
            apply_ops(session.graph, [{"op": "connect", "from": pair["end"], "to": pair["begin"], "port": "geo"}])

    def test_insertion_with_downstream_wire_is_atomic(self):
        g = Graph.from_dict({"nodes": {"box": {"type": "box"}, "out": {"type": "output", "inputs": {"geo": "box"}}}})
        edited, results = apply_ops(g, [{"op": "loop", "from": "box", "to": "out", "port": "geo"}])
        self.assertEqual(edited.nodes["out"].inputs["geo"], results[0]["end"])
        edited.check()
        with self.assertRaises(EditError):
            apply_ops(g, [{"op": "loop", "from": "out", "to": "out"}])
        self.assertEqual(len(g.nodes), 2)

    def test_system_body_and_repeat_inside_system(self):
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp) / "repeat_system_test.json"
            g = repeat_graph(2)
            g.nodes["box"].type = "input"
            g.nodes["box"].params = {"name": "geo"}
            g.save(path)
            lib = NodeLibrary([temp])
            try:
                lib.refresh()
                self.assertFalse(lib.status()["files"][0]["error"])
                outer = repeat_graph(3)
                outer.nodes["step"].type = "repeat_system_test"
                outer.nodes["step"].params = {}
                ev = Evaluator()
                initial = ev.cook(outer, "box")
                np.testing.assert_allclose(ev.cook(outer).P, initial.P + [3, 0, 0])
                g.set_param("step", "translate", [1, 0, 0])
                g.save(path)
                lib.refresh()
                np.testing.assert_allclose(ev.cook(outer).P, initial.P + [6, 0, 0])
            finally:
                lib.unload()

    def test_time_in_body_invalidates_repeat(self):
        g = repeat_graph(2)
        g.nodes["step"].type = "wrangle"
        g.nodes["step"].params = {"snippet": "@P.x += @Frame;"}
        ev = Evaluator()
        a = ev.cook(g, frame=1)
        b = ev.cook(g, frame=3)
        np.testing.assert_allclose(b.P, a.P + [4, 0, 0])


if __name__ == "__main__":
    unittest.main()
