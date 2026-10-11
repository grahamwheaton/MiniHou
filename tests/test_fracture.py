"""voronoi_fracture: closed pieces that add up to the input, cut exactly along bisector planes; exploded_view."""
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


def volume(g, mask=None):
    tris, tp = fan_triangulate(g.topology)
    if mask is not None:
        tris = tris[mask[tp]]
    T = g.P[g.topology.vertex_point[tris]].astype(np.float64)
    return float(np.einsum("ij,ij->i", T[:, 0], np.cross(T[:, 1], T[:, 2])).sum() / 6.0)


def fracture(src, count, **params):
    nodes = {"s": src, "p": {"type": "scatter", "inputs": {"geo": "s"}, "params": {"count": count, "seed": 3}},
             "f": {"type": "voronoi_fracture", "inputs": {"geo": "s", "points": "p"}, "params": params}}
    return cook(nodes, "s"), cook(nodes, "p"), cook(nodes, "f")


class FractureTests(unittest.TestCase):
    def test_pieces_are_closed_and_add_up(self):
        for src in ({"type": "box", "params": {"size": [2, 1, 1]}}, {"type": "torus"},
                    {"type": "sphere", "params": {"rows": 12, "columns": 24}}):
            with self.subTest(src=src["type"]):
                orig, seeds, g = fracture(src, 25)
                self.assertTrue(np.all(twins(g) >= 0), "every piece is a closed, consistently wound shell")
                self.assertAlmostEqual(volume(g), volume(orig), places=4)
                piece = g.prim["piece"]
                for k in np.unique(piece):
                    self.assertGreater(volume(g, piece == k), 0.0)
                self.assertGreater(int(g.prim["group_inside"].sum()), 0)

    def test_points_lie_in_their_own_cell(self):
        _, seeds, g = fracture({"type": "box", "params": {"size": [2, 2, 2]}}, 12)
        S = seeds.P.astype(np.float64)
        vp = g.topology.vertex_point
        owner = g.prim["piece"][np.repeat(np.arange(g.nprims), g.topology.prim_sizes())]
        P = g.P[vp].astype(np.float64)
        d_own = np.linalg.norm(P - S[owner], axis=1)
        d_min = np.linalg.norm(P[:, None] - S[None], axis=2).min(1)
        np.testing.assert_allclose(d_own, d_min, atol=1e-4)                 # no corner is nearer another seed

    def test_gap_and_open_input(self):
        orig, _, g = fracture({"type": "box"}, 8, gap=0.05)
        self.assertLess(volume(g), volume(orig))
        self.assertTrue(np.all(twins(g) >= 0))
        grid = {"g": {"type": "grid"}, "p": {"type": "scatter", "inputs": {"geo": "g"}, "params": {"count": 6}},
                "f": {"type": "voronoi_fracture", "inputs": {"geo": "g", "points": "p"}}}
        g = cook(grid, "f")
        self.assertEqual(int(g.prim["group_inside"].sum()), 0)             # nothing to cap on an open sheet
        self.assertEqual(len(np.unique(g.prim["piece"])), 6)
        with self.assertRaises(NodeError):
            cook({"c": {"type": "helix"}, "p": {"type": "box"},
                  "f": {"type": "voronoi_fracture", "inputs": {"geo": "c", "points": "p"}}}, "f")

    def test_exploded_view(self):
        nodes = {"s": {"type": "sphere"}, "p": {"type": "scatter", "inputs": {"geo": "s"}, "params": {"count": 8}},
                 "f": {"type": "voronoi_fracture", "inputs": {"geo": "s", "points": "p"}},
                 "e": {"type": "exploded_view", "inputs": {"geo": "f"}, "params": {"scale": 1.0}}}
        f, e = cook(nodes, "f"), cook(nodes, "e")
        self.assertAlmostEqual(volume(e), volume(f), places=4)                # rigid moves
        self.assertGreater(np.linalg.norm(e.P, axis=1).mean(), np.linalg.norm(f.P, axis=1).mean() + 0.2)
        with self.assertRaises(NodeError):
            cook({"s": {"type": "sphere"}, "e": {"type": "exploded_view", "inputs": {"geo": "s"}}}, "e")


if __name__ == "__main__":
    unittest.main()
