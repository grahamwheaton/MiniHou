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
        np.testing.assert_allclose(head["bounds"], [[-1, -1, -1], [1, 1, 1]], atol=1e-6)
        self.assertFalse(head["truncated"])
        self.assertNotIn("Cd", a)

    def test_quads_and_concave_polygons_are_triangulated(self):
        box = run("box", size=[1, 1, 1])
        self.assertEqual(decode(box)[0]["tris"], 12)
        L = np.array([[0, 0, 0], [2, 0, 0], [2, 1, 0], [1, 1, 0], [1, 2, 0], [0, 2, 0]], np.float32)
        lshape = Geometry.from_arrays(L, np.arange(6, dtype=np.int32), np.array([0, 6], np.int32))
        head, a = decode(lshape)
        self.assertEqual(head["tris"], 4)
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

    def test_big_geometry_is_cut_and_flagged(self):
        old = preview.MAX_TRIS, preview.MAX_EDGES
        preview.MAX_TRIS, preview.MAX_EDGES = 50, 80
        try:
            head, a = decode(run("grid", columns=30, rows=30))
        finally:
            preview.MAX_TRIS, preview.MAX_EDGES = old
        self.assertEqual((head["tris"], head["edges"], head["truncated"]), (50, 80, True))
        self.assertLess(a["tri"].max(), 900)
        oldp = preview.MAX_POINTS
        preview.MAX_POINTS = 100
        try:
            head, a = decode(Geometry.points_only(np.random.default_rng(2).random((1000, 3)).astype(np.float32)))
        finally:
            preview.MAX_POINTS = oldp
        self.assertEqual((head["points"], len(a["P"]) // 3, head["truncated"]), (1000, 100, True))

    def test_unpack_rejects_other_data(self):
        with self.assertRaises(ValueError):
            preview.unpack(b'{"ok": false}')


if __name__ == "__main__":
    unittest.main()
