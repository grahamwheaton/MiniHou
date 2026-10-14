"""Regression tests for extended group operations and their mesh subsets."""
import unittest

import numpy as np

from minihou import Geometry
from minihou import topo
from minihou.nodes import group_extended  # noqa: F401 - registers nodes
from minihou.nodes import group_rules as rules
from tests.test_nodes import run


def mesh(P, faces, closed=None):
    return Geometry.from_arrays(np.asarray(P, np.float32), np.asarray(sum(faces, []), np.int32),
                                np.asarray([0] + list(np.cumsum([len(x) for x in faces])), np.int32), closed)


def ids(geo, domain, name):
    return np.flatnonzero(rules.membership(geo, domain, name)).tolist()


class GroupCopyTests(unittest.TestCase):
    def test_index_copy_all_domains_conflict_and_empty(self):
        src = mesh([[0, 0, 0], [1, 0, 0], [1, 1, 0], [0, 1, 0]], [[0, 1, 2, 3]])
        src = rules.write(src, "point", "p", [True, False, True, False])
        src = rules.write(src, "prim", "f", [True])
        src = rules.write(src, "vertex", "v", [False, True, False, False])
        src = rules.write(src, "edge", "e", [True, False, False, False])
        src = rules.write(src, "point", "empty", [False] * 4)
        dst = mesh([[2, 0, 0], [3, 0, 0], [3, 1, 0], [2, 1, 0]], [[0, 1, 2, 3]])
        dst = rules.write(dst, "point", "p", [False, True, False, False])
        out = run("group_copy", dst, src, conflict="suffix")
        self.assertEqual(ids(out, "point", "p"), [1])
        self.assertEqual(ids(out, "point", "p2"), [0, 2])
        self.assertEqual(ids(out, "prim", "f"), [0])
        self.assertEqual(ids(out, "vertex", "v"), [1])
        self.assertEqual(ids(out, "edge", "e"), [0])
        self.assertIn("empty", rules.names(out, "point"))
        self.assertNotIn("empty", rules.names(run("group_copy", dst, src, copy_empty=False), "point"))
        self.assertEqual(ids(dst, "point", "p"), [1])
        self.assertNotIn("p2", rules.names(dst, "point"))
        skipped = run("group_copy", dst, src, conflict="skip")
        self.assertEqual(ids(skipped, "point", "p"), [1])
        self.assertNotIn("p2", rules.names(skipped, "point"))
        overwritten = run("group_copy", dst, src, conflict="overwrite")
        self.assertEqual(ids(overwritten, "point", "p"), [0, 2])
        self.assertIs(overwritten.topology, dst.topology)
        self.assertIs(overwritten.point["P"], dst.point["P"])
        self.assertEqual(ids(src, "point", "p"), [0, 2])

    def test_match_by_integer_attribute_first_duplicate_and_unmatched(self):
        src = Geometry.points_only(np.zeros((3, 3), np.float32))
        src.point["id"] = np.array([7, 7, 9], np.int32)
        src = rules.write(src, "point", "a", [True, False, True])
        dst = Geometry.points_only(np.zeros((4, 3), np.float32))
        dst.point["id"] = np.array([9, 7, 6, 7], np.int32)
        out = run("group_copy", dst, src, point_match_attribute="id")
        self.assertEqual(ids(out, "point", "a"), [0, 1, 3])
        with self.assertRaisesRegex(ValueError, "must exist"):
            run("group_copy", dst, src, point_match_attribute="missing")

    def test_primitive_and_vertex_match_by_id_with_suffix_search(self):
        src = mesh([[0, 0, 0], [1, 0, 0], [2, 0, 0]], [[0, 1], [1, 2]], [False, False])
        src.prim["id"] = np.array([10, 20], np.int32)
        src.vertex["id"] = np.array([7, 8, 9, 10], np.int32)
        src = rules.write(src, "prim", "f", [True, False])
        src = rules.write(src, "vertex", "v", [True, False, True, False])
        dst = mesh([[0, 0, 0], [1, 0, 0], [2, 0, 0]], [[0, 1], [1, 2]], [False, False])
        dst.prim["id"] = np.array([20, 10], np.int32)
        dst.vertex["id"] = np.array([10, 9, 8, 7], np.int32)
        dst = rules.write(dst, "prim", "f", [True, True])
        dst = rules.write(dst, "prim", "f2", [False, True])
        out = run("group_copy", dst, src, prim_match_attribute="id", vertex_match_attribute="id",
                  conflict="suffix")
        self.assertEqual(ids(out, "prim", "f3"), [1])
        self.assertEqual(ids(out, "vertex", "v"), [1, 3])
        self.assertEqual(ids(dst, "prim", "f"), [0, 1])


