"""revolve: profiles spun around an axis."""
import unittest

import numpy as np

from minihou import Geometry
from minihou.nodes.bevel import twins
from tests.test_nodes import run


def curve(P, closed=False):
    P = np.asarray(P, np.float32)
    return Geometry.from_arrays(P, np.arange(len(P), dtype=np.int32), np.array([0, len(P)], np.int32),
                                closed=None if closed else np.zeros(1, bool))


def volume(g):
    P, vp, offs = g.P.astype(float), g.topology.vertex_point, g.topology.prim_offsets
    v = 0.0
    for p in range(g.nprims):
        q = P[vp[offs[p]:offs[p + 1]]]
        for k in range(1, len(q) - 1):
            v += np.dot(q[0], np.cross(q[k], q[k + 1])) / 6.0
    return v


def area(g):
    P, vp, offs = g.P.astype(float), g.topology.vertex_point, g.topology.prim_offsets
    a = 0.0
    for p in range(g.nprims):
        q = P[vp[offs[p]:offs[p + 1]]]
        a += 0.5 * np.linalg.norm(np.cross(q[1:-1] - q[0], q[2:] - q[0]), axis=1).sum()
    return a


def semicircle(n=25):
    t = np.linspace(-np.pi / 2, np.pi / 2, n)
    return curve(np.stack([np.cos(t), np.zeros_like(t), np.sin(t)], 1))


class RevolveTests(unittest.TestCase):
    def test_a_semicircle_makes_a_closed_sphere_with_triangles_at_the_poles(self):
        s = run("revolve", semicircle(), divisions=48)
        self.assertEqual(s.npoints, 23 * 48 + 2)                                   # the two poles are single points
        self.assertEqual(sorted(set(s.topology.prim_sizes().tolist())), [3, 4])
        self.assertEqual(int((s.topology.prim_sizes() == 3).sum()), 2 * 48)
        self.assertTrue(np.all(twins(s) >= 0))
        self.assertAlmostEqual(volume(s), 4 / 3 * np.pi, delta=0.04)               # outwards and close to a sphere's volume
        self.assertAlmostEqual(area(s), 4 * np.pi, delta=0.12)

    def test_cylinder_torus_and_other_axes(self):
        wall = curve([[1, 0, 0], [1, 0, 2]])
        cyl = run("revolve", wall, divisions=64)
        self.assertAlmostEqual(area(cyl), 2 * np.pi * 1 * 2, delta=0.05)
        self.assertEqual(int((twins(cyl) < 0).sum()), 2 * 64)                      # two open rims
        a = np.linspace(0, 2 * np.pi, 24, endpoint=False)
        ring = curve(np.stack([2 + 0.5 * np.cos(a), np.zeros_like(a), 0.5 * np.sin(a)], 1), closed=True)
        tor = run("revolve", ring, divisions=48)
        self.assertTrue(np.all(twins(tor) >= 0))
        self.assertAlmostEqual(volume(tor), 2 * np.pi ** 2 * 2 * 0.25, delta=0.25)
        can = run("revolve", curve([[1, -1, 0], [1, 1, 0]]), axis=[0, 1, 0], divisions=32)
        self.assertAlmostEqual(area(can), 2 * np.pi * 2, delta=0.1)
        np.testing.assert_allclose(can.P[:, 1].min(), -1, atol=1e-6)

    def test_partial_and_negative_angles(self):
        half = run("revolve", semicircle(13), angle=180, divisions=12)
        self.assertEqual(half.nprims, 12 * 12)                                      # 12 profile segments, 12 steps
        self.assertTrue(np.any(twins(half) < 0))                                    # an open surface
        pos, neg = run("revolve", semicircle(13), angle=360, divisions=24), run("revolve", semicircle(13), angle=-360, divisions=24)
        self.assertAlmostEqual(volume(pos), volume(neg))                            # the faces keep facing outwards
        self.assertTrue(np.all(twins(neg) >= 0))
        quarter = run("revolve", curve([[1, 0, 0], [1, 0, 1]]), angle=90, divisions=9)
        self.assertEqual(quarter.npoints, 20)
        np.testing.assert_allclose(np.ptp(quarter.P[:, 0]), 1.0, atol=1e-5)

    def test_attributes_and_uvs(self):
        prof = curve([[1, 0, 0], [2, 0, 1], [1, 0, 2]]).copy()
        prof.point["w"] = np.array([1, 2, 3], np.float32)
        prof.prim["id"] = np.array([7], np.int32)
        prof.point["N"] = np.array([[1, 0, 0], [1, 0, 0], [1, 0, 0]], np.float32)
        out = run("revolve", prof, divisions=8)
        self.assertEqual(out.npoints, 3 * 8)
        self.assertTrue(np.all(out.prim["id"] == 7))
        self.assertEqual(sorted(set(out.point["w"].tolist())), [1.0, 2.0, 3.0])
        np.testing.assert_allclose(np.linalg.norm(out.point["N"], axis=1), 1.0, atol=1e-5)
        np.testing.assert_allclose(out.point["N"][:, :2] * out.P[:, :2] > -1e-6, True)         # normals turned with the points
        uv = out.vertex["uv"]
        self.assertEqual(uv.shape, (out.topology.nvertices, 2))
        self.assertAlmostEqual(float(uv[:, 0].min()), 0.0)
        self.assertAlmostEqual(float(uv[:, 0].max()), 1.0)                                      # a full turn reaches u = 1
        self.assertAlmostEqual(float(uv[:, 1].min()), 0.0)
        self.assertAlmostEqual(float(uv[:, 1].max()), 1.0)

    def test_degenerate_input(self):
        with self.assertRaises(Exception):
            run("revolve", semicircle(), axis=[0, 0, 0])
        empty = run("revolve", Geometry.points_only(np.zeros((3, 3), np.float32)))
        self.assertEqual((empty.npoints, empty.nprims), (0, 0))
        on_axis = run("revolve", curve([[0, 0, 0], [0, 0, 1]]), divisions=8)                  # nothing leaves the axis
        self.assertEqual(on_axis.nprims, 0)


if __name__ == "__main__":
    unittest.main()
