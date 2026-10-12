"""External-engine contracts for staged Geometry Nodes placements.

Native operations are represented by explicit array edits here; actual Blender
socket evaluation is tested separately in test_blender_geonodes.py.
"""
import copy
from pathlib import Path
import unittest

import numpy as np

from minihou import Graph
from minihou.link.instance_client import InstanceClient
from minihou.link.server import LinkServer
from minihou.link.session import Session


TRANSFORM = {
    "nodes": {
        "input": {"type": "modifier_input"},
        "move": {"type": "transform", "inputs": {"geo": "input"}},
        "out": {"type": "output", "inputs": {"geo": "move"}},
    },
    "display": "out",
    "exposed": [
        {"name": "Move", "node": "move", "param": "translate"},
        {"name": "Scale", "node": "move", "param": "uniform_scale"},
    ],
}


def upstream():
    return {
        "point/P": np.array([[0, 0, 0], [1, 0, 0], [1, 1, 0], [0, 1, 0]], np.float32),
        "point/weight": np.array([0.1, 0.3, 0.7, 0.9], np.float32),
        "vertex/uv": np.array([[0, 0], [1, 0], [1, 1], [0, 0], [1, 1], [0, 1]], np.float32),
        "prim/material_index": np.array([0, 2], np.int32),
        "prim/region": np.array([5, 9], np.int32),
        "topo/vertex_point": np.array([0, 1, 2, 0, 2, 3], np.int32),
        "topo/prim_offsets": np.array([0, 3, 6], np.int32),
    }


class GeometryNodesEngineContracts(unittest.TestCase):
    def setUp(self):
        self.session = Session(Graph.from_dict({"nodes": {"grid": {"type": "grid"}}, "display": "grid"}))
        self.server = LinkServer(self.session, port=0)
        self.server.start()
        self.client = InstanceClient(port=self.server.port)

    def tearDown(self):
        self.client.close()
        self.server.stop()

    def cook(self, ident, arrays, params=None, graph=None):
        return self.client.cook(ident, "", params or {}, arrays, graph=graph or TRANSFORM)[1]

    def assert_payload(self, actual, expected):
        for key, array in expected.items():
            self.assertIn(key, actual)
            np.testing.assert_array_equal(actual[key], array, err_msg=key)

    def chain(self, source, move=(2, 0, 0), scale=3):
        before = copy.deepcopy(source)
        before["point/P"] *= 2  # native operation before placement A
        a = self.cook("object-1/A", before, {"Move": list(move)})
        between = copy.deepcopy(a)
        between["point/P"] += np.array([0, 4, 0], np.float32)
        b = self.cook("object-1/B", between, {"Scale": scale})
        return a, b

    def test_two_placements_native_stages_preserve_attributes_and_material_indices(self):
        source = upstream()
        original = copy.deepcopy(source)
        a, b = self.chain(source)
        np.testing.assert_array_equal(a["point/P"], source["point/P"] * 2 + [2, 0, 0])
        np.testing.assert_array_equal(b["point/P"], (source["point/P"] * 2 + [2, 4, 0]) * 3)
        for result in (a, b):
            self.assert_payload(result, {k: v for k, v in source.items() if k != "point/P"})
        self.assert_payload(source, original)
        self.assertEqual(self.session.graph.output_node(), "grid")

    def test_upstream_and_each_parameter_change_propagate_without_cross_placement_leak(self):
        source = upstream()
        a, b = self.chain(source)
        a2, b2 = self.chain(source, scale=5)
        self.assert_payload(a2, a)
        np.testing.assert_array_equal(b2["point/P"], b["point/P"] / 3 * 5)
        a3, b3 = self.chain(source, move=(7, 0, 0))
        np.testing.assert_array_equal(a3["point/P"], a["point/P"] + [5, 0, 0])
        np.testing.assert_array_equal(b3["point/P"], b["point/P"] + [15, 0, 0])
        changed = copy.deepcopy(source)
        changed["point/P"][2] += [0, 0, 2]
        changed["point/weight"][:] = 0.5
        changed["prim/material_index"][:] = [2, 0]
        _, output = self.chain(changed)
        np.testing.assert_array_equal(output["point/P"], (changed["point/P"] * 2 + [2, 4, 0]) * 3)
        self.assert_payload(output, {k: v for k, v in changed.items() if k != "point/P"})
        duplicate = self.cook("object-2/A", source, {"Move": [-10, 0, 0]})
        np.testing.assert_array_equal(duplicate["point/P"], source["point/P"] + [-10, 0, 0])
        self.assert_payload(self.chain(source)[1], b)

    def test_graph_snapshots_and_repeat_placements_are_independent(self):
        repeat = Graph.load(Path(__file__).resolve().parents[1] / "examples/modifiers/repeat_transform.json").to_dict()
        snapshot = copy.deepcopy(repeat)
        source = upstream()
        a = self.cook("repeat/A", source, {"Repeat Count": 3, "Step": [2, 0, 0]}, repeat)
        b = self.cook("repeat/B", a, {"Repeat Count": 2, "Step": [0, 1, 0]}, snapshot)
        np.testing.assert_array_equal(b["point/P"], source["point/P"] + [6, 2, 0])
        self.assert_payload(b, {k: v for k, v in source.items() if k != "point/P"})
        repeat["nodes"]["step"]["params"]["translate"] = [4, 0, 0]
        edited = self.cook("repeat/A", source, {"Repeat Count": 2}, repeat)
        np.testing.assert_array_equal(edited["point/P"], source["point/P"] + [8, 0, 0])
        unchanged = self.cook("repeat/B", source, {"Repeat Count": 2}, snapshot)
        np.testing.assert_array_equal(unchanged["point/P"], source["point/P"] + [1, 0, 0])
        self.assert_payload(self.cook("repeat/A", source, {"Repeat Count": 0}, repeat), source)

    def test_failed_cook_keeps_connection_other_instance_and_allows_recovery(self):
        source = upstream()
        good = self.cook("A", source, {"Move": [2, 0, 0]})
        other = self.cook("B", source, {"Scale": 4})
        connection = self.client.sock
        bad = copy.deepcopy(source)
        bad["point/P"][0, 0] = np.nan
        with self.assertRaisesRegex(RuntimeError, "finite"):
            self.cook("A", bad)
        with self.assertRaisesRegex(RuntimeError, "exposed"):
            self.cook("A", source, {"Unknown": 1})
        self.assertIs(self.client.sock, connection)
        self.assert_payload(self.cook("B", source, {"Scale": 4}), other)
        self.assert_payload(self.cook("A", source, {"Move": [2, 0, 0]}), good)
        self.client.remove("A")
        self.assert_payload(self.cook("A", source), source)


if __name__ == "__main__":
    unittest.main()
