"""skin: quads across a row of curves."""
import unittest

import numpy as np

from minihou.geometry import Geometry
from minihou.nodes.bevel import twins
from tests.test_nodes import run


def rings(radii_z, counts):
    P, sizes = [], []
    for (r, z), n in zip(radii_z, counts):
        a = np.arange(n) * 2 * np.pi / n
        P.append(np.c_[r * np.cos(a), r * np.sin(a), np.full(n, z)])
        sizes.append(n)
    P = np.concatenate(P).astype(np.float32)
    offs = np.concatenate([[0], np.cumsum(sizes)]).astype(np.int32)
    return Geometry.from_arrays(P, np.arange(len(P), dtype=np.int32), offs)


def volume(g):
    P, vp, offs = g.P.astype(float), g.topology.vertex_point, g.topology.prim_offsets
    v = 0.0
    for p in range(g.nprims):
        q = P[vp[offs[p]:offs[p + 1]]]
        for k in range(1, len(q) - 1):
            v += np.dot(q[0], np.cross(q[k], q[k + 1])) / 6.0
    return v


class SkinTests(unittest.TestCase):
    def test_tube_from_rings_with_different_point_counts(self):
        g = rings([(1, 0), (1, 1), (1, 2)], [32, 40, 24])
        out = run("skin", g, samples=48, cap=True)
        self.assertEqual(out.npoints, 3 * 48)
        self.assertEqual(out.nprims, 2 * 48 + 2)                               # two bands of quads plus two caps
        self.assertTrue(np.all(twins(out) >= 0))                               # watertight
        self.assertGreater(volume(out), 0)                                     # facing outwards
        self.assertAlmostEqual(volume(out), 2 * np.pi, delta=0.1)   # r=1, height 2 (the 24-gon ring is a little smaller than the circle)
        self.assertEqual(out.point["uv"].shape, (144, 2))

    def test_default_samples_and_loop(self):
        g = rings([(1, 0), (1.5, 0.2), (1, 0.4)], [10, 16, 12])
        out = run("skin", g, loop=True)
        self.assertEqual(out.npoints, 3 * 16)
        self.assertEqual(out.nprims, 3 * 16)                                   # a torus-like closed surface
        self.assertTrue(np.all(twins(out) >= 0))

    def test_open_curves_make_a_sheet(self):
        a = np.linspace(0, 1, 5)
        rows = [np.c_[a * 4, np.full(5, y), np.sin(a * 3) * y] for y in (0.0, 1.0, 2.0)]
        P = np.concatenate(rows).astype(np.float32)
        g = Geometry.from_arrays(P, np.arange(15, dtype=np.int32), np.array([0, 5, 10, 15], np.int32),
                                 closed=np.zeros(3, bool))
        out = run("skin", g)
        self.assertEqual((out.npoints, out.nprims), (15, 8))
        self.assertAlmostEqual(float(out.P[:, 1].max()), 2.0)
        self.assertEqual(float(out.point["uv"][:, 0].max()), 1.0)

    def test_bad_input(self):
        with self.assertRaises(Exception):
            run("skin", rings([(1, 0)], [8]))                                   # one curve
        mixed = rings([(1, 0), (1, 1)], [8, 8])
        with self.assertRaises(Exception):
            run("skin", Geometry.from_arrays(mixed.P, mixed.topology.vertex_point, mixed.topology.prim_offsets,
                                             closed=np.array([True, False])))


if __name__ == "__main__":
    unittest.main()
