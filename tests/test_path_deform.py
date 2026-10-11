"""path_deform: identity on a straight line, Pappus volume round a circle, up vector, twist, normals."""
import unittest

import numpy as np

from minihou import Evaluator, Graph, NodeError
from minihou.nodes._util import fan_triangulate


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


LINE_X = {"type": "line", "params": {"origin": [0, 0, 0], "direction": [1, 0, 0], "length": 10, "points": 5}}


class PathDeformTests(unittest.TestCase):
    def test_straight_line_is_identity(self):
        box = {"type": "box", "params": {"size": [4, 1, 0.5], "divisions": 3, "center": [2, 0, 0]}}
        g = cook({"b": box, "c": LINE_X, "d": {"type": "path_deform", "inputs": {"geo": "b", "curve": "c"}}}, "d")
        np.testing.assert_allclose(g.P, cook({"b": box}, "b").P, atol=1e-6)

    def test_circle_keeps_volume_and_closes(self):
        r = 2.0
        circle = {"type": "circle", "params": {"radius": r, "divisions": 256}}
        bar = {"type": "box", "params": {"size": [2 * np.pi * r, 0.2, 0.2], "divisions": 64}}
        g = cook({"b": bar, "c": circle, "d": {"type": "path_deform", "inputs": {"geo": "b", "curve": "c"},
                                               "params": {"stretch": True}}}, "d")
        self.assertAlmostEqual(volume(g), 0.04 * 2 * np.pi * r, delta=0.02 * 0.04 * 2 * np.pi * r)   # Pappus
        rad = np.hypot(g.P[:, 0], g.P[:, 1])
        self.assertTrue(np.all((rad > r - 0.15) & (rad < r + 0.15)))
        self.assertLess(np.abs(g.P[:, 2]).max(), 0.1 + 1e-5)                   # the up axis stays up

    def test_up_twist_and_normals(self):
        plank = {"type": "box", "params": {"size": [4, 1, 0.1], "center": [2, 0, 0], "divisions": 8}}
        nodes = {"b": plank, "n": {"type": "normal", "inputs": {"geo": "b"}}, "c": LINE_X}
        side = cook({**nodes, "d": {"type": "path_deform", "inputs": {"geo": "n", "curve": "c"},
                                    "params": {"up": [0, 1, 0]}}}, "d")
        self.assertAlmostEqual(float(np.ptp(side.P[:, 2])), 1.0, places=5)         # the plank stands on edge
        np.testing.assert_allclose(np.linalg.norm(side.point["N"], axis=1), 1.0, atol=1e-5)
        tw = cook({**nodes, "d": {"type": "path_deform", "inputs": {"geo": "b", "curve": "c"},
                                  "params": {"twist": 90, "stretch": True}}}, "d")     # 90 degrees at the plank's end
        end = tw.P[:, 0] > 9.99
        self.assertAlmostEqual(float(np.ptp(tw.P[end, 2])), 1.0, places=4)
        with self.assertRaises(NodeError):
            cook({"b": plank, "c": {"type": "blast", "inputs": {"geo": "b"}, "params": {"group": "*"}},
                  "d": {"type": "path_deform", "inputs": {"geo": "b", "curve": "c"}}}, "d")

    def test_up_vector_frames_stay_level(self):
        from minihou.nodes.normal import prim_normals
        from minihou.nodes._util import normalize_rows
        plank = {"type": "box", "params": {"size": [12, 0.9, 0.1], "center": [6, 0, 0], "divisions": 40}}
        helix = {"type": "helix", "params": {"radius": 1.6, "radius_end": 1.6, "height": 4, "turns": 1.25,
                                             "points": 200}}
        top = normalize_rows(prim_normals(cook({"b": plank}, "b")))[:, 2] > 0.99
        for frame, level in (("up_vector", True), ("minimal_rotation", False)):
            g = cook({"b": plank, "c": helix, "d": {"type": "path_deform", "inputs": {"geo": "b", "curve": "c"},
                                                    "params": {"stretch": True, "frame": frame}}}, "d")
            nz = normalize_rows(prim_normals(g))[top, 2]
            self.assertEqual(bool(nz.min() > 0.9), level, frame)      # a helix's minimal frames roll over


if __name__ == "__main__":
    unittest.main()
