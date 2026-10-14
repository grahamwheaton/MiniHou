"""Regression coverage for colour, normal and proximity group helpers."""
import unittest

import numpy as np

from minihou import Geometry, edges, topo
from minihou.nodes import group_helpers  # noqa: F401 - register these nodes before their package import is wired
from tests.test_nodes import run


def triangle():
    geo = Geometry.from_arrays(np.array([[0, 0, 0], [1, 0, 0], [0, 1, 0]], np.float32),
                               [0, 1, 2], [0, 3])
    geo.point["Cd"] = np.array([[1, 0, 0], [1, 0, 0], [0, 0, 1]], np.float32)
    return geo


def selected(geo, domain, name="group1"):
    if domain == "edges":
        return edges.values(geo, "edge_group_" + name).astype(bool)
    table = geo.table({"points": "point", "primitives": "prim", "vertices": "vertex"}[domain])
    return table["group_" + name].astype(bool)


class GroupByColorTests(unittest.TestCase):
    def test_rgb_matching_and_point_colour_fallback_on_every_domain(self):
        geo = triangle()
        expected = {
            "points": [True, True, False],
            "vertices": [True, True, False],
            "primitives": [True],
            "edges": [True, False, False],  # lexicographic edge (0,1) alone is red
        }
        for domain in expected:
            with self.subTest(domain=domain):
                color = (2 / 3, 0, 1 / 3) if domain == "primitives" else (1, 0, 0)
                out = run("group_by_color", geo, domain=domain, name="picked", color=color,
                          tolerance=0.001)
                np.testing.assert_array_equal(selected(out, domain, "picked"), expected[domain])
                self.assertIs(out.topology, geo.topology)
                self.assertIs(out.point["P"], geo.point["P"])

    def test_native_attribute_tolerance_base_inversion_and_merge(self):
        geo = triangle()
        geo.vertex["Cd"] = np.array([[0.98, 0, 0], [1, 0, 0], [0, 0, 1]], np.float32)
        close = run("group_by_color", geo, domain="vertices", tolerance=0.03,
                    color=(1, 0, 0), name="picked")
        np.testing.assert_array_equal(selected(close, "vertices", "picked"), [True, True, False])
        far = run("group_by_color", geo, domain="vertices", tolerance=0.01,
                  color=(1, 0, 0), name="picked")
        np.testing.assert_array_equal(selected(far, "vertices", "picked"), [False, True, False])
        base = run("group_by_color", geo, domain="points", name="base", color=(1, 0, 0), tolerance=0)
        out = run("group_by_color", base, domain="points", name="picked", base_group="base",
                  color=(0, 0, 1), invert=True)
        np.testing.assert_array_equal(selected(out, "points", "picked"), [True, True, False])
        union = run("group_by_color", far, domain="vertices", name="picked", color=(0, 0, 1),
                    tolerance=0, combine="union")
        np.testing.assert_array_equal(selected(union, "vertices", "picked"), [False, True, True])
        np.testing.assert_allclose(geo.vertex["Cd"], [[.98, 0, 0], [1, 0, 0], [0, 0, 1]])

    def test_missing_and_malformed_colour_are_explicit_and_empty_is_safe(self):
        geo = Geometry.points_only(np.array([[0, 0, 0]], np.float32))
        with self.assertRaisesRegex(ValueError, "colour attribute 'Cd'"):
            run("group_by_color", geo)
        geo = geo.copy()
        geo.point["Cd"] = np.array([1], np.int32)
        with self.assertRaisesRegex(ValueError, "three numeric components"):
            run("group_by_color", geo)
        empty = Geometry.points_only(np.zeros((0, 3), np.float32))
        for domain in ("points", "primitives", "edges", "vertices"):
            out = run("group_by_color", empty, domain=domain, invert=True)
            self.assertEqual(len(selected(out, domain)), 0)


