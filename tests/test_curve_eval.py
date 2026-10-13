"""Curve sampling and geometry parts for the Blender add-on (bpy-free; the same maths is checked against Blender itself in
tests/test_blender_geometry_exchange.py)."""
import unittest

import numpy as np

from tests.addon_loader import pure_module

ce = pure_module("curve_eval")
gp = pure_module("geometry_parts")
proto = pure_module("protocol")


def line(n, step=1.0):
    return np.array([(i * step, 0, 0) for i in range(n)], np.float32)


class SamplingTests(unittest.TestCase):
    def test_poly_curves_are_their_points_and_cyclic_ones_closed(self):
        P = np.concatenate([line(4), line(3) + [0, 5, 0]])
        S = ce.evaluate(P, np.array([0, 4, 7]), cyclic=np.array([False, True]))
        np.testing.assert_array_equal(S.counts, [4, 3])
        np.testing.assert_array_equal(S.closed, [False, True])
        np.testing.assert_allclose(S.P, P)

    def test_bezier_has_resolution_points_per_segment_and_ends_on_the_control_points(self):
        P = line(3, 2.0)
        S = ce.evaluate(P, np.array([0, 3]), curve_type=np.array([ce.BEZIER]), resolution=np.array([5]), handle_left=P - [0.5, 0, 0],
                        handle_right=P + [0.5, 0, 0])
        self.assertEqual(len(S.P), 2 * 5 + 1)
        np.testing.assert_allclose(S.P[0], P[0])
        np.testing.assert_allclose(S.P[-1], P[-1], atol=1e-6)
        np.testing.assert_allclose(S.P[5], P[1], atol=1e-6)                          # the middle control point is on the curve
        self.assertTrue(np.all(np.diff(S.P[:, 0]) > 0))                              # straight handles keep it a monotone line

    def test_cyclic_bezier_does_not_repeat_its_first_point(self):
        P = np.array([(0, 0, 0), (1, 0, 0), (1, 1, 0), (0, 1, 0)], np.float32)
        S = ce.evaluate(P, np.array([0, 4]), curve_type=np.array([ce.BEZIER]), cyclic=np.array([True]), resolution=np.array([3]),
                        handle_left=P, handle_right=P)
        self.assertEqual(len(S.P), 4 * 3)
        self.assertTrue(S.closed[0])

    def test_nurbs_endpoint_clamped_touches_both_ends_and_counts_like_blender(self):
        P = np.array([(0, 0, 0), (1, 2, 0), (2, -2, 0), (3, 2, 0), (4, -2, 0), (5, 0, 0)], np.float32)
        S = ce.evaluate(P, np.array([0, 6]), curve_type=np.array([ce.NURBS]), resolution=np.array([6]), nurbs_order=np.array([4]),
                        knots_mode=np.array([ce.KNOT_ENDPOINT]))
        self.assertEqual(len(S.P), 6 * (6 - 4 + 1) + 1)                               # 19, as Blender gives
        np.testing.assert_allclose(S.P[0], P[0], atol=1e-6)
        np.testing.assert_allclose(S.P[-1], P[-1], atol=1e-6)

    def test_a_heavier_nurbs_weight_pulls_the_curve_towards_that_point(self):
        P = np.array([(0, 0, 0), (1, 3, 0), (2, 0, 0), (3, 0, 0), (4, 0, 0)], np.float32)
        kw = dict(curve_type=np.array([ce.NURBS]), resolution=np.array([8]), nurbs_order=np.array([3]), knots_mode=np.array([ce.KNOT_ENDPOINT]))
        flat = ce.evaluate(P, np.array([0, 5]), **kw).P[:, 1].max()
        heavy = ce.evaluate(P, np.array([0, 5]), nurbs_weight=np.array([1, 5, 1, 1, 1], np.float32), **kw).P[:, 1].max()
        self.assertGreater(heavy, flat)

    def test_catmull_rom_passes_through_its_control_points(self):
        P = np.array([(0, 0, 0), (1, 1, 0), (2, 0, 1), (3, 1, 0)], np.float32)
        S = ce.evaluate(P, np.array([0, 4]), curve_type=np.array([ce.CATMULL_ROM]), resolution=np.array([4]))
        self.assertEqual(len(S.P), 3 * 4 + 1)
        for k in range(4):
            np.testing.assert_allclose(S.P[min(k * 4, len(S.P) - 1)], P[k], atol=1e-6)

    def test_attributes_ride_along(self):
        P = line(3, 2.0)
        S = ce.evaluate(P, np.array([0, 3]), curve_type=np.array([ce.BEZIER]), resolution=np.array([4]), handle_left=P, handle_right=P)
        r = S.blend(np.array([1.0, 2.0, 3.0], np.float32))
        np.testing.assert_allclose(r, np.linspace(1, 3, 9), atol=1e-6)                 # linear between control points
        np.testing.assert_array_equal(S.take(np.array([7])), np.full(9, 7))
        v = S.blend(np.array([[1, 0], [0, 1], [1, 1]], np.float32))
        self.assertEqual(v.shape, (9, 2))

    def test_mixed_curve_types_and_empty_curves_in_one_object(self):
        P = np.concatenate([line(3), line(4) + [0, 1, 0], line(2) + [0, 2, 0]])
        offs = np.array([0, 3, 3, 7, 9])                                               # the second curve has no points
        S = ce.evaluate(P, offs, curve_type=np.array([ce.POLY, ce.POLY, ce.CATMULL_ROM, ce.POLY]),
                        resolution=np.array([1, 1, 3, 1]))
        np.testing.assert_array_equal(S.counts, [3, 0, 3 * 3 + 1, 2])
        self.assertEqual(len(S.curve_of_point), len(S.P))

    def test_control_polygon_ignores_the_curve_type(self):
        P = line(4)
        S = ce.control_polygon(P, np.array([0, 4]), np.array([False]))
        np.testing.assert_allclose(S.P, P)