class GroupTransferTests(unittest.TestCase):
    def test_nearest_threshold_and_empty_conflict(self):
        src = Geometry.points_only(np.array([[0, 0, 0], [10, 0, 0]], np.float32))
        src = rules.write(src, "point", "near", [True, False])
        dst = Geometry.points_only(np.array([[0.1, 0, 0], [9.9, 0, 0], [30, 0, 0]], np.float32))
        dst = rules.write(dst, "point", "near", [False, True, True])
        out = run("group_transfer", dst, src, distance_threshold=0.25, conflict="suffix")
        self.assertEqual(ids(out, "point", "near"), [1, 2])
        self.assertEqual(ids(out, "point", "near2"), [0])
        out = run("group_transfer", dst, src, distance_threshold=0.05, create_empty=False)
        self.assertEqual(ids(out, "point", "near"), [1, 2])

    def test_closest_unselected_source_blocks_distant_selected_source(self):
        src = Geometry.points_only(np.array([[0, 0, 0], [2, 0, 0]], np.float32))
        src = rules.write(src, "point", "chosen", [True, False])
        dst = Geometry.points_only(np.array([[1.9, 0, 0], [0.25, 0, 0]], np.float32))
        out = run("group_transfer", dst, src, distance_threshold=2.0)
        self.assertEqual(ids(out, "point", "chosen"), [1])
        exact = run("group_transfer", dst, src, distance_threshold=0.25)
        self.assertEqual(ids(exact, "point", "chosen"), [1])
        too_far = run("group_transfer", dst, src, distance_threshold=0.24)
        self.assertEqual(ids(too_far, "point", "chosen"), [])
        self.assertIs(out.topology, dst.topology)
        self.assertIs(out.point["P"], dst.point["P"])
        self.assertEqual(ids(src, "point", "chosen"), [0])

    def test_empty_source_preserves_destination_and_optional_empty_group(self):
        src = Geometry.points_only(np.empty((0, 3), np.float32))
        src = rules.write(src, "point", "present", np.zeros(0, bool))
        dst = Geometry.points_only(np.array([[0, 0, 0]], np.float32))
        out = run("group_transfer", dst, src)
        self.assertEqual(ids(out, "point", "present"), [])
        omitted = run("group_transfer", dst, src, create_empty=False)
        self.assertNotIn("present", rules.names(omitted, "point"))
        self.assertIs(omitted, dst)

    def test_primitive_and_edge_centroid_transfer(self):
        src = mesh([[0, 0, 0], [1, 0, 0], [1, 1, 0], [0, 1, 0]], [[0, 1, 2, 3]])
        src = rules.write(src, "prim", "face", [True])
        src = rules.write(src, "edge", "side", [True, False, False, False])
        dst = mesh([[0.1, 0, 0], [1.1, 0, 0], [1.1, 1, 0], [0.1, 1, 0]], [[0, 1, 2, 3]])
        out = run("group_transfer", dst, src, distance_threshold=0.2)
        self.assertEqual(ids(out, "prim", "face"), [0])
        self.assertEqual(ids(out, "edge", "side"), [0])