class GroupByNormalTests(unittest.TestCase):
    def test_computed_normals_on_all_domains_and_both_sides(self):
        P = np.array([[0, 0, 0], [1, 0, 0], [0, 1, 0],
                      [3, 0, 0], [3, 1, 0], [4, 0, 0]], np.float32)
        geo = Geometry.from_arrays(P, list(range(6)), [0, 3, 6])
        for domain in ("points", "primitives", "edges", "vertices"):
            with self.subTest(domain=domain):
                out = run("group_by_normal", geo, domain=domain, direction=(0, 0, 10), angle=0)
                self.assertEqual(int(selected(out, domain).sum()),
                                 {"points": 3, "primitives": 1, "edges": 3, "vertices": 3}[domain])
                both = run("group_by_normal", geo, domain=domain, direction=(0, 0, 2),
                           angle=0, both_sides=True)
                self.assertEqual(int(selected(both, domain).sum()), len(selected(both, domain)))

    def test_existing_n_is_normalized_and_zero_normals_do_not_match(self):
        geo = triangle()
        geo.point["N"] = np.array([[0, 0, 4], [0, 0, -20], [0, 0, 0]], np.float32)
        out = run("group_by_normal", geo, domain="points", angle=10)
        np.testing.assert_array_equal(selected(out, "points"), [True, False, False])
        both = run("group_by_normal", geo, domain="points", angle=10, both_sides=True)
        np.testing.assert_array_equal(selected(both, "points"), [True, True, False])
        wide = run("group_by_normal", geo, domain="points", angle=180)
        np.testing.assert_array_equal(selected(wide, "points"), [True, True, False])
        with self.assertRaisesRegex(ValueError, "nonzero"):
            run("group_by_normal", geo, direction=(0, 0, 0))

    def test_supplied_primitive_and_vertex_normals_override_geometry(self):
        geo = triangle()
        geo.prim["N"] = np.array([[0, 0, -25]], np.float32)
        geo.vertex["N"] = np.array([[0, 0, 100], [0, 0, -3], [0, 0, 0]], np.float32)
        face = run("group_by_normal", geo, domain="primitives", direction=(0, 0, -1), angle=0)
        np.testing.assert_array_equal(selected(face, "primitives"), [True])
        corners = run("group_by_normal", geo, domain="vertices", angle=0, both_sides=True)
        np.testing.assert_array_equal(selected(corners, "vertices"), [True, True, False])

    def test_open_degenerate_and_nonmanifold_topology(self):
        open_curve = Geometry.from_arrays(np.array([[0, 0, 0], [1, 0, 0], [0, 1, 0]], np.float32),
                                          [0, 1, 2], [0, 3], closed=[False])
        degenerate = Geometry.from_arrays(np.array([[0, 0, 0], [1, 0, 0], [2, 0, 0]], np.float32),
                                          [0, 1, 2], [0, 3])
        for geo in (open_curve, degenerate):
            for domain in ("points", "primitives", "edges", "vertices"):
                out = run("group_by_normal", geo, domain=domain, angle=180)
                self.assertFalse(selected(out, domain).any(), (domain, geo.topology.closed))
        # Three incident faces are legal; their unit normals (+Z, -Z, +Z)
        # average to +Z along the shared, nonmanifold edge.
        P = np.array([[0, 0, 0], [1, 0, 0], [0, 1, 0], [0, 2, 0], [0, 3, 0]], np.float32)
        geo = Geometry.from_arrays(P, [0, 1, 2, 1, 0, 3, 0, 1, 4], [0, 3, 6, 9])
        edge_id = np.flatnonzero(np.all(topo.edges(geo).points == [0, 1], axis=1))[0]
        result = run("group_by_normal", geo, domain="edges", angle=0)
        self.assertTrue(selected(result, "edges")[edge_id])

    def test_edge_supplied_normals_are_unitized_before_averaging(self):
        P = np.array([[0, 0, 0], [1, 0, 0], [0, 1, 0], [0, -1, 0]], np.float32)
        geo = Geometry.from_arrays(P, [0, 1, 2, 1, 0, 3], [0, 3, 6])
        N = np.tile([0, 0, 1], (6, 1)).astype(np.float32)
        N[0] *= 100
        N[3] = [0, 0, -1]
        geo.vertex["N"] = N
        edge_id = np.flatnonzero(np.all(topo.edges(geo).points == [0, 1], axis=1))[0]
        out = run("group_by_normal", geo, domain="edges", angle=180)
        self.assertFalse(selected(out, "edges")[edge_id])  # opposite incident N cancel exactly

    def test_base_group_inversion_and_subtract_merge(self):
        geo = triangle()
        geo.point["N"] = np.array([[0, 0, 1], [0, 0, -1], [0, 0, 1]], np.float32)
        base = run("group_by_color", geo, name="base", color=(1, 0, 0), tolerance=0)
        old = run("group_by_normal", base, name="picked", angle=180)
        out = run("group_by_normal", old, name="picked", base_group="base", angle=0,
                  invert=True, combine="subtract")
        np.testing.assert_array_equal(selected(out, "points", "picked"), [True, False, True])