class PartsTests(unittest.TestCase):
    def test_curve_arrays_describe_open_and_closed_primitives(self):
        P = np.concatenate([line(4), line(3) + [0, 2, 0], line(1) + [0, 4, 0]])
        arrays, info = gp.curve_arrays(P, np.array([0, 4, 7, 8]), cyclic=np.array([False, True, False]),
                                       point_attrs={"radius": np.arange(8, dtype=np.float32)},
                                       curve_attrs={"kind": np.array([1, 2, 3], np.int32)})
        gp.check(arrays)
        np.testing.assert_array_equal(arrays["topo/prim_offsets"], [0, 4, 7])               # the one-point curve is a point, not a primitive
        np.testing.assert_array_equal(arrays["topo/prim_closed"], [0, 1])
        self.assertEqual(len(arrays["point/P"]), 8)
        np.testing.assert_array_equal(arrays["prim/kind"], [1, 2])
        self.assertEqual(info["curves"], 3)

    def test_a_cyclic_curve_of_two_points_stays_open(self):
        arrays, _ = gp.curve_arrays(line(2), np.array([0, 2]), cyclic=np.array([True]))
        np.testing.assert_array_equal(arrays["topo/prim_closed"], [0])

    def test_integer_point_attributes_stay_integers(self):
        P = line(3, 2.0)
        arrays, _ = gp.curve_arrays(P, np.array([0, 3]), curve_type=np.array([ce.BEZIER]), resolution=np.array([4]), handle_left=P,
                                    handle_right=P, point_attrs={"id": np.array([10, 20, 30], np.int32)})
        self.assertEqual(arrays["point/id"].dtype.kind, "i")
        ids = arrays["point/id"]
        self.assertTrue({10, 20, 30} <= set(ids.tolist()))                                    # the control points keep their values
        self.assertTrue(np.all(np.diff(ids) >= 0) and ids.min() == 10 and ids.max() == 30)

    def test_a_point_cloud_is_points_without_primitives(self):
        arrays = gp.point_arrays(line(5), {"radius": np.ones(5, np.float32)})
        gp.check(arrays)
        self.assertEqual(len(arrays["topo/prim_offsets"]), 1)

    def test_combine_keeps_the_parts_apart_and_zero_fills_missing_attributes(self):
        quad = {"point/P": np.array([[0, 0, 0], [1, 0, 0], [1, 1, 0], [0, 1, 0]], np.float32), "topo/vertex_point": np.arange(4, dtype=np.int32),
                "topo/prim_offsets": np.array([0, 4], np.int32), "prim/mat": np.array([3], np.int32),
                "vertex/uv": np.zeros((4, 2), np.float32)}
        curves, _ = gp.curve_arrays(line(3) + [5, 0, 0], np.array([0, 3]), point_attrs={"radius": np.array([1, 2, 3], np.float32)})
        pts = gp.point_arrays(line(2) + [9, 0, 0], {"radius": np.array([5, 6], np.float32)})
        out = gp.combine([quad, curves, pts])
        gp.check(out)
        self.assertEqual(len(out["point/P"]), 4 + 3 + 2)
        np.testing.assert_array_equal(out["topo/prim_offsets"], [0, 4, 7])
        np.testing.assert_array_equal(out["topo/vertex_point"], [0, 1, 2, 3, 4, 5, 6])
        np.testing.assert_array_equal(out["topo/prim_closed"], [1, 0])                          # quad is a polygon, curve open
        np.testing.assert_array_equal(out["point/radius"], [0, 0, 0, 0, 1, 2, 3, 5, 6])
        np.testing.assert_array_equal(out["prim/mat"], [3, 0])
        self.assertEqual(out["vertex/uv"].shape, (7, 2))

    def test_combine_of_one_part_is_that_part(self):
        a = gp.point_arrays(line(2))
        self.assertIs(gp.combine([a]), a)

    def test_check_catches_inconsistent_arrays(self):
        bad = gp.point_arrays(line(2))
        bad["point/x"] = np.zeros(5, np.float32)
        with self.assertRaises(ValueError):
            gp.check(bad)


if __name__ == "__main__":
    unittest.main()
