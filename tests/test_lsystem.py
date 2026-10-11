"""L-systems: rewriting (deterministic and stochastic), the turtle, branches, widths, leaves, guards."""
import unittest

import numpy as np

from minihou import Evaluator, Graph, NodeError
from minihou.nodes.lsystem import parse_rules, rewrite


def grow(**params):
    return Evaluator().cook(Graph.from_dict({"nodes": {
        "l": {"type": "lsystem", "params": params}, "o": {"type": "output", "inputs": {"geo": "l"}}}, "display": "o"}))


class RewriteTests(unittest.TestCase):
    def test_rules_and_generations(self):
        self.assertEqual(rewrite("A", parse_rules("A=AB;B=A"), 4, 0), "ABAABABA")   # Fibonacci words
        self.assertEqual(len(rewrite("F", parse_rules("F=F+F-F-F+F"), 3, 0).replace("+", "").replace("-", "")), 125)
        stoch = parse_rules("F=0.5:FF\nF=0.5:F+F")
        a, b = rewrite("F", stoch, 6, 1), rewrite("F", stoch, 6, 1)
        self.assertEqual(a, b)                                            # same seed, same plant
        self.assertNotEqual(a, rewrite("F", stoch, 6, 2))
        with self.assertRaises(ValueError):
            parse_rules("FF=F")
        with self.assertRaises(ValueError):
            parse_rules("F F")


class TurtleTests(unittest.TestCase):
    def test_square_path_and_lengths(self):
        g = grow(premise="F+F+F+F", rules="", generations=0, angle=90, step=1)
        self.assertEqual(g.nprims, 1)                                     # one unbroken curve
        np.testing.assert_allclose(g.P[0], g.P[-1], atol=1e-6)            # back where it started
        seg = np.linalg.norm(np.diff(g.P[g.topology.vertex_point], axis=0), axis=1)
        np.testing.assert_allclose(seg, 1.0, atol=1e-6)
        self.assertFalse(g.topology.closed_mask()[0])

    def test_branches_share_points_and_widths_shrink(self):
        g = grow(premise="F[+F]F", rules="", generations=0, angle=45, step=1, width=1.0, width_scale=0.5)
        self.assertEqual(g.nprims, 3)                                     # trunk, branch, trunk continued
        from minihou.nodes.attrib_tools import piece_labels
        self.assertEqual(int(piece_labels(g)[0].max()), 0)                 # all connected
        self.assertEqual(sorted(set(np.round(g.point["width"], 4).tolist())), [0.5, 1.0])
        self.assertAlmostEqual(float(g.P[:, 2].max()), 2.0, places=5)     # trunk: two steps up (+Z)

    def test_leaves_pitch_roll_and_errors(self):
        g = grow(premise="F&FJ", rules="", generations=0, angle=90, step=1)
        leaf = g.point["group_leaves"] == 1
        self.assertEqual(int(leaf.sum()), 1)
        self.assertAlmostEqual(float(abs(g.P[leaf][0][2])), 1.0, places=5)  # pitched sideways after one step up
        with self.assertRaises(NodeError):
            grow(premise="F]", rules="", generations=0)
        with self.assertRaises(NodeError):
            grow(premise="F", rules="F=FFFF", generations=12)              # explosive growth is refused


if __name__ == "__main__":
    unittest.main()
