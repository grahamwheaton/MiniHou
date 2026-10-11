"""polyreduce: hits the target, keeps closed shells closed and manifold, keeps the shape, borders and attributes."""
import unittest

import numpy as np

from minihou import Evaluator, Graph, NodeError
from minihou.nodes._util import fan_triangulate
from minihou.nodes.bevel import twins


def cook(nodes, display):
    nodes = dict(nodes)
    nodes["out"] = {"type": "output", "inputs": {"geo": display}}
    g = Evaluator().cook(Graph.from_dict({"nodes": nodes, "display": "out"}))
    g.validate()
    return g


def volume(g):
    tris, _ = fan_triangulate(g.topology)
    T = g.P[g.topology.vertex_point[tris]].astype(np.float64)
    return float(np.einsum("ij,ij->i", T[:, 0], np.cross(T[:, 1], T[:, 2])).sum() / 6.0)


def area(g):
    tris, _ = fan_triangulate(g.topology)
    T = g.P[g.topology.vertex_point[tris]].astype(np.float64)
    return float(np.linalg.norm(np.cross(T[:, 1] - T[:, 0], T[:, 2] - T[:, 0]), axis=1).sum() / 2.0)


def reduce(src, **params):
    nodes = {"s": src, "r": {"type": "polyreduce", "inputs": {"geo": "s"}, "params": params}}
    return cook(nodes, "s"), cook(nodes, "r")


class PolyReduceTests(unittest.TestCase):
    def test_sphere_keeps_its_shape(self):
        before, g = reduce({"type": "sphere", "params": {"rows": 48, "columns": 96}}, percentage=10)
        n0 = len(fan_triangulate(before.topology)[0])
        self.assertLessEqual(g.nprims, int(np.ceil(n0 * 0.1)) + 2)
        self.assertTrue(np.all(g.topology.prim_sizes() == 3))
        self.assertTrue(np.all(twins(g) >= 0), "closed, manifold, consistently wound")
        self.assertAlmostEqual(volume(g) / volume(before), 1.0, delta=0.01)
        r = np.linalg.norm(g.P, axis=1)
        self.assertLess(np.abs(r - 1.0).max(), 0.01)                     # points stay on the sphere
        _, tiny = reduce({"type": "sphere", "params": {"rows": 48, "columns": 96}}, target=200)
        self.assertLessEqual(tiny.nprims, 202)
        self.assertTrue(np.all(twins(tiny) >= 0))

    def test_flat_regions_and_sharp_edges(self):
        before, g = reduce({"type": "box", "params": {"divisions": 10}}, percentage=10)
        self.assertLessEqual(g.nprims, 120)
        self.assertAlmostEqual(volume(g), 1.0, places=5)                  # corners and edges stay exact
        self.assertTrue(np.all(twins(g) >= 0))

    def test_borders_stay(self):
        before, g = reduce({"type": "grid", "params": {"columns": 40, "rows": 40}}, percentage=5)
        self.assertLess(g.nprims, 0.06 * 2 * 39 * 39)
        self.assertAlmostEqual(area(g), 100.0, places=3)                  # the 10 x 10 outline is kept
        np.testing.assert_allclose(g.P.min(0), before.P.min(0), atol=1e-6)
        np.testing.assert_allclose(g.P.max(0), before.P.max(0), atol=1e-6)

    def test_attributes_follow(self):
        nodes = {"t": {"type": "torus", "params": {"rows": 24, "columns": 64}},
                 "w": {"type": "wrangle", "inputs": {"geo": "t"}, "params": {"snippet": "f@h = @P.z"}},
                 "r": {"type": "polyreduce", "inputs": {"geo": "w"}, "params": {"percentage": 20}}}
        g = cook(nodes, "r")
        np.testing.assert_allclose(g.point["h"], g.P[:, 2], atol=0.02)    # each point keeps (about) its own value
        self.assertEqual(g.vertex["uv"].shape, (g.topology.nvertices, 2))
        with self.assertRaises(NodeError):
            cook({"c": {"type": "helix"}, "r": {"type": "polyreduce", "inputs": {"geo": "c"}}}, "r")


if __name__ == "__main__":
    unittest.main()
