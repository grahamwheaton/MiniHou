"""copy_transform, peak, bound and switch."""
import unittest

import numpy as np

from minihou import Evaluator, Graph, NodeError


def cook(nodes, display):
    nodes = dict(nodes)
    nodes["out"] = {"type": "output", "inputs": {"geo": display}}
    g = Evaluator().cook(Graph.from_dict({"nodes": nodes, "display": "out"}))
    g.validate()
    return g


class UtilityNodeTests(unittest.TestCase):
    def test_copy_transform_accumulates(self):
        step = {"type": "box", "params": {"size": [1, 0.3, 0.1], "center": [0.5, 0, 0]}}
        g = cook({"b": step, "c": {"type": "copy_transform", "inputs": {"geo": "b"},
                                   "params": {"copies": 4, "translate": [0, 0, 0.2], "rotate": [0, 0, 90]}}}, "c")
        n = g.npoints // 4
        self.assertEqual(g.point["copynum"].tolist(), sum([[i] * n for i in range(4)], []))
        far = [g.P[g.point["copynum"] == i].mean(0) for i in range(4)]
        np.testing.assert_allclose(far[1], [0, 0.5, 0.2], atol=1e-6)        # turned 90 degrees, one step up
        np.testing.assert_allclose(far[3], [0, -0.5, 0.6], atol=1e-6)
        s = cook({"b": step, "c": {"type": "copy_transform", "inputs": {"geo": "b"},
                                   "params": {"copies": 3, "translate": [0, 0, 0], "scale": 0.5}}}, "c")
        ext = [np.ptp(s.P[s.point["copynum"] == i, 0]) for i in range(3)]
        np.testing.assert_allclose(ext, [1, 0.5, 0.25], atol=1e-6)

    def test_peak(self):
        g = cook({"s": {"type": "sphere", "params": {"rows": 32, "columns": 64}},
                  "p": {"type": "peak", "inputs": {"geo": "s"}, "params": {"distance": 0.5, "group": "@P.z > 0"}}}, "p")
        r = np.linalg.norm(g.P, axis=1)
        up = g.P[:, 2] > 0.01
        self.assertTrue(np.allclose(r[up], 1.5, atol=0.01) and np.allclose(r[g.P[:, 2] < -0.01], 1.0, atol=1e-5))

    def test_bound(self):
        g = cook({"t": {"type": "torus"}, "b": {"type": "bound", "inputs": {"geo": "t"},
                                                  "params": {"padding": [0.1, 0.1, 0.1]}}}, "b")
        np.testing.assert_allclose(g.detail["bound_size"], [2.7, 2.7, 0.7], atol=1e-5)
        self.assertEqual((g.npoints, g.nprims), (8, 6))
        r = cook({"t": {"type": "torus"}, "r": {"type": "transform", "inputs": {"geo": "t"},
                                                 "params": {"rotate": [30, 20, 10]}},
                  "b": {"type": "bound", "inputs": {"geo": "r"}, "params": {"oriented": True}}}, "b")
        np.testing.assert_allclose(sorted(r.detail["bound_size"]), [0.5, 2.5, 2.5], atol=0.01)   # a 32-gon ring

    def test_switch(self):
        nodes = {"a": {"type": "box"}, "s": {"type": "sphere"}}
        for i, n in ((0, 8), (1, 482), (7, 482)):
            g = cook({**nodes, "w": {"type": "switch", "inputs": {"geos": ["a", "s"]}, "params": {"index": i}}}, "w")
            self.assertEqual(g.npoints, n)
        with self.assertRaises((NodeError, Exception)):
            cook({"w": {"type": "switch"}}, "w")


if __name__ == "__main__":
    unittest.main()
