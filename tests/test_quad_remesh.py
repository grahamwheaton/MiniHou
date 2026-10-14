"""Quad Remesh: field-aligned, quad-dominant retopology -- checked by quad fraction, watertightness and volume."""
import collections
import time
import unittest

import numpy as np

from minihou import Evaluator, Graph, NodeError, topo
from minihou.nodes.normal import prim_normals


def cook(nodes, display=None):
    nodes = dict(nodes)
    last = display or list(nodes)[-1]
    nodes["o"] = {"type": "output", "inputs": {"geo": last}}
    g = Evaluator().cook(Graph.from_dict({"nodes": nodes, "display": "o"}))
    g.validate()
    return g


def closed(g):
    return bool(len(g.P)) and bool(np.all(topo.edges(g).count == 2))


def volume(g):
    return float(np.einsum("ij,ij->", prim_normals(g), topo.prim_centroids(g))) / 6


def quad_fraction(g):
    sizes = collections.Counter(g.topology.prim_sizes().tolist())
    return sizes.get(4, 0) / max(g.nprims, 1)


BOX = {"b": {"type": "box", "params": {"size": [2, 2, 2]}}}
SPHERE = {"s": {"type": "sphere", "params": {"radius": 1.0, "rows": 48, "columns": 72}}}
TORUS = {"t": {"type": "torus", "params": {"radius": 1.0, "thickness": 0.35, "rows": 64, "columns": 96}}}


class QuadRemeshTests(unittest.TestCase):
    def test_sphere_is_closed_mostly_quads_and_the_right_size(self):
        g = cook({**SPHERE, "q": {"type": "quad_remesh", "inputs": {"geo": "s"},
                                  "params": {"edge_length": 0.15, "seed": 0}}})
        self.assertTrue(closed(g))
        self.assertGreater(quad_fraction(g), 0.6)                          # quad-dominant, not triangles
        self.assertAlmostEqual(volume(g), 4 / 3 * np.pi, delta=0.16)
        self.assertLess(g.nprims, 5 * SPHERE["s"]["params"]["rows"] * SPHERE["s"]["params"]["columns"])

    def test_box_keeps_its_shape(self):
        g = cook({**BOX, "q": {"type": "quad_remesh", "inputs": {"geo": "b"}, "params": {"seed": 0}}})
        self.assertTrue(closed(g))
        self.assertGreater(quad_fraction(g), 0.6)
        self.assertAlmostEqual(volume(g), 8.0, delta=0.5)

    def test_torus_keeps_its_shape(self):
        g = cook({**TORUS, "q": {"type": "quad_remesh", "inputs": {"geo": "t"},
                                 "params": {"edge_length": 0.12, "seed": 0}}})
        self.assertTrue(closed(g))
        self.assertAlmostEqual(volume(g), 2 * np.pi ** 2 * 1.0 * 0.35 ** 2, delta=0.12)

    def test_edge_length_controls_density(self):
        coarse = cook({**SPHERE, "q": {"type": "quad_remesh", "inputs": {"geo": "s"},
                                       "params": {"edge_length": 0.30, "seed": 0}}})
        fine = cook({**SPHERE, "q": {"type": "quad_remesh", "inputs": {"geo": "s"},
                                     "params": {"edge_length": 0.12, "seed": 0}}})
        self.assertLess(coarse.npoints, fine.npoints)

    def test_adaptivity_finishes_quickly(self):
        t = time.time()
        g = cook({**SPHERE, "q": {"type": "quad_remesh", "inputs": {"geo": "s"},
                                  "params": {"edge_length": 0.25, "adaptivity": 2.0, "seed": 0}}})
        self.assertLess(time.time() - t, 30.0)                             # bounded sample count, no runaway
        self.assertTrue(closed(g))

    def test_transfer_carries_point_attributes_from_the_nearest_original_point(self):
        g = cook({**BOX, "c": {"type": "color", "inputs": {"geo": "b"}, "params": {"color": [1, 0, 0]}},
                  "q": {"type": "quad_remesh", "inputs": {"geo": "c"}, "params": {"seed": 0, "transfer": "Cd"}}})
        np.testing.assert_allclose(g.point["Cd"], np.tile([1, 0, 0], (g.npoints, 1)))

    def test_curves_are_refused(self):
        with self.assertRaises(NodeError):
            cook({"l": {"type": "line"}, "q": {"type": "quad_remesh", "inputs": {"geo": "l"}}})

    def test_empty_geometry_passes_through(self):
        g = cook({**BOX, "x": {"type": "blast", "inputs": {"geo": "b"}, "params": {"group": "*"}},
                  "q": {"type": "quad_remesh", "inputs": {"geo": "x"}}})
        self.assertEqual(g.nprims, 0)


if __name__ == "__main__":
    unittest.main()
