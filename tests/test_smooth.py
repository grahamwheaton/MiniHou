"""smooth (Taubin keeps volume, Laplacian shrinks; borders pinned) and match_size."""
import unittest

import numpy as np

from minihou import Evaluator, Graph
from minihou.nodes._util import fan_triangulate


def cook(nodes, display):
    nodes = dict(nodes)
    nodes["out"] = {"type": "output", "inputs": {"geo": display}}
    return Evaluator().cook(Graph.from_dict({"nodes": nodes, "display": "out"}))


def volume(g):
    tris, _ = fan_triangulate(g.topology)
    T = g.P[g.topology.vertex_point[tris]].astype(np.float64)
    return float(np.einsum("ij,ij->i", T[:, 0], np.cross(T[:, 1], T[:, 2])).sum() / 6.0)


NOISY = {"s": {"type": "sphere", "params": {"rows": 24, "columns": 48}},
         "n": {"type": "noise", "inputs": {"geo": "s"}, "params": {"amplitude": [0.1] * 3, "frequency": 4}}}


class SmoothTests(unittest.TestCase):
    def test_taubin_keeps_volume_and_removes_noise(self):
        noisy = cook(NOISY, "n")
        rough = np.linalg.norm(noisy.P, axis=1).std()
        tau = cook({**NOISY, "m": {"type": "smooth", "inputs": {"geo": "n"}, "params": {"iterations": 20}}}, "m")
        lap = cook({**NOISY, "m": {"type": "smooth", "inputs": {"geo": "n"},
                                   "params": {"iterations": 20, "method": "laplacian"}}}, "m")
        self.assertLess(np.linalg.norm(tau.P, axis=1).std(), 0.7 * rough)
        self.assertAlmostEqual(volume(tau) / volume(noisy), 1.0, delta=0.03)
        self.assertLess(volume(lap) / volume(noisy), 0.9)

    def test_borders_and_groups_stay(self):
        nodes = {"g": {"type": "grid"}, "n": {"type": "noise", "inputs": {"geo": "g"}, "params": {"amplitude": [0, 0, 1]}},
                 "m": {"type": "smooth", "inputs": {"geo": "n"}, "params": {"group": "@P.x < 0"}}}
        before, after = cook(nodes, "n"), cook(nodes, "m")
        fixed = (np.abs(before.P[:, 0]) > 4.99) | (np.abs(before.P[:, 1]) > 4.99) | (before.P[:, 0] >= 0)
        np.testing.assert_array_equal(after.P[fixed], before.P[fixed])
        self.assertFalse(np.allclose(after.P[~fixed], before.P[~fixed]))


class MatchSizeTests(unittest.TestCase):
    def test_fit_and_ground(self):
        box = {"type": "box", "params": {"size": [2, 4, 1], "center": [5, 5, 5]}}
        g = cook({"b": box, "m": {"type": "match_size", "inputs": {"geo": "b"},
                                  "params": {"justify_z": "min", "scale": True, "size": [1, 1, 1]}}}, "m")
        np.testing.assert_allclose(g.P.min(0), [-0.25, -0.5, 0.0], atol=1e-6)
        np.testing.assert_allclose(g.P.max(0), [0.25, 0.5, 0.25], atol=1e-6)
        g = cook({"b": box, "m": {"type": "match_size", "inputs": {"geo": "b"},
                                  "params": {"scale": True, "uniform": False, "size": [1, 2, 3], "center": [0, 0, 1]}}}, "m")
        np.testing.assert_allclose(g.P.max(0) - g.P.min(0), [1, 2, 3], atol=1e-6)
        np.testing.assert_allclose((g.P.max(0) + g.P.min(0)) / 2, [0, 0, 1], atol=1e-6)


if __name__ == "__main__":
    unittest.main()
