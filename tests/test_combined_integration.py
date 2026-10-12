"""Cross-feature contracts for external repeats and evaluator-owned RBD worlds."""
import importlib.util
import unittest

import numpy as np

from minihou import Evaluator, Graph, GraphError


def node(kind, params=None, **inputs):
    return {"type": kind, "params": params or {}, "inputs": inputs}


def scene():
    return {
        "box": node("box", {"center": [0, 0, 3]}),
        "body": node("rbd_configure", geo="box"),
        "sim": node("rbd_solver", {"gravity": [0, 0, -9.81], "include_colliders": False}, bodies="body"),
    }


def repeat(nodes, incoming, count=3):
    nodes["begin"] = node("loop_begin", geo=incoming)
    nodes["step"] = node("transform", {"translate": [1, 0, 0]}, geo="begin")
    nodes["end"] = node("loop_end", {"iterations": count}, geo="step", begin="begin")


class CombinedRepeatValidation(unittest.TestCase):
    def test_rbd_inside_repeat_rejected_even_when_zero_iterations(self):
        for count in (0, 2):
            with self.subTest(iterations=count):
                nodes = scene()
                repeat(nodes, "box", count)
                nodes["body"]["inputs"]["geo"] = "begin"
                nodes["step"]["inputs"]["geo"] = "sim"
                with self.assertRaisesRegex(GraphError, "RBD|rbd|stateful|solver"):
                    Graph.from_dict({"nodes": nodes, "display": "end"})
                graph = Graph.from_dict({"nodes": nodes, "display": "end"}, check=False)
                evaluator = Evaluator()
                try:
                    with self.assertRaisesRegex(GraphError, "RBD|rbd|stateful|solver"):
                        evaluator.cook(graph, frame=10)
                    self.assertFalse(evaluator.rbd.worlds)
                finally:
                    evaluator.close()


class ClaudeCodexIntegration(unittest.TestCase):
    def test_completed_repeat_inspector_and_named_output(self):
        from minihou.link.session import Session
        nodes = {"box": node("box")}
        repeat(nodes, "box", count=3)
        nodes["main"] = node("output", geo="box")
        nodes["named"] = node("output", {"object": "Repeated"}, geo="end")
        session = Session(Graph.from_dict({"nodes": nodes, "display": "main"}))
        self.addCleanup(session.stop)
        session.start()
        self.assertTrue(session.wait_idle())
        inspection = session.inspect("end")
        self.assertTrue(inspection["ok"], inspection)
        self.assertIn("flow", inspection)
        result = session.latest_good
        self.assertIsNotNone(result)
        np.testing.assert_allclose(result.extras["Repeated"][0].P, result.geo.P + [3, 0, 0])

    def test_named_output_proxy_and_modifier_share_server(self):
        from minihou.link.instance_client import InstanceClient
        from tests.test_link import Harness, wait_for
        from tests.test_modifier_instances import GRAPH, mesh
        from tests.test_outputs import TWO
        harness = Harness(Graph.from_dict({"nodes": TWO, "display": "main"}))
        self.addCleanup(harness.close)
        proxy, messages = harness.client()
        instance = InstanceClient(port=harness.port)
        self.addCleanup(instance.close)
        self.assertIn("instances_v1", instance.connect()["capabilities"])
        named, _ = wait_for(messages, "geo", pred=lambda h: h.get("output") == "Trees")
        self.assertTrue(named["full"])
        _, result = instance.cook("combined", "", {"Move": [2, 0, 0]}, mesh(), graph=GRAPH)
        np.testing.assert_array_equal(result["point/P"], mesh()["point/P"] + [2, 0, 0])
        harness.session.edit([{"op": "set", "id": "b", "params": {"size": [2, 2, 2]}}])
        changed, _ = wait_for(messages, "geo", pred=lambda h: h.get("output") == "Trees")
        self.assertFalse(changed["full"])
        _, again = instance.cook("combined", "", {"Move": [2, 0, 0]}, mesh(), graph=GRAPH)
        np.testing.assert_array_equal(again["point/P"], result["point/P"])


@unittest.skipUnless(importlib.util.find_spec("mujoco"), "optional physics extra not installed")
class CombinedRepeatPhysics(unittest.TestCase):
    def setUp(self):
        self.ev = Evaluator()
        self.other = Evaluator()
        self.addCleanup(self.ev.close)
        self.addCleanup(self.other.close)

    def test_static_repeat_before_rbd_isolated_world_and_replay(self):
        nodes = scene()
        repeat(nodes, "box")
        nodes["body"]["inputs"]["geo"] = "end"
        graph = Graph.from_dict({"nodes": nodes, "display": "sim"})
        initial = self.ev.cook(graph, frame=1)
        self.assertAlmostEqual(float(initial.P[:, 0].mean()), 3)
        first = self.ev.cook(graph, frame=18).P.copy()
        self.assertLess(float(first[:, 2].mean()), float(initial.P[:, 2].mean()))
        self.other.cook(graph, frame=4)
        world = self.ev.rbd.worlds["sim"][1]
        self.assertIsNot(world, self.other.rbd.worlds["sim"][1])
        np.testing.assert_array_equal(first, self.other.cook(graph, frame=18).P)
        # Evict both geometry and physics snapshots to force actual backward replay.
        self.ev.cache.clear()
        world.frames.clear()
        world.cache_bytes = 0
        self.ev.cook(graph, frame=3)
        np.testing.assert_array_equal(first, self.ev.cook(graph, frame=18).P)
        self.assertIs(world, self.ev.rbd.worlds["sim"][1])

    def test_repeat_after_rbd_transforms_pose_without_restarting_world(self):
        nodes = scene()
        repeat(nodes, "sim", count=4)
        graph = Graph.from_dict({"nodes": nodes, "display": "end"})
        result = self.ev.cook(graph, frame=12)
        raw = self.ev.cook(graph, target="sim", frame=12)
        world = self.ev.rbd.worlds["sim"][1]
        np.testing.assert_allclose(result.P, raw.P + [4, 0, 0], atol=1e-6)
        self.assertIs(result.topology, raw.topology)
        graph.set_param("end", "iterations", 2)
        changed = self.ev.cook(graph, frame=12)
        np.testing.assert_allclose(changed.P, raw.P + [2, 0, 0], atol=1e-6)
        self.assertIs(world, self.ev.rbd.worlds["sim"][1])
        next_pose = self.ev.cook(graph, frame=15)
        self.assertLess(float(next_pose.P[:, 2].mean()), float(changed.P[:, 2].mean()))
        self.assertNotIn("iteration", next_pose.detail)
        self.assertNotIn("numiterations", next_pose.detail)


if __name__ == "__main__":
    unittest.main()
