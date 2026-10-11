"""The instance node: which object goes on each point, rotations Blender reads as Euler XYZ, merging lists."""
import unittest

import numpy as np

from minihou import Evaluator, Graph, NodeError
from minihou.nodes.instance import euler_xyz_to_matrix, matrix_to_euler_xyz


def cook(nodes, display):
    nodes = dict(nodes)
    nodes["out"] = {"type": "output", "inputs": {"geo": display}}
    return Evaluator().cook(Graph.from_dict({"nodes": nodes, "display": "out"}))


BASE = {"s": {"type": "sphere", "params": {"radius": 2, "rows": 6, "columns": 8}},
        "n": {"type": "normal", "inputs": {"geo": "s"}}}


class InstanceTests(unittest.TestCase):
    def test_attributes_alignment_and_scale(self):
        g = cook({**BASE, "i": {"type": "instance", "inputs": {"points": "n"},
                               "params": {"objects": "Tree Rock", "random_spin": 0, "scale_min": 2, "scale_max": 3}}}, "i")
        self.assertEqual(g.detail["instance_objects"], ["Tree", "Rock"])
        self.assertTrue(set(g.point["instance"].tolist()) <= {1, 2})
        R = euler_xyz_to_matrix(g.point["rot"].astype(np.float64))
        np.testing.assert_allclose(R[:, :, 2], g.point["N"], atol=1e-5)   # object +Z follows N
        s = g.point["scale"]
        self.assertTrue(np.all((s >= 2) & (s <= 3)) and np.allclose(s[:, 0], s[:, 1]))
        up = cook({**BASE, "i": {"type": "instance", "inputs": {"points": "n"},
                                "params": {"objects": "Tree", "align": "up", "random_spin": 90}}}, "i")
        R = euler_xyz_to_matrix(up.point["rot"].astype(np.float64))
        np.testing.assert_allclose(R[:, :, 2], np.tile([0, 0, 1], (len(R), 1)), atol=1e-6)   # upright, spun

    def test_pick_modes_groups_and_errors(self):
        g = cook({**BASE, "i": {"type": "instance", "inputs": {"points": "n"},
                               "params": {"objects": "A B C", "pick": "cycle", "group": "@P.z > 0"}}}, "i")
        up = g.P[:, 2] > 0
        self.assertTrue(np.all(g.point["instance"][~up] == 0))
        self.assertTrue(np.all(g.point["instance"][up] > 0))
        g = cook({**BASE, "v": {"type": "wrangle", "inputs": {"geo": "n"}, "params": {"snippet": "i@variant = @ptnum"}},
                  "i": {"type": "instance", "inputs": {"points": "v"}, "params": {"objects": "A B", "pick": "attribute"}}}, "i")
        self.assertEqual(g.point["instance"][:4].tolist(), [1, 2, 1, 2])
        with self.assertRaises(NodeError):
            cook({**BASE, "i": {"type": "instance", "inputs": {"points": "n"}, "params": {"objects": " "}}}, "i")

    def test_merge_combines_object_lists(self):
        nodes = {**BASE,
                 "a": {"type": "instance", "inputs": {"points": "n"}, "params": {"objects": "Tree Rock", "pick": "cycle"}},
                 "b": {"type": "instance", "inputs": {"points": "n"}, "params": {"objects": "Rock Bush", "pick": "cycle"}},
                 "m": {"type": "merge", "inputs": {"geos": ["a", "s", "b"]}}}
        g = cook(nodes, "m")
        k = cook(BASE, "n").npoints
        self.assertEqual(g.detail["instance_objects"], ["Tree", "Rock", "Bush"])
        inst = g.point["instance"]
        self.assertEqual(inst[:2].tolist(), [1, 2])
        self.assertTrue(np.all(inst[k:2 * k] == 0))
        self.assertEqual(inst[2 * k:2 * k + 2].tolist(), [2, 3])            # Rock, Bush in the combined list

    def test_euler_round_trip_including_poles(self):
        from minihou.nodes._util import rotation_from_z_to
        rng = np.random.default_rng(1)
        N = rng.normal(size=(500, 3))
        N[:4] = [[0, 0, 1], [0, 0, -1], [1, 0, 0], [0, 1e-9, -1]]
        N /= np.linalg.norm(N, axis=1, keepdims=True)
        R = rotation_from_z_to(N)
        np.testing.assert_allclose(euler_xyz_to_matrix(matrix_to_euler_xyz(R)), R, atol=1e-9)


if __name__ == "__main__":
    unittest.main()