class GroupByProximityTests(unittest.TestCase):
    def test_curve_hierarchy_matches_brute_force_with_long_segments(self):
        rng = np.random.default_rng(51)
        a = rng.normal(size=(81, 3))
        b = a + rng.normal(size=(81, 3)) * rng.uniform(.01, 8, size=(81, 1))
        a[0], b[0] = [-100, 0, 0], [100, 0, 0]
        query = rng.normal(size=(101, 3))
        query[0] = [0, .025, 0]
        ab = b - a
        t = np.clip(np.einsum("qsi,si->qs", query[:, None] - a[None], ab) /
                    np.maximum(np.einsum("si,si->s", ab, ab), 1e-300), 0, 1)
        foot = a[None] + t[:, :, None] * ab[None]
        nearest = np.linalg.norm(query[:, None] - foot, axis=2).min(axis=1)
        for radius in (0, .02, .025, .1, 1.0):
            np.testing.assert_array_equal(group_helpers._segment_hits(query, a, b, radius), nearest <= radius + 1e-12)

    def test_position_uses_representatives_of_all_domains(self):
        geo = triangle()
        cases = {"points": [True, False, False],
                 "vertices": [True, False, False],
                 "primitives": [False],
                 "edges": [False, False, False]}
        for domain, expected in cases.items():
            out = run("group_by_proximity", geo, domain=domain, target_mode="position",
                      position=(0, 0, 0), distance=0.1)
            np.testing.assert_array_equal(selected(out, domain), expected)
        edge = run("group_by_proximity", geo, domain="edges", target_mode="position",
                   position=(0.5, 0, 0), distance=0)
        self.assertEqual(int(selected(edge, "edges").sum()), 1)

    def test_points_and_actual_surface_are_distinct(self):
        target = Geometry.from_arrays(np.array([[0, 0, 0], [2, 0, 0], [2, 2, 0], [0, 2, 0]], np.float32),
                                      [0, 1, 2, 3], [0, 4])
        query = Geometry.points_only(np.array([[1, 1, .2], [4, 4, .2]], np.float32))
        points = run("group_by_proximity", query, target, target_mode="points", distance=.3)
        surface = run("group_by_proximity", query, target, target_mode="surface", distance=.3)
        auto = run("group_by_proximity", query, target, distance=.3)
        np.testing.assert_array_equal(selected(points, "points"), [False, False])
        np.testing.assert_array_equal(selected(surface, "points"), [True, False])
        np.testing.assert_array_equal(selected(auto, "points"), [True, False])
        below = run("group_by_proximity", query, target, target_mode="surface", distance=.199)
        boundary = run("group_by_proximity", query, target, target_mode="surface", distance=.2)
        np.testing.assert_array_equal(selected(below, "points"), [False, False])
        np.testing.assert_array_equal(selected(boundary, "points"), [True, False])

    def test_concave_polygon_notch_is_outside_actual_surface(self):
        P = np.array([[0, 0, 0], [3, 0, 0], [3, 1, 0], [1, 1, 0],
                      [1, 2, 0], [3, 2, 0], [3, 3, 0], [0, 3, 0]], np.float32)
        target = Geometry.from_arrays(P, list(range(8)), [0, 8])
        query = Geometry.points_only(np.array([[.5, 1.5, .1], [2, 1.5, .1]], np.float32))
        out = run("group_by_proximity", query, target, target_mode="surface", distance=.2)
        np.testing.assert_array_equal(selected(out, "points"), [True, False])
        auto = run("group_by_proximity", query, target, distance=.2)
        np.testing.assert_array_equal(selected(auto, "points"), [True, False])

    def test_degenerate_face_is_not_a_surface(self):
        target = Geometry.from_arrays(np.array([[0, 0, 0], [1, 0, 0], [2, 0, 0]], np.float32),
                                      [0, 1, 2], [0, 3])
        query = Geometry.points_only(np.array([[.5, 0, 0]], np.float32))
        with self.assertRaisesRegex(ValueError, "no polygon faces or curve segments"):
            run("group_by_proximity", query, target, target_mode="surface", distance=0)
        # Auto mode uses target points when a connected face has no area.
        out = run("group_by_proximity", query, target, distance=.1)
        np.testing.assert_array_equal(selected(out, "points"), [False])

    def test_exact_u_notch_and_reversed_winding(self):
        P = np.array([[0,0,0],[3,0,0],[3,3,0],[2,3,0],
                      [2,1,0],[1,1,0],[1,3,0],[0,3,0]], np.float32)
        query = Geometry.points_only(np.array([[1.5,2,0],[.5,2,0]], np.float32))
        for order in (list(range(8)), list(reversed(range(8)))):
            target = Geometry.from_arrays(P, order, [0,8])
            for mode in ('surface', 'auto'):
                out = run('group_by_proximity', query, target, target_mode=mode, distance=.1)
                np.testing.assert_array_equal(selected(out,'points'), [False,True])
            triangles = group_helpers._surface_triangles(target)
            self.assertAlmostEqual(float(np.linalg.norm(np.cross(triangles[:,1]-triangles[:,0],triangles[:,2]-triangles[:,0]),axis=1).sum()/2),7)

    def test_bridged_hole_preserves_empty_hole_and_surface_area(self):
        # Outer square with a clockwise hole, joined by a repeated diagonal bridge.
        P = np.array([[0,0,0],[4,0,0],[4,4,0],[0,4,0],
                      [1,1,0],[1,3,0],[3,3,0],[3,1,0]], np.float32)
        order = [0,1,2,3,0,4,5,6,7,4]
        query = Geometry.points_only(np.array([[2,2,0],[.5,2,0],[.5,.5,0]],np.float32))
        for winding in (order, list(reversed(order))):
            target = Geometry.from_arrays(P,winding,[0,len(winding)])
            for mode in ('surface','auto'):
                out = run('group_by_proximity',query,target,target_mode=mode,distance=.1)
                np.testing.assert_array_equal(selected(out,'points'),[False,True,True])
            triangles = group_helpers._surface_triangles(target)
            area = np.linalg.norm(np.cross(triangles[:,1]-triangles[:,0],triangles[:,2]-triangles[:,0]),axis=1).sum()/2
            self.assertAlmostEqual(float(area),12)

    def test_repeated_degenerate_face_is_not_filled(self):
        target = Geometry.from_arrays(np.array([[0,0,0],[1,0,0],[2,0,0]],np.float32),
                                      [0,1,2,1,0],[0,5])
        query = Geometry.points_only(np.array([[.5,0,0]],np.float32))
        with self.assertRaisesRegex(ValueError,'no polygon faces or curve segments'):
            run('group_by_proximity',query,target,target_mode='surface',distance=.1)
        np.testing.assert_array_equal(selected(run('group_by_proximity',query,target,distance=.1),'points'),[False])

    def test_open_curve_segments_and_missing_target(self):
        target = Geometry.from_arrays(np.array([[0, 0, 0], [2, 0, 0]], np.float32),
                                      [0, 1], [0, 2], closed=[False])
        query = Geometry.points_only(np.array([[1, .1, 0]], np.float32))
        surface = run("group_by_proximity", query, target, target_mode="surface", distance=.2)
        points = run("group_by_proximity", query, target, target_mode="points", distance=.2)
        self.assertTrue(selected(surface, "points")[0])
        self.assertFalse(selected(points, "points")[0])
        with self.assertRaisesRegex(ValueError, "requires a connected target"):
            run("group_by_proximity", query, target_mode="surface")

    def test_empty_geometry_and_empty_target_are_safe(self):
        empty = Geometry.points_only(np.zeros((0, 3), np.float32))
        query = Geometry.points_only(np.array([[0, 0, 0]], np.float32))
        for domain in ("points", "primitives", "edges", "vertices"):
            out = run("group_by_proximity", empty, domain=domain, invert=True)
            self.assertEqual(len(selected(out, domain)), 0)
        out = run("group_by_proximity", query, empty, target_mode="points", distance=1)
        np.testing.assert_array_equal(selected(out, "points"), [False])
        # A connected target with points but no usable surface falls back to points in auto mode.
        degenerate = Geometry.from_arrays(np.array([[0, 0, 0]], np.float32), [0], [0, 1])
        near = run("group_by_proximity", query, degenerate, distance=0)
        np.testing.assert_array_equal(selected(near, "points"), [True])
        with self.assertRaisesRegex(ValueError, "no polygon faces or curve segments"):
            run("group_by_proximity", query, degenerate, target_mode="surface")

    def test_inversion_merge_and_frozen_input_preserve_attributes(self):
        geo = triangle()
        geo.point["weight"] = np.array([2, 3, 4], np.float32)
        base = run("group_by_color", geo, name="base", color=(1, 0, 0), tolerance=0)
        old = run("group_by_proximity", base, name="picked", target_mode="position",
                  position=(0, 0, 0), distance=0)
        result = run("group_by_proximity", old, name="picked", base_group="base",
                     target_mode="position", position=(0, 0, 0), distance=0,
                     invert=True, combine="union")
        np.testing.assert_array_equal(selected(result, "points", "picked"), [True, True, False])
        self.assertTrue(geo.frozen)
        self.assertIs(result.topology, geo.topology)
        self.assertIs(result.point["P"], geo.point["P"])
        self.assertIs(result.point["weight"], geo.point["weight"])
        np.testing.assert_array_equal(geo.point["weight"], [2, 3, 4])
        self.assertNotIn("group_picked", geo.point)


if __name__ == "__main__":
    unittest.main()
