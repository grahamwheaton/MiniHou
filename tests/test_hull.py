"""3-D convex hull: closed outward-facing triangles that hold every input point, same volume from both backends."""
import unittest

import numpy as np

from minihou import Geometry, hull as hl
from minihou.nodes.bevel import twins
from tests.test_nodes import run

BACKENDS = [True, False] if hl._ConvexHull is not None else [False]


def volume(P, F):
    return np.einsum("ij,ij->", P[F[:, 0]], np.cross(P[F[:, 1]], P[F[:, 2]])) / 6.0


class HullTests(unittest.TestCase):
    def check(self, P, F):
        n = np.cross(P[F[:, 1]] - P[F[:, 0]], P[F[:, 2]] - P[F[:, 0]])
        n /= np.linalg.norm(n, axis=1, keepdims=True)
        d = np.einsum("ij,ij->i", n, P[F[:, 0]])
        self.assertLessEqual(float((P @ n.T - d).max()), 1e-9 * max(np.ptp(P), 1))          # nothing outside any face
        self.assertGreater(volume(P - P.mean(0), F), 0)                                       # facing outwards

    def test_cube_with_interior_and_duplicate_points(self):
        rng = np.random.default_rng(0)
        corners = np.array([[x, y, z] for x in (0, 1) for y in (0, 1) for z in (0, 1)], float)
        P = np.concatenate([corners, rng.random((300, 3)) * 0.9 + 0.05, corners[:3]])
        for us in BACKENDS:
            F = hl.convex_hull(P, use_scipy=us)
            self.check(P, F)
            self.assertAlmostEqual(volume(P, F), 1.0, places=9)
            self.assertEqual(set(F.reshape(-1).tolist()) <= set(range(8)) | {8 + 300, 9 + 300, 10 + 300}, True)
            self.assertEqual(len(F), 12)

    def test_random_clouds_and_sphere_points(self):
        rng = np.random.default_rng(1)
        for n in (4, 5, 30, 1000):
            P = rng.normal(size=(n, 3)) * [3, 1, 0.5]
            vols = []
            for us in BACKENDS:
                F = hl.convex_hull(P, use_scipy=us)
                self.check(P, F)
                vols.append(volume(P, F))
            self.assertAlmostEqual(vols[0], vols[-1], places=8)
        S = rng.normal(size=(500, 3))
        S /= np.linalg.norm(S, axis=1, keepdims=True)
        for us in BACKENDS:
            F = hl.convex_hull(S, use_scipy=us)
            self.check(S, F)
            self.assertEqual(len(F), 2 * len(np.unique(F)) - 4)                                  # Euler: V - E + F = 2

    def test_degenerate_input(self):
        rng = np.random.default_rng(2)
        flat = np.c_[rng.random((50, 2)), np.zeros(50)]
        line = np.c_[np.arange(6.0), np.arange(6.0), np.arange(6.0)]
        for us in BACKENDS:
            for P in (np.zeros((0, 3)), np.ones((3, 3)), np.ones((9, 3)), flat, line):
                self.assertEqual(hl.convex_hull(P, use_scipy=us).shape, (0, 3))

    def test_cocircular_and_coplanar_faces(self):
        g = np.array([[x, y, z] for x in range(3) for y in range(3) for z in range(3)], float)   # many coplanar points
        for us in BACKENDS:
            F = hl.convex_hull(g, use_scipy=us)
            self.check(g, F)
            self.assertAlmostEqual(volume(g, F), 8.0, places=9)


class HullNodeTests(unittest.TestCase):
    def test_node_makes_a_closed_mesh_and_keeps_corner_attributes(self):
        rng = np.random.default_rng(3)
        P = np.concatenate([np.array([[x, y, z] for x in (0, 1) for y in (0, 1) for z in (0, 1)], float),
                            rng.random((100, 3)) * 0.8 + 0.1]).astype(np.float32)
        geo = Geometry.points_only(P)
        geo = geo.copy()
        geo.point["id"] = np.arange(len(P), dtype=np.int32)
        out = run("convex_hull", geo)
        self.assertEqual((out.npoints, out.nprims), (8, 12))
        self.assertTrue(np.all(twins(out) >= 0))                                                 # closed
        self.assertCountEqual(out.point["id"].tolist(), list(range(8)))
        self.assertAlmostEqual(float(volume(out.P.astype(float), out.topology.vertex_point.reshape(-1, 3))), 1.0, 5)

    def test_flat_points_come_back_unchanged(self):
        P = np.c_[np.random.default_rng(4).random((10, 2)), np.zeros(10)].astype(np.float32)
        out = run("convex_hull", Geometry.points_only(P))
        self.assertEqual((out.npoints, out.nprims), (10, 0))


if __name__ == "__main__":
    unittest.main()
