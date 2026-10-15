"""Regression tests for nodes given a target without triangles."""
import unittest

import numpy as np

from minihou import Geometry
from minihou.nodes.deform import point_relax
from minihou.nodes.ray import ray


class TestEmptyTargetNodes(unittest.TestCase):
    def test_ray_no_triangles_matches_a_triangle_miss(self):
        points = Geometry.points_only(np.array([[0.25, 0.25, 1.0], [0.5, 0.25, 2.0]], np.float32))
        triangle = Geometry.from_arrays(
            np.array([[0, 0, 0], [1, 0, 0], [0, 1, 0]], np.float32),
            vertex_point=np.array([0, 1, 2], np.int32),
            prim_offsets=np.array([0, 3], np.int32),
        )
        single_point = Geometry.points_only(np.zeros((1, 3), np.float32))
        coincident_curve = Geometry.from_arrays(
            np.zeros((2, 3), np.float32),
            vertex_point=np.array([0, 1], np.int32),
            prim_offsets=np.array([0, 2], np.int32),
            closed=np.array([False], bool),
        )
        for target in (triangle, single_point, coincident_curve):
            target.point["Cd"] = np.ones((target.npoints, 3), np.float32)
            target.freeze()
        points.freeze()

        for method in ("direction", "minimum_distance"):
            def cast(target):
                return ray(points, target, method=method, direction=(0, 0, 1), both_ways=False,
                           max_distance=0.5, scale=1.0, lift=0.0, transform_points=True,
                           import_attribs="Cd", delete_misses=False, group="")

            expected = cast(triangle)
            for name, target in (("single_point", single_point), ("coincident_curve", coincident_curve)):
                with self.subTest(method=method, target=name):
                    actual = cast(target)
                    actual.validate()
                    self.assertEqual(set(actual.point.names()), set(expected.point.names()))
                    self.assertEqual(set(actual.vertex.names()), set(expected.vertex.names()))
                    self.assertEqual(set(actual.prim.names()), set(expected.prim.names()))
                    for attr in expected.point.names():
                        np.testing.assert_array_equal(actual.point[attr], expected.point[attr])
                    np.testing.assert_array_equal(actual.P, points.P)
                    np.testing.assert_array_equal(actual.point["hitdist"], [-1.0, -1.0])
                    np.testing.assert_array_equal(actual.point["hitprim"], [-1, -1])
                    np.testing.assert_array_equal(actual.point["group_rayHit"], [0, 0])

    def test_point_relax_open_curve_keeps_points(self):
        points = Geometry.points_only(np.array([[0, 0, 1], [0.01, 0, 1], [0.02, 0, 1]], np.float32))
        surface = Geometry.from_arrays(
            np.array([[0, 0, 0], [1, 0, 0]], np.float32),
            vertex_point=np.array([0, 1], np.int32),
            prim_offsets=np.array([0, 2], np.int32),
            closed=np.array([False], bool),
        )
        points.freeze()
        surface.freeze()

        out = point_relax(points, surface, iterations=3, radius=0.5, update_normals=True)
        self.assertIs(out, points)
        np.testing.assert_array_equal(out.P, points.P)
        out.validate()


if __name__ == "__main__":
    unittest.main()
