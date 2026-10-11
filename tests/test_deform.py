"""bend / twist / taper, point_jitter, point_relax, clean, and the spatial pair query under them."""
import unittest

import numpy as np

from minihou import Evaluator, Geometry, Graph, spatial


def cook(nodes):
    nodes = dict(nodes)
    last = list(nodes)[-1]
    nodes["o"] = {"type": "output", "inputs": {"geo": last}}
    g = Evaluator().cook(Graph.from_dict({"nodes": nodes, "display": "o"}))
    g.validate()
    return g


LINE = {"l": {"type": "line", "params": {"length": 2, "points": 41}}}


def nearest_neighbour(P):
    i, j, d = spatial.pairs_within(P, 2.0)
    m = np.full(len(P), np.inf)
    np.minimum.at(m, i, d)
    np.minimum.at(m, j, d)
    return m


class BendTests(unittest.TestCase):
    def test_bend_keeps_length_and_follows_the_arc(self):
        g = cook({**LINE, "b": {"type": "bend", "inputs": {"geo": "l"}, "params": {"length": 2, "bend": 90}}})
        R = 2 / (np.pi / 2)
        np.testing.assert_allclose(g.P[-1], [R, 0, R], atol=1e-4)
        self.assertAlmostEqual(float(np.linalg.norm(np.diff(g.P, axis=0), axis=1).sum()), 2.0, places=3)
        r = np.linalg.norm(g.P[:, [0, 2]] - [R, 0], axis=1)
        np.testing.assert_allclose(r, R, atol=1e-4)                      # every point on the arc

    def test_beyond_the_region_and_twist_taper(self):
        g = cook({"l": {"type": "line", "params": {"length": 4, "points": 41}},
                  "b": {"type": "bend", "inputs": {"geo": "l"}, "params": {"length": 2, "bend": 180}}})
        np.testing.assert_allclose(g.P[-1], [4 / np.pi, 0, -2], atol=1e-4)   # straight on after the half circle
        box = {"b": {"type": "box", "params": {"size": [1, 1, 2], "center": [0, 0, 1], "divisions": 4}}}
        tw = cook({**box, "t": {"type": "bend", "inputs": {"geo": "b"},
                                "params": {"length": 2, "bend": 0, "twist": 90, "taper": 0.5}}})
        top = tw.P[np.isclose(tw.P[:, 2], 2)]
        np.testing.assert_allclose(np.abs(top[:, :2]).max(), 0.25, atol=1e-5)   # tapered to half
        bottom = tw.P[np.isclose(tw.P[:, 2], 0)]
        np.testing.assert_allclose(np.abs(bottom[:, :2]).max(), 0.5, atol=1e-5)

    def test_normals_follow_and_groups(self):
        g = cook({"g": {"type": "grid", "params": {"columns": 2, "rows": 20, "size_x": 0.1, "size_y": 2,
                                                  "center": [0, 1, 0]}},
                  "n": {"type": "normal", "inputs": {"geo": "g"}},
                  "b": {"type": "bend", "inputs": {"geo": "n"}, "params": {"direction": [0, 1, 0], "up": [1, 0, 0],
                                                                          "length": 2, "bend": 90}}})
        np.testing.assert_allclose(np.linalg.norm(g.point["N"], axis=1), 1, atol=1e-5)
        self.assertLess(float(g.point["N"][g.P[:, 1].argmax()][2]), 0.9)     # the tip's normal turned
        part = cook({**LINE, "b": {"type": "bend", "inputs": {"geo": "l"}, "params": {"length": 2, "bend": 90,
                                                                                    "group": "0-9"}}})
        np.testing.assert_allclose(part.P[10:, 0], 0)                        # outside the group: untouched


class RelaxJitterCleanTests(unittest.TestCase):
    def test_pairs_within_matches_brute_force(self):
        rng = np.random.default_rng(3)
        P = rng.random((300, 3)) * [3, 3, 0.2]
        i, j, d = spatial.pairs_within(P, 0.3)
        D = np.linalg.norm(P[:, None] - P[None], axis=2)
        want = {(a, b) for a, b in zip(*np.nonzero(np.triu(D < 0.3, 1)))}
        self.assertEqual({(min(a, b), max(a, b)) for a, b in zip(i.tolist(), j.tolist())}, want)

    def test_relax_spreads_points_on_the_surface(self):
        nodes = {"s": {"type": "sphere", "params": {"radius": 1, "rows": 16, "columns": 24}},
                 "p": {"type": "scatter", "inputs": {"geo": "s"}, "params": {"count": 300, "seed": 4}}}
        before = cook(dict(nodes))
        after = cook({**nodes, "r": {"type": "point_relax", "inputs": {"points": "p", "surface": "s"},
                                     "params": {"iterations": 15}}})
        self.assertGreater(nearest_neighbour(after.P).min(), 3 * nearest_neighbour(before.P).min())
        np.testing.assert_allclose(np.linalg.norm(after.P, axis=1), 1, atol=0.02)   # still on the sphere
        self.assertTrue(np.all(np.einsum("ij,ij->i", after.point["N"], after.P) > 0.9))

    def test_jitter_and_clean(self):
        j = cook({**LINE, "j": {"type": "point_jitter", "inputs": {"geo": "l"}, "params": {"scale": [0.1, 0, 0]}}})
        self.assertTrue(np.all(np.abs(j.P[:, 0]) <= 0.1) and np.any(j.P[:, 0] != 0))
        np.testing.assert_allclose(j.P[:, 1], 0)
        P = np.array([[0, 0, 0], [1, 0, 0], [1, 1, 0], [0, 1, 0], [5, 5, 5]], np.float32)
        g = Geometry.from_arrays(P, [0, 1, 1, 2, 3, 0, 1, 2, 2, 2], [0, 6, 8, 9, 10])
        from minihou.nodes.deform import clean
        c = clean(g, 0.0, True, True)
        self.assertEqual(c.topology.polys(), [[0, 1, 2, 3]])                  # repeats removed, degenerates dropped
        self.assertEqual(c.npoints, 4)                                       # the unused point is gone


if __name__ == "__main__":
    unittest.main()
