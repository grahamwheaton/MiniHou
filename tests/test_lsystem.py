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

    def test_independent_axes_and_initial_orientation(self):
        settings = dict(rules="", generations=0, step=1, separate_angles=True,
                        yaw=90, pitch=90, roll=90)
        g = grow(premise="F+F&F", **settings)
        np.testing.assert_allclose(g.P, [[0,0,0], [0,0,1], [-1,0,1], [-1,-1,1]], atol=1e-6)
        self.assertEqual(np.linalg.matrix_rank(g.P[1:] - g.P[0]), 3)
        g = grow(premise="F", initial_orientation=[90,0,0], **settings)
        np.testing.assert_allclose(g.P[-1], [-1,0,0], atol=1e-6)
        g = grow(premise="F", initial_orientation=[0,90,0], **settings)
        np.testing.assert_allclose(g.P[-1], [0,-1,0], atol=1e-6)
        g = grow(premise="+F", initial_orientation=[0,0,90], **settings)
        np.testing.assert_allclose(g.P[-1], [0,-1,0], atol=1e-6)

    def test_roll_changes_subsequent_turn_and_inverse_rotations(self):
        settings = dict(rules="", generations=0, step=1, separate_angles=True,
                        yaw=90, pitch=90, roll=90)
        rolled = grow(premise="\\+F", **settings)
        np.testing.assert_allclose(rolled.P[-1], [0,-1,0], atol=1e-6)
        inverse = grow(premise="+-&^\\/F", **settings)
        np.testing.assert_allclose(inverse.P[-1], [0,0,1], atol=1e-6)
        g = grow(premise="+F&F\\+F" * 100, **settings)
        np.testing.assert_allclose(np.linalg.norm(np.diff(g.P, axis=0), axis=1), 1, atol=1e-6)
        np.testing.assert_allclose(np.linalg.norm(g.point["N"], axis=1), 1, atol=1e-6)

    def test_branch_restores_full_frame_step_and_width(self):
        g = grow(premise='F[+&\\!"F]F+F&F', rules="", generations=0,
                 angle=90, step=1, width=1, width_scale=0.5, step_scale=0.25)
        np.testing.assert_allclose(g.P[3:], [[0,0,2], [-1,0,2], [-1,-1,2]], atol=1e-6)
        np.testing.assert_allclose(g.point["width"][3:], 1)
        self.assertAlmostEqual(float(np.linalg.norm(g.P[2]-g.P[1])), 0.25)
        with self.assertRaises(NodeError):
            grow(premise="F[+F", rules="", generations=0)

    def test_planar_compatibility_and_angles_are_opt_in(self):
        params = dict(premise="F[+F][-F]F", rules="", generations=0, angle=30)
        old = grow(**params)
        ignored = grow(**params, yaw=5, pitch=80, roll=140)
        np.testing.assert_array_equal(old.P, ignored.P)
        np.testing.assert_array_equal(old.P[:,1], 0)

    def test_3d_examples_are_deterministic_and_nonplanar(self):
        import json
        from pathlib import Path
        root = Path(__file__).resolve().parents[1]
        for name in ("lsystem_3d_coral", "lsystem_3d_tree"):
            data = json.loads((root / "examples" / "graphs" / (name + ".json")).read_text())
            a = Evaluator().cook(Graph.from_dict(data))
            b = Evaluator().cook(Graph.from_dict(data))
            np.testing.assert_array_equal(a.P, b.P)
            self.assertEqual(np.linalg.matrix_rank(a.P - a.P.mean(axis=0)), 3)

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
