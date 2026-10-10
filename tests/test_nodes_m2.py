import time
import unittest

import numpy as np

from minihou import Geometry
from minihou.wrangle import WrangleError
from tests.helpers import quad_geo
from tests.test_nodes import run


def cube():
    P = np.array([[x, y, z] for z in (0, 1) for y in (0, 1) for x in (0, 1)], np.float32) - 0.5
    faces = [[0, 2, 3, 1], [4, 5, 7, 6], [0, 1, 5, 4], [2, 6, 7, 3], [0, 4, 6, 2], [1, 3, 7, 5]]   # outward CCW
    vp = [i for f in faces for i in f]
    return Geometry.from_arrays(P, vp, np.arange(7) * 4)


class NormalTests(unittest.TestCase):
    def test_flat_grid_and_flipped_winding(self):
        g = run("grid", columns=4, rows=3)
        np.testing.assert_allclose(run("normal", g).point["N"], [[0, 0, 1]] * 12, atol=1e-6)
        flipped = Geometry.from_arrays(quad_geo().P, [0, 3, 2, 1], [0, 4])
        np.testing.assert_allclose(run("normal", flipped).point["N"], [[0, 0, -1]] * 4, atol=1e-6)

    def test_tilted_plane(self):
        g = run("transform", run("grid", columns=3, rows=3), rotate=(90, 0, 0))   # XY plane -> XZ plane
        np.testing.assert_allclose(run("normal", g).point["N"], [[0, -1, 0]] * 9, atol=1e-6)

    def test_closed_cube_points_outward(self):
        c = cube()
        N = run("normal", c).point["N"]
        np.testing.assert_allclose(N, c.P / np.linalg.norm(c.P, axis=1, keepdims=True), atol=1e-6)
        fN = run("normal", c, domain="prim").prim["N"]
        np.testing.assert_allclose(np.abs(fN).sum(1), 1, atol=1e-6)          # axis aligned unit normals

    def test_ngon_degenerate_and_unused_points(self):
        P = np.array([[0, 0, 0], [2, 0, 0], [3, 1, 0], [1, 2, 0], [-1, 1, 0],     # pentagon
                      [5, 5, 5], [5, 5, 5], [5, 5, 5],                           # degenerate triangle
                      [9, 9, 9]], np.float32)                                    # unused point
        g = Geometry.from_arrays(P, [0, 1, 2, 3, 4, 5, 6, 7], [0, 5, 8])
        N = run("normal", g).point["N"]
        np.testing.assert_allclose(N[:5], [[0, 0, 1]] * 5, atol=1e-6)
        np.testing.assert_array_equal(N[5:], 0)
        self.assertFalse(np.isnan(N).any())

    def test_area_weighting(self):
        # a point shared by a big +Z quad and a tiny +X quad leans to +Z
        P = np.array([[0, 0, 0], [10, 0, 0], [10, 10, 0], [0, 10, 0], [0, 0, 0.1], [0, 0.1, 0.1], [0, 0.1, 0]], np.float32)
        g = Geometry.from_arrays(P, [0, 1, 2, 3, 0, 6, 5, 4], [0, 4, 8])
        n0 = run("normal", g).point["N"][0]
        self.assertGreater(n0[2], 0.99)

    def test_feeds_copy_orientation(self):
        sphere_ish = run("noise", run("grid", columns=20, rows=20), amplitude=(0, 0, 2), frequency=0.3)
        pts = run("normal", sphere_ish)
        out = run("copy_to_points", Geometry.points_only(np.array([[0, 0, 1]], np.float32)), pts)
        np.testing.assert_allclose(out.P - pts.P, pts.point["N"], atol=1e-5)    # +Z of the source follows N

    def test_speed_one_million_points(self):
        g = run("grid", columns=1000, rows=1000)
        t = time.perf_counter()
        run("normal", g)
        self.assertLess(time.perf_counter() - t, 3.0)


class DeleteTests(unittest.TestCase):
    def grid_with_attrs(self):
        g = run("grid", columns=4, rows=4).copy()
        g.point["id"] = np.arange(16, dtype=np.int32)
        g.vertex["vid"] = np.arange(g.nvertices, dtype=np.int32)
        g.prim["pid"] = np.arange(g.nprims, dtype=np.int32)
        return g.freeze()

    def test_points_and_dependent_prims_are_removed_and_compacted(self):
        g = self.grid_with_attrs()
        d = run("delete", g, expr="@P.x > 0")
        d.validate()
        self.assertEqual((d.npoints, d.nprims, d.nvertices), (8, 3, 12))
        self.assertTrue((g.P[d.point["id"], 0] <= 0).all())
        np.testing.assert_array_equal(d.prim["pid"], [0, 3, 6])                # left column of quads
        np.testing.assert_array_equal(d.vertex["vid"], np.concatenate([np.arange(0, 4), np.arange(12, 16), np.arange(24, 28)]))
        # every surviving quad still references the same original points
        for new, old in zip(d.topology.polys(), [g.topology.polys()[i] for i in d.prim["pid"]]):
            self.assertEqual(d.point["id"][new].tolist(), old)

    def test_invert_noop_and_everything(self):
        g = self.grid_with_attrs()
        kept = run("delete", g, expr="@P.x > 0", invert=True)
        self.assertTrue((g.P[kept.point["id"], 0] > 0).all())
        self.assertIs(run("delete", g, expr="@P.x > 1000"), g)                 # nothing matched: same object
        self.assertIs(run("delete", g, expr=""), g)
        none = run("delete", g, expr="1")
        self.assertEqual((none.npoints, none.nprims), (0, 0))
        none.validate()

    def test_works_on_point_clouds_and_attribute_expressions(self):
        s = run("scatter", run("grid"), count=500, seed=1)
        d = run("delete", s, expr="rand(@ptnum, 1) < 0.5")
        self.assertTrue(150 < d.npoints < 350)
        self.assertEqual(d.point["N"].shape, (d.npoints, 3))

    def test_expression_is_sandboxed_and_single(self):
        g = self.grid_with_attrs()
        for bad in ("__import__('os')", "1; @P.z = 5", "1\n@P.z = 5", "@nope > 0"):
            with self.subTest(bad=bad):
                with self.assertRaises(WrangleError):
                    run("delete", g, expr=bad)


if __name__ == "__main__":
    unittest.main()
