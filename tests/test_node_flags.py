import unittest
import numpy as np

from minihou import Graph, Evaluator
from minihou.edit import apply_ops
from minihou.link.session import Session


def scene():
    return Graph.from_dict({"nodes": {
        "box": {"type": "box"},
        "move": {"type": "transform", "inputs": {"geo": "box"}, "params": {"translate": [4, 0, 0]}},
        "other": {"type": "box", "params": {"center": [-4, 0, 0]}},
        "out": {"type": "output", "inputs": {"geo": "move"}}}, "display": "out"})


class NodeFlagsTests(unittest.TestCase):
    def setUp(self):
        self.g = scene()
        self.ev = Evaluator()
        self.addCleanup(self.ev.close)

    def flag(self, name, nid="move", value=True, **kw):
        self.g, _ = apply_ops(self.g, [{"op": "flag", "id": nid, "flag": name, "value": value, **kw}])

    def test_bypass_invalidates_downstream_and_restores(self):
        before = self.ev.cook(self.g).P.copy()
        self.flag("bypass")
        np.testing.assert_allclose(self.ev.cook(self.g).P, before - [4, 0, 0])
        self.flag("bypass", value=False)
        np.testing.assert_array_equal(self.ev.cook(self.g).P, before)

    def test_bypassed_generator_is_empty(self):
        self.flag("bypass", "box")
        self.assertEqual(self.ev.cook(self.g).npoints, 0)

    def test_bypass_skips_broken_secondary_input(self):
        self.g.add_node("broken", "transform")
        self.g.add_node("copies", "copy_to_points", inputs={"source": "box", "points": "broken"})
        self.flag("bypass", "copies")
        self.assertEqual(self.ev.cook(self.g, "copies").npoints, 8)
        self.assertNotIn("broken", self.ev.last_stats.cooked)

    def test_render_follows_display_until_separated_and_survives_rename(self):
        self.assertEqual(self.g.render_node(), "out")
        self.flag("render", "box")
        self.flag("display", "move")
        self.assertEqual(self.g.render_node(), "box")
        self.g, _ = apply_ops(self.g, [{"op": "rename", "id": "box", "to": "cube"}])
        self.assertEqual(self.g.render_node(), "cube")
        restored = Graph.from_dict(self.g.to_dict())
        self.assertEqual(restored.render_node(), "cube")
        restored.remove_node("cube")
        self.assertIsNone(restored.render)

    def test_template_exclusive_additive_and_roundtrip(self):
        self.flag("template", "box")
        self.flag("template", "move")
        self.assertFalse(self.g.nodes["box"].meta["flags"]["template"])
        self.flag("selectable_template", "other", additive=True)
        restored = Graph.from_dict(self.g.to_dict())
        self.assertTrue(restored.nodes["move"].meta["flags"]["template"])
        self.assertTrue(restored.nodes["other"].meta["flags"]["selectable_template"])

    def test_lock_persists_attributes_and_ignores_upstream(self):
        s = Session(self.g)
        self.addCleanup(s.stop)
        expected = self.ev.cook(self.g, "move").P.copy()
        s.edit([{"op": "flag", "id": "move", "flag": "lock", "value": True}])
        s.edit([{"op": "set", "id": "box", "params": {"size": [9, 9, 9]}}])
        restored = Graph.from_dict(s.graph.to_dict())
        np.testing.assert_array_equal(self.ev.cook(restored).P, expected)
        self.assertNotIn("box", self.ev.last_stats.cooked)
        s.edit([{"op": "flag", "id": "move", "flag": "lock", "value": False}])
        self.assertFalse(np.array_equal(self.ev.cook(s.graph).P, expected))
        s.undo()
        np.testing.assert_array_equal(self.ev.cook(s.graph).P, expected)

    def test_template_preview_and_selectable_merge(self):
        self.flag("template", "box")
        self.flag("selectable_template", "other", additive=True)
        s = Session(self.g)
        self.addCleanup(s.stop)
        r = s.preview("out", templates=True)
        self.assertTrue(r["ok"], r)
        self.assertEqual(r["geo"].npoints, 16)
        self.assertEqual(r["reference"].npoints, 8)
        self.assertEqual(r["selectable"], ["other"])
        self.assertEqual(self.ev.cook(s.graph).npoints, 8)
        merged, result = apply_ops(self.g, [{"op": "merge_templates", "id": "out"}])
        np.testing.assert_array_equal(self.ev.cook(merged, result[0]["id"]).P, r["geo"].P)
        self.assertFalse(merged.nodes["other"].meta["flags"]["selectable_template"])

    def test_broken_template_does_not_break_display(self):
        self.g.add_node("bad", "transform")
        self.flag("template", "bad")
        s = Session(self.g)
        self.addCleanup(s.stop)
        r = s.preview("out", templates=True)
        self.assertTrue(r["ok"])
        self.assertTrue(r["template_warnings"])

    def test_instance_template_expands_when_display_is_plain_mesh(self):
        from types import SimpleNamespace
        from unittest.mock import patch
        self.flag("template", "other")
        s = Session(self.g)
        self.addCleanup(s.stop)
        s.live_modifier = SimpleNamespace(connected=True, sources={})
        main = self.ev.cook(self.g, "out")
        ref = self.ev.cook(self.g, "other").copy()
        ref.detail["instance_objects"] = ["Monkey"]
        with patch.object(s, "_modifier_feeds", return_value=None), \
             patch.object(s.evaluator, "cook", side_effect=[main, ref]), \
             patch("minihou.link.live.preview_instances", return_value=ref) as expand:
            result = s.preview("out", templates=True)
        self.assertTrue(result["ok"])
        expand.assert_called_once_with(ref, {})

    def test_unload_releases_upstream_but_keeps_display(self):
        self.flag("unload", "box")
        keys = self.ev.compute_keys(self.g, self.g.upstream_order("out"))
        self.ev.cook(self.g)
        self.assertIsNone(self.ev.cache.get(keys["box"]))
        self.ev.cook(self.g, "box")
        self.assertIsNotNone(self.ev.cache.get(keys["box"]))

    def test_bad_flags_rejected(self):
        for value in ({"bypass": "yes"}, {"invisible": True}, []):
            data = self.g.to_dict()
            data["nodes"]["box"]["flags"] = value
            with self.assertRaises(ValueError):
                Graph.from_dict(data, check=False)