class GroupFindPathTests(unittest.TestCase):
    def test_ordered_pairs_collision_and_disconnected(self):
        g = mesh([[0, 0, 0], [1, 0, 0], [2, 0, 0], [3, 0, 0], [10, 0, 0]],
                 [[0, 1, 2, 3]], [False])
        out = run("group_find_path", g, base_group="0 2 3")
        self.assertEqual(ids(out, "point", "path"), [0, 1, 2, 3])
        out = run("group_find_path", g, base_group="0 2", path_type="pairs")
        self.assertEqual(ids(out, "point", "path"), [0, 1, 2])
        with self.assertRaisesRegex(ValueError, "even number"):
            run("group_find_path", g, base_group="0 2 3", path_type="pairs")
        with self.assertRaisesRegex(ValueError, "no path"):
            run("group_find_path", g, base_group="0 4")
        with self.assertRaisesRegex(ValueError, "no path"):
            run("group_find_path", g, base_group="0 3", collision_group="1")

    def test_edge_path_open_curve_has_no_closing_edge(self):
        g = mesh([[0, 0, 0], [1, 0, 0], [2, 0, 0], [3, 0, 0]], [[0, 1, 2, 3]], [False])
        self.assertEqual(len(topo.edges(g).points), 3)
        out = run("group_find_path", g, domain="edges", base_group="0 2")
        self.assertEqual(ids(out, "edge", "path"), [0, 1, 2])


class AttributeBoundaryTests(unittest.TestCase):
    def test_primitive_seam_open_border_and_base_limit(self):
        g = mesh([[0, 0, 0], [1, 0, 0], [2, 0, 0], [0, 1, 0], [1, 1, 0], [2, 1, 0]],
                 [[0, 1, 4, 3], [1, 2, 5, 4]])
        g.prim["piece"] = np.array([0, 1], np.int32)
        seam = run("group_from_attribute_boundary", g, attribute="piece", include_unshared_edges=False)
        shared = np.flatnonzero(topo.edges(g).count == 2).tolist()
        self.assertEqual(ids(seam, "edge", "boundary"), shared)
        all_edges = run("group_from_attribute_boundary", g, attribute="piece")
        self.assertEqual(len(ids(all_edges, "edge", "boundary")), len(topo.edges(g).points))
        boundary_faces = run("group_from_attribute_boundary", g, attribute="piece", domain="primitives",
                             include_unshared_edges=False)
        self.assertEqual(ids(boundary_faces, "prim", "boundary"), [0, 1])
        boundary_vertices = run("group_from_attribute_boundary", g, attribute="piece", domain="vertices",
                                include_unshared_edges=False)
        self.assertEqual(ids(boundary_vertices, "vertex", "boundary"), [1, 2, 4, 7])
        g = rules.write(g, "prim", "left", [True, False])
        only_left = run("group_from_attribute_boundary", g, base_group="left", include_unshared_edges=False)
        self.assertEqual(ids(only_left, "edge", "boundary"), shared)
        self.assertEqual(ids(g, "prim", "left"), [0])

    def test_point_and_vertex_discontinuity_and_nonmanifold(self):
        g = mesh([[0, 0, 0], [1, 0, 0], [0, 1, 0], [1, 1, 0], [0.5, -1, 0]],
                 [[0, 1, 2], [1, 0, 3], [0, 1, 4]])
        g.point["region"] = np.array([0, 1, 0, 0, 0], np.int32)
        out = run("group_from_attribute_boundary", g, attribute="region", attribute_domain="points",
                  include_unshared_edges=False)
        e = topo.edges(g).points
        expected = np.flatnonzero(np.any(e == 1, axis=1)).tolist()
        self.assertEqual(ids(out, "edge", "boundary"), expected)
        g = g.copy()
        g.vertex["uvpiece"] = np.array([0, 0, 0, 1, 0, 0, 0, 0, 0], np.int32)
        out = run("group_from_attribute_boundary", g, attribute="uvpiece", attribute_domain="vertices",
                  include_unshared_edges=False)
        self.assertIn(int(np.flatnonzero((e == [0, 1]).all(axis=1))[0]), ids(out, "edge", "boundary"))

    def test_nonmanifold_primitive_attribute_checks_every_incident_pair(self):
        g = mesh([[0, 0, 0], [1, 0, 0], [0, 1, 0], [1, 1, 0], [0.5, -1, 0]],
                 [[0, 1, 2], [1, 0, 3], [0, 1, 4]])
        g.prim["weight"] = np.array([0.0, 0.0005, 0.01], np.float32)
        shared = int(np.flatnonzero(topo.edges(g).count == 3)[0])
        out = run("group_from_attribute_boundary", g, attribute="weight", tolerance=0.001,
                  include_unshared_edges=False)
        self.assertEqual(ids(out, "edge", "boundary"), [shared])
        out = run("group_from_attribute_boundary", g, attribute="weight", tolerance=0.02,
                  include_unshared_edges=False)
        self.assertEqual(ids(out, "edge", "boundary"), [])
        self.assertIs(out.topology, g.topology)

    def test_curve_edges_respect_separate_unshared_option(self):
        g = mesh([[0, 0, 0], [1, 0, 0], [2, 0, 0]], [[0, 1, 2]], [False])
        default = run("group_from_attribute_boundary", g)
        self.assertEqual(ids(default, "edge", "boundary"), [])
        enabled = run("group_from_attribute_boundary", g, include_all_curve_edges=True)
        self.assertEqual(ids(enabled, "edge", "boundary"), [0, 1])


