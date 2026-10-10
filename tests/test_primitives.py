import unittest
from collections import Counter

import numpy as np

from minihou import Geometry
from tests.test_nodes import run


def directed_edges(g):
    out = []
    for poly in g.topology.polys():
        out += [(poly[i], poly[(i + 1) % len(poly)]) for i in range(len(poly))]
    return out


class ClosedSurfaceMixin:
    def assert_watertight_and_consistent(self, g):
        de = directed_edges(g)
        counts = Counter(de)
        self.assertTrue(all(v == 1 for v in counts.values()), "a directed edge repeats: inconsistent winding")
        for a, b in de:
            self.assertIn((b, a), counts, "an edge has no opposite: the surface is open")

    def assert_outward(self, g, center=(0, 0, 0)):
        N = run("normal", g).point["N"]
        d = g.P - np.asarray(center, np.float32)
        self.assertTrue((np.einsum("ij,ij->i", N, d) > 0).all(), "normals must point away from the centre")


class BoxTests(unittest.TestCase, ClosedSurfaceMixin):
    def test_unit_box(self):
        b = run("box")
        self.assertEqual((b.npoints, b.nprims), (8, 6))
        np.testing.assert_allclose(np.abs(b.P), 0.5)
        self.assert_watertight_and_consistent(b)
        self.assert_outward(b)

    def test_divisions_size_center(self):
        b = run("box", divisions=4, size=(2, 3, 4), center=(1, 2, 3))
        self.assertEqual(b.nprims, 6 * 16)
        self.assertEqual(b.npoints, 6 * 4 * 4 + 2)                    # 6n^2 + 2 for a subdivided cube surface
        np.testing.assert_allclose(b.P.min(0), [0, 0.5, 1], atol=1e-6)
        np.testing.assert_allclose(b.P.max(0), [2, 3.5, 5], atol=1e-6)
        self.assert_watertight_and_consistent(b)
        self.assert_outward(b, (1, 2, 3))


class SphereTests(unittest.TestCase, ClosedSurfaceMixin):
    def test_counts_radius_and_closure(self):
        s = run("sphere", radius=2, rows=6, columns=10, center=(1, 0, 0))
        self.assertEqual(s.npoints, 2 + 5 * 10)
        self.assertEqual(s.nprims, 10 + 4 * 10 + 10)
        np.testing.assert_allclose(np.linalg.norm(s.P - [1, 0, 0], axis=1), 2, atol=1e-5)
        self.assert_watertight_and_consistent(s)
        self.assert_outward(s, (1, 0, 0))

    def test_minimum_resolution(self):
        s = run("sphere", rows=2, columns=3)                            # two triangle fans, no quad band
        self.assertEqual((s.npoints, s.nprims), (5, 6))
        self.assert_watertight_and_consistent(s)
        self.assert_outward(s)


class CircleAndFuseTests(unittest.TestCase):
    def test_circle(self):
        c = run("circle", divisions=6, radius=2, rotate=(90, 0, 0))
        self.assertEqual((c.npoints, c.nprims), (6, 1))
        np.testing.assert_allclose(np.linalg.norm(c.P, axis=1), 2, atol=1e-6)
        np.testing.assert_allclose(run("normal", c).point["N"], [[0, -1, 0]] * 6, atol=1e-6)

    def test_fuse_merges_duplicates_and_keeps_order_and_attributes(self):
        a, b = run("grid", columns=3, rows=3), run("grid", columns=3, rows=3, center=(10, 0, 0))   # shares the x=5 column
        m = run("merge", [a, b]).copy()
        m.point["id"] = np.arange(m.npoints, dtype=np.int32)
        f = run("fuse", m.freeze(), distance=1e-4)
        self.assertEqual(f.npoints, 18 - 3)                             # the shared column of 3 points
        self.assertEqual(f.nprims, 8)
        f.validate()
        self.assertTrue((np.diff(f.point["id"]) > 0).all())             # survivors keep original order
        for poly_new, poly_old in zip(f.topology.polys(), m.topology.polys()):
            np.testing.assert_allclose(f.P[poly_new], m.P[poly_old], atol=1e-6)

    def test_fuse_noop_and_empty(self):
        g = run("grid")
        self.assertEqual(run("fuse", g, distance=1e-6).npoints, g.npoints)
        e = Geometry.points_only(np.zeros((0, 3), np.float32)).freeze()
        self.assertEqual(run("fuse", e).npoints, 0)

    def test_scatter_and_copy_on_a_sphere(self):
        s = run("normal", run("sphere", rows=12, columns=24))
        pts = run("scatter", s, count=200, seed=1)
        np.testing.assert_allclose(np.linalg.norm(pts.P, axis=1), 1, atol=0.05)
        spikes = run("copy_to_points", run("box", size=(0.05, 0.05, 0.2)), pts)
        self.assertEqual(spikes.nprims, 200 * 6)


if __name__ == "__main__":
    unittest.main()
