"""Geometry for the 3-D preview: arrays and the wire format."""
import unittest

import numpy as np

from minihou import Geometry, preview
from tests.test_nodes import run


def decode(geo):
    head, arrays = preview.arrays_for(geo)
    return preview.unpack(preview.pack(head, arrays))


class PreviewArraysTests(unittest.TestCase):
    def test_mesh_triangles_edges_bounds_and_round_trip(self):
        sphere = run("sphere", rows=10, columns=12)
        head, a = decode(sphere)
        self.assertEqual((head["points"], head["prims"]), (sphere.npoints, sphere.nprims))
        self.assertEqual(a["P"].shape, (sphere.npoints * 3,))
        tri = a["tri"].reshape(-1, 3)
        self.assertEqual(len(tri), head["tris"])
        self.assertTrue(tri.max() < sphere.npoints)
        P = a["P"].reshape(-1, 3)
        area = 0.5 * np.linalg.norm(np.cross(P[tri[:, 1]] - P[tri[:, 0]], P[tri[:, 2]] - P[tri[:, 0]]), axis=1).sum()
        self.assertAlmostEqual(area, 4 * np.pi, delta=0.6)                                # a sphere's surface area
        self.assertEqual(len(a["edge"].reshape(-1, 2)), head["edges"])
        self.assertEqual(len(a["tri_prim"]), head["tris"])                         # which primitive each triangle belongs to
        self.assertTrue(a["tri_prim"].max() < sphere.nprims)
        np.testing.assert_allclose(head["bounds"], [[-1, -1, -1], [1, 1, 1]], atol=1e-6)
        self.assertFalse(head["truncated"])
        self.assertNotIn("Cd", a)

    def test_quads_and_concave_polygons_are_triangulated(self):
        box = run("box", size=[1, 1, 1])
        self.assertEqual(decode(box)[0]["tris"], 12)
        self.assertEqual(sorted(set(decode(box)[1]["tri_prim"].tolist())), [0, 1, 2, 3, 4, 5])
        mixed = run("merge", [run("grid", columns=3, rows=3), run("box")])             # triangles, quads and prims in order
        a = decode(mixed)[1]
        counts = np.bincount(a["tri_prim"], minlength=mixed.nprims)
        self.assertTrue(np.all(counts == 2))
        L = np.array([[0, 0, 0], [2, 0, 0], [2, 1, 0], [1, 1, 0], [1, 2, 0], [0, 2, 0]], np.float32)
        lshape = Geometry.from_arrays(L, np.arange(6, dtype=np.int32), np.array([0, 6], np.int32))
        head, a = decode(lshape)
        self.assertEqual(head["tris"], 4)
        self.assertEqual(a["tri_prim"].tolist(), [0, 0, 0, 0])
        P, tri = a["P"].reshape(-1, 3), a["tri"].reshape(-1, 3)
        area = 0.5 * np.abs(np.cross(P[tri[:, 1]] - P[tri[:, 0]], P[tri[:, 2]] - P[tri[:, 0]])[:, 2]).sum()
        self.assertAlmostEqual(area, 3.0)                                                 # not the convex hull's 3.5

    def test_points_curves_colours_and_empty(self):
        pts = Geometry.points_only(np.random.default_rng(0).random((50, 3)).astype(np.float32)).copy()
        pts.point["Cd"] = np.random.default_rng(1).random((50, 3)).astype(np.float32) * 2
        head, a = decode(pts)
        self.assertEqual((head["tris"], head["edges"], head["points"]), (0, 0, 50))
        self.assertTrue(a["Cd"].shape == (150,) and a["Cd"].max() <= 1.0)                  # clipped to 0..1
        curve = run("helix", points=30)
        head, a = decode(curve)
        self.assertEqual((head["tris"], head["edges"]), (0, 29))
        head, a = decode(Geometry.points_only(np.zeros((0, 3), np.float32)))
        self.assertEqual((head["points"], head["tris"]), (0, 0))
        self.assertEqual(head["bounds"], [[0.0, 0.0, 0.0], [0.0, 0.0, 0.0]])

    def test_big_geometry_is_reduced_not_cut_off(self):
        """Over the budget a mesh is clustered: the whole shape stays (bounds, area) and the header says it was reduced."""
        grid = run("grid", columns=60, rows=60, size_x=10.0, size_y=10.0)
        head, a = preview.unpack(preview.pack(*preview.arrays_for(grid, tris=1000)))
        self.assertTrue(head["proxy"]["active"])
        self.assertLessEqual(head["tris"], 1000)
        self.assertGreater(head["tris"], 300)
        self.assertEqual((head["points"], head["prims"], head["proxy"]["tris"]), (3600, 59 * 59, 2 * 59 * 59))
        P = a["P"].reshape(-1, 3)
        np.testing.assert_allclose(P.min(0)[:2], [-5, -5], atol=0.8)                   # the far corners are still there
        np.testing.assert_allclose(P.max(0)[:2], [5, 5], atol=0.8)
        np.testing.assert_allclose(head["bounds"], [[-5, -5, 0], [5, 5, 0]], atol=1e-5)  # the real bounds
        tri = a["tri"].reshape(-1, 3)
        area = 0.5 * np.linalg.norm(np.cross(P[tri[:, 1]] - P[tri[:, 0]], P[tri[:, 2]] - P[tri[:, 0]]), axis=1).sum()
        self.assertAlmostEqual(area, 100.0, delta=25.0)                                 # still covers most of the 10x10 surface
        self.assertEqual(len(a["point_src"]), len(P))                                   # what each drawn point stands for
        self.assertLess(a["point_src"].max(), 3600)
        self.assertLess(a["tri_prim"].max(), grid.nprims)
        small = preview.arrays_for(run("sphere", rows=10, columns=12))[0]
        self.assertFalse(small["proxy"]["active"])                                      # within budget: untouched
        self.assertNotIn("point_src", preview.arrays_for(run("sphere", rows=10, columns=12))[1])

    def test_full_resolution_raises_the_budget(self):
        grid = run("grid", columns=80, rows=80)
        old = preview.DISPLAY_TRIS
        preview.DISPLAY_TRIS = 2000
        try:
            reduced = preview.arrays_for(grid)[0]
            full = preview.arrays_for(grid, full=True)[0]
        finally:
            preview.DISPLAY_TRIS = old
        self.assertTrue(reduced["proxy"]["active"])
        self.assertFalse(full["proxy"]["active"])
        self.assertEqual(full["tris"], 2 * 79 * 79)

    def test_loose_points_and_curves_are_sampled(self):
        old = preview.DISPLAY_POINTS
        preview.DISPLAY_POINTS = 100
        try:
            head, a = decode(Geometry.points_only(np.random.default_rng(2).random((1000, 3)).astype(np.float32)))
            self.assertEqual((head["points"], len(a["P"]) // 3, head["proxy"]["active"]), (1000, 100, True))
            head, a = decode(run("helix", points=400))
            self.assertTrue(head["proxy"]["active"])
            self.assertLessEqual(len(a["P"]) // 3, 400)
        finally:
            preview.DISPLAY_POINTS = old

    def test_geometry_over_the_hard_caps_is_flagged_truncated(self):
        old = preview.MAX_EDGES
        preview.MAX_EDGES = 80
        try:
            head, _ = decode(run("grid", columns=30, rows=30))
        finally:
            preview.MAX_EDGES = old
        self.assertEqual((head["edges"], head["truncated"]), (80, True))

    def test_repeated_requests_for_one_geometry_are_remembered(self):
        grid = run("grid", columns=40, rows=40).freeze()
        h1, a1 = preview.arrays_for(grid)
        h2, a2 = preview.arrays_for(grid)
        self.assertIs(a1["P"], a2["P"])
        h1["node"] = "x"                                                                 # callers may add header fields
        self.assertNotIn("node", preview.arrays_for(grid)[0])

    def test_unpack_rejects_other_data(self):
        with self.assertRaises(ValueError):
            preview.unpack(b'{"ok": false}')


if __name__ == "__main__":
    unittest.main()
