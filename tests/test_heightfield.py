"""heightfield_erode: droplet erosion and thermal slumping on grids."""
import unittest

import numpy as np

from minihou import Evaluator, Graph, NodeError

TERRAIN = {"g": {"type": "grid", "params": {"columns": 64, "rows": 64, "size_x": 10, "size_y": 10}},
           "n": {"type": "noise", "inputs": {"geo": "g"}, "params": {"amplitude": [0, 0, 2], "frequency": 0.2,
                                                                      "octaves": 4, "seed": 3}}}


def cook(params, extra=None):
    nodes = dict(TERRAIN, e={"type": "heightfield_erode", "inputs": {"geo": "n"}, "params": params}, **(extra or {}))
    last = list(nodes)[-1]
    nodes["o"] = {"type": "output", "inputs": {"geo": last}}
    ev = Evaluator()
    g = Graph.from_dict({"nodes": nodes, "display": "o"})
    return ev.cook(g), ev.cook(g, "n")


def max_slope(g, rows=64, cols=64, cell=10 / 63):
    H = g.P[:, 2].reshape(rows, cols)
    return max(np.abs(np.diff(H, axis=0)).max(), np.abs(np.diff(H, axis=1)).max()) / cell


class HeightfieldTests(unittest.TestCase):
    def test_droplets_carve_and_deposit_deterministically(self):
        params = {"iterations": 6, "droplets": 1500, "thermal_iterations": 0, "seed": 2}
        a, src = cook(params)
        b, _ = cook(params)
        np.testing.assert_array_equal(a.P, b.P)                          # same seed, same terrain
        d = a.P[:, 2] - src.P[:, 2]
        self.assertLess(d.min(), -0.01)                                   # some places carved...
        self.assertGreater(d.max(), 0.01)                                 # ...and some filled
        self.assertLess(np.abs(d).max(), 2.0)                             # nothing blew up
        for name in ("flow", "erosion", "deposit"):
            self.assertIn(name, a.point)
        self.assertTrue(0 <= a.point["flow"].min() and a.point["flow"].max() <= 1)
        self.assertLess(abs(float(a.P[:, 2].mean() - src.P[:, 2].mean())), 0.05)   # material mostly moved, not lost

    def test_thermal_relaxes_steep_slopes(self):
        steep = {"s": {"type": "wrangle", "inputs": {"geo": "n"}, "params": {"snippet": "@P.z *= 4"}}}
        nodes = dict(TERRAIN, **steep,
                     e={"type": "heightfield_erode", "inputs": {"geo": "s"},
                        "params": {"iterations": 0, "thermal_iterations": 60, "talus_angle": 30}},
                     o={"type": "output", "inputs": {"geo": "e"}})
        ev = Evaluator()
        g = Graph.from_dict({"nodes": nodes, "display": "o"})
        after, before = ev.cook(g), ev.cook(g, "s")
        self.assertLess(max_slope(after), 0.75 * max_slope(before))
        self.assertAlmostEqual(float(after.P[:, 2].mean()), float(before.P[:, 2].mean()), places=4)  # mass kept

    def test_needs_a_grid(self):
        nodes = {"s": {"type": "sphere"}, "e": {"type": "heightfield_erode", "inputs": {"geo": "s"}},
                 "o": {"type": "output", "inputs": {"geo": "e"}}}
        with self.assertRaises(NodeError) as cm:
            Evaluator().cook(Graph.from_dict({"nodes": nodes, "display": "o"}))
        self.assertIn("grid", str(cm.exception))


if __name__ == "__main__":
    unittest.main()
