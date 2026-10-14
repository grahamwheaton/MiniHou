"""scene_objects' wired inputs (source / collection from other objects) and the blender_collection node."""
import unittest

import numpy as np

import minihou  # noqa: F401
from minihou import Evaluator, Geometry, Graph
from minihou.inputs import INPUTS
from minihou.link.session import Session
from minihou.nodes.scene_objects import build_manifest
from minihou.sdk import REGISTRY


def tri():
    return Geometry.from_arrays(np.array([[0, 0, 0], [1, 0, 0], [1, 1, 0]], np.float32), np.array([0, 1, 2], np.int32),
                                np.array([0, 3], np.int32))


def put(name, x, coll="Props"):
    m = np.eye(4)
    m[0, 3] = x
    INPUTS.put(name, tri(), [float(v) for v in m.reshape(-1)], info={"collection": coll})


class SceneInputTests(unittest.TestCase):
    def setUp(self):
        INPUTS.clear()
        for n, x in (("Rock", 0.0), ("Tree", 5.0), ("Bush", 9.0)):
            put(n, x)
        INPUTS.put_collection("Props", ["Bush", "Rock", "Tree"])

    def tearDown(self):
        INPUTS.clear()

    def graph(self, so_inputs, **params):
        nodes = {
            "g": {"type": "grid", "params": {"columns": 3, "rows": 3}},
            "p": {"type": "scatter", "inputs": {"geo": "g"}, "params": {"count": 6, "seed": 1}},
            "one": {"type": "blender_object", "params": {"object": "Tree"}},
            "col": {"type": "blender_collection", "params": {"collection": "Props"}},
            "so": {"type": "scene_objects", "inputs": dict(geo="p", **so_inputs), "params": params},
            "out": {"type": "output", "inputs": {"geo": "g"}},
        }
        return Graph.from_dict({"nodes": nodes, "display": "out"})

    def manifest(self, so_inputs, **params):
        g = self.graph(so_inputs, **params)
        geo = Evaluator().cook(g, "so")
        p = REGISTRY.get("scene_objects").coerce_params(params)
        return build_manifest("so", p, geo, "k")[0]

    def test_a_wired_object_is_the_source(self):
        m = self.manifest({"source_object": "one"})
        self.assertEqual(m["sources"], ["Tree"])
        self.assertEqual(m["src"], [0] * 6)

    def test_a_wired_collection_gives_every_member_cycled(self):
        m = self.manifest({"source_object": "col"})
        self.assertEqual(m["sources"], ["Bush", "Rock", "Tree"])
        self.assertEqual(m["src"], [0, 1, 2, 0, 1, 2])

    def test_pick_first_and_random_are_deterministic(self):
        self.assertEqual(self.manifest({"source_object": "col"}, pick="first")["src"], [0] * 6)
        a = self.manifest({"source_object": "col"}, pick="random", seed=4)["src"]
        self.assertEqual(a, self.manifest({"source_object": "col"}, pick="random", seed=4)["src"])
        self.assertTrue(all(0 <= i < 3 for i in a))

    def test_the_wired_input_overrides_the_parameter(self):
        m = self.manifest({"source_object": "one"}, source="Rock")
        self.assertEqual(m["sources"], ["Tree"])
        self.assertEqual(self.manifest({}, source="Rock")["sources"], ["Rock"])

    def test_the_collection_comes_from_an_object_or_a_collection_input(self):
        self.assertEqual(self.manifest({"source_object": "one", "target_collection": "one"})["collection"], "Props")
        self.assertEqual(self.manifest({"source_object": "one", "target_collection": "col"}, collection="Other")["collection"], "Props")
        self.assertEqual(self.manifest({"source_object": "one"}, collection="Other")["collection"], "Other")

    def test_a_source_name_in_a_detail_attribute_works(self):
        nodes = {"p": {"type": "scatter", "inputs": {"geo": "g"}, "params": {"count": 4}},
                 "g": {"type": "grid"},
                 "so": {"type": "scene_objects", "inputs": {"geo": "p"}, "params": {}}}
        g = Graph.from_dict({"nodes": nodes, "display": "p"})
        pts = Evaluator().cook(g, "p").copy()
        pts.detail["source"] = "Bush"
        from minihou.nodes.scene_objects import names_from
        self.assertEqual(names_from(pts, "source"), ["Bush"])
        self.assertEqual(names_from(pts, "collection"), [])

    def test_blender_collection_points(self):
        geo = Evaluator().cook(self.graph({}), "col")
        self.assertEqual(geo.detail["instance_objects"], ["Bush", "Rock", "Tree"])
        np.testing.assert_allclose(geo.P[:, 0], [9, 0, 5])
        self.assertEqual(geo.detail["collection_name"], "Props")

    def test_blender_collection_pattern_and_geometry(self):
        g = self.graph({})
        g.nodes["col"].params.update(pattern="^Rock")
        geo = Evaluator().cook(g, "col")
        self.assertEqual(geo.detail["collection_objects"], ["Bush", "Tree"])
        g.nodes["col"].params.update(output="geometry", pattern="")
        geo = Evaluator().cook(g, "col")
        self.assertEqual(geo.npoints, 9)

    def test_missing_collection_is_a_clear_error(self):
        g = self.graph({})
        g.nodes["col"].params.update(collection="Nope")
        with self.assertRaises(Exception) as cm:
            Evaluator().cook(g, "col")
        self.assertIn("waiting for Blender collection", str(cm.exception))

    def test_session_watches_members_of_watched_collections(self):
        s = Session(self.graph({}))
        self.assertEqual(s.watched_collections(), ["Props"])
        self.assertEqual(s.watched_objects(), ["Bush", "Rock", "Tree"])
        s.collection_received("Props", ["Rock"])
        self.assertEqual(s.watched_objects(), ["Rock", "Tree"])       # Tree: the blender_object node


if __name__ == "__main__":
    unittest.main()