class GroupsFromNameTests(unittest.TestCase):
    def test_integer_name_table_empty_invalid_collision_and_existing_group(self):
        g = Geometry.points_only(np.zeros((6, 3), np.float32))
        g.point["piece"] = np.array([0, 1, 2, 3, 4, -1], np.int32)
        g.detail["piece_names"] = ["oak", "", "1bad", "a-b", "a b"]
        g = rules.write(g, "point", "oak", [False, True, False, False, False, False])
        ignored = run("groups_from_name", g, attribute="piece", domain="points")
        self.assertEqual(ids(ignored, "point", "oak"), [0])
        self.assertNotIn("1bad", rules.names(ignored, "point"))
        fixed = run("groups_from_name", g, attribute="piece", domain="points", invalid_names="force_valid",
                    conflict="union", prefix="pre_")
        self.assertEqual(ids(fixed, "point", "pre_oak"), [0])
        self.assertEqual(ids(fixed, "point", "pre_1bad"), [2])
        self.assertEqual(ids(fixed, "point", "pre_a_b"), [3, 4])
        union = run("groups_from_name", g, attribute="piece", domain="points", conflict="union")
        self.assertEqual(ids(union, "point", "oak"), [0, 1])
        self.assertEqual(ids(g, "point", "oak"), [1])
        with self.assertRaisesRegex(ValueError, "piece_names"):
            h = g.copy()
            h.detail.pop("piece_names")
            run("groups_from_name", h, attribute="piece", domain="points")


class GroupRangeExtensionTests(unittest.TestCase):
    def test_base_range_modes_inversion_and_disconnected_regions(self):
        g = mesh([[i, 0, 0] for i in range(6)], [[0, 1, 2], [3, 4, 5]], [False, False])
        g = rules.write(g, "point", "eligible", [True, True, True, False, True, True])
        out = run("group_range", g, domain="points", base_group="eligible", range_type="length",
                  start=1, length=2)
        self.assertEqual(ids(out, "point", "group1"), [1, 2])
        out = run("group_range", g, domain="points", base_group="eligible", range_type="relative",
                  start=1, end=1)
        self.assertEqual(ids(out, "point", "group1"), [1, 2, 4])
        out = run("group_range", g, domain="points", base_group="eligible", range_type="equal_partitions",
                  partition=1, num_partitions=2)
        self.assertEqual(ids(out, "point", "group1"), [4, 5])
        out = run("group_range", g, domain="points", base_group="eligible", range_type="length",
                  start=0, length=1, invert=True)
        self.assertEqual(ids(out, "point", "group1"), [1, 2, 4, 5])
        out = run("group_range", g, domain="points", range_type="length", start=1, length=1,
                  disconnected_separately=True)
        self.assertEqual(ids(out, "point", "group1"), [1, 4])
        self.assertEqual(ids(g, "point", "eligible"), [0, 1, 2, 4, 5])


if __name__ == "__main__":
    unittest.main()
