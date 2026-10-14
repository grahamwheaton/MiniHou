"""Inputs added to match Houdini's nodes: bounding objects for groups, match_size's destination, sweep's cross-section."""
import unittest

import numpy as np

from minihou import Evaluator, Graph


def cook(nodes, last):
    nodes = dict(nodes)
    nodes["out"] = {"type": "output", "inputs": {"geo": last}}
    g = Evaluator().cook(Graph.from_dict({"nodes": nodes, "display": "out"}))
    g.validate()
    return g


class GroupBoundsTests(unittest.TestCase):
    SRC = {"g": {"type": "grid", "params": {"size_x": 4, "size_y": 4, "rows": 5, "columns": 5}},
           "box": {"type": "box", "params": {"size": (2, 2, 2), "center": (1, 1, 0)}}}

    def test_group_create_uses_the_bounding_object(self):
        nodes = dict(self.SRC)
        nodes["grp"] = {"type": "group_create", "inputs": {"geo": "g", "bounds": "box"},
                        "params": {"name": "in", "domain": "points", "method": "bounding_box"}}
        g = cook(nodes, "grp")
        m = g.point["group_in"] != 0
        P = g.P
        want = (np.abs(P[:, 0] - 1) <= 1.0 + 1e-6) & (np.abs(P[:, 1] - 1) <= 1.0 + 1e-6)
        self.assertTrue(want.any())
        self.assertTrue(np.array_equal(m, want))

    def test_group_create_without_it_keeps_the_parameters(self):
        nodes = dict(self.SRC)
        nodes["grp"] = {"type": "group_create", "inputs": {"geo": "g"},
                        "params": {"name": "in", "domain": "points", "method": "bounding_box", "box_center": (0, 0, 0), "box_size": (1, 1, 1)}}
        g = cook(nodes, "grp")
        self.assertEqual(int((g.point["group_in"] != 0).sum()), 1)

    def test_edge_group_bounding_object(self):
        nodes = dict(self.SRC)
        nodes["eg"] = {"type": "edge_group", "inputs": {"geo": "g", "bounds": "box"},
                       "params": {"name": "e", "method": "bounding_object"}}
        g = cook(nodes, "eg")
        from minihou import edges, topo
        m = edges.values(g, edges.PREFIX + "e") != 0
        self.assertGreater(int(m.sum()), 0)
        e = topo.edges(g)
        P = g.P
        inside = (np.abs(P[:, 0] - 1) <= 1.0 + 1e-6) & (np.abs(P[:, 1] - 1) <= 1.0 + 1e-6)
        self.assertTrue(np.array_equal(m, inside[e.points].all(axis=1)))

    def test_edge_group_bounding_object_needs_the_input(self):
        nodes = dict(self.SRC)
        nodes["eg"] = {"type": "edge_group", "inputs": {"geo": "g"}, "params": {"name": "e", "method": "bounding_object"}}
        with self.assertRaises(Exception):
            cook(nodes, "eg")


class MatchSizeTargetTests(unittest.TestCase):
    def test_destination_size_input(self):
        nodes = {"src": {"type": "box", "params": {"size": (1, 1, 1)}},
                 "dst": {"type": "box", "params": {"size": (4, 2, 6), "center": (10, 0, 0)}},
                 "m": {"type": "match_size", "inputs": {"geo": "src", "target": "dst"}, "params": {"scale": True, "uniform": False}}}
        g = cook(nodes, "m")
        lo, hi = g.P.min(0), g.P.max(0)
        np.testing.assert_allclose(hi - lo, (4, 2, 6), atol=1e-5)
        np.testing.assert_allclose((lo + hi) / 2, (10, 0, 0), atol=1e-5)


class SweepSectionTests(unittest.TestCase):
    LINE = {"l": {"type": "line", "params": {"length": 4, "points": 5}}}

    def test_closed_square_section_makes_a_square_tube(self):
        nodes = dict(self.LINE)
        nodes["sec"] = {"type": "circle", "params": {"divisions": 4, "radius": 0.5}}
        nodes["s"] = {"type": "sweep", "inputs": {"geo": "l", "section": "sec"}, "params": {}}
        g = cook(nodes, "s")
        sizes = g.topology.prim_sizes()
        self.assertEqual(int((sizes == 4).sum()), 4 * 4 + 2)          # 4 segments x 4 sides + 2 caps
        self.assertEqual(g.npoints, 5 * 4)
        # the section's size (radius 0.5) is used as is
        ring = g.P[:4]
        self.assertAlmostEqual(float(np.linalg.norm(ring - ring.mean(0), axis=1).max()), 0.5, places=4)

    def test_open_section_makes_a_sheet_without_caps(self):
        nodes = dict(self.LINE)
        nodes["sec"] = {"type": "line", "params": {"length": 1, "points": 3, "direction": (1, 0, 0)}}
        nodes["s"] = {"type": "sweep", "inputs": {"geo": "l", "section": "sec"}, "params": {}}
        g = cook(nodes, "s")
        self.assertEqual(g.nprims, 4 * 2)                             # (3 section points - 1) segments x 4 spans
        self.assertEqual(g.npoints, 5 * 3)

    def test_without_a_section_the_circle_is_unchanged(self):
        nodes = dict(self.LINE)
        nodes["s"] = {"type": "sweep", "inputs": {"geo": "l"}, "params": {"divisions": 6}}
        g = cook(nodes, "s")
        self.assertEqual(g.npoints, 5 * 6)
        self.assertEqual(g.nprims, 4 * 6 + 2)


class BendRestTests(unittest.TestCase):
    def test_rest_geometry_drives_the_bend(self):
        base = {"b": {"type": "box", "params": {"size": (1, 1, 2), "center": (0, 0, 1), "divisions": 4}},
                "moved": {"type": "transform", "inputs": {"geo": "b"}, "params": {"translate": (3, 0, 0)}}}
        plain = cook({**base, "bend": {"type": "bend", "inputs": {"geo": "b"}, "params": {"length": 2, "bend": 60}}}, "bend")
        with_rest = cook({**base, "bend": {"type": "bend", "inputs": {"geo": "moved", "rest": "b"}, "params": {"length": 2, "bend": 60}}}, "bend")
        np.testing.assert_allclose(with_rest.P - np.array([3, 0, 0], np.float32), plain.P, atol=1e-4)
        moved_only = cook({**base, "bend": {"type": "bend", "inputs": {"geo": "moved"}, "params": {"length": 2, "bend": 60}}}, "bend")
        self.assertGreater(float(np.abs((moved_only.P - np.array([3, 0, 0], np.float32)) - plain.P).max()), 0.1)   # no rest: a different shape

    def test_rest_with_other_point_count_is_an_error(self):
        nodes = {"b": {"type": "box", "params": {"divisions": 2}}, "c": {"type": "box", "params": {"divisions": 3}},
                 "bend": {"type": "bend", "inputs": {"geo": "b", "rest": "c"}, "params": {}}}
        with self.assertRaises(Exception):
            cook(nodes, "bend")


class ErodeMaskTests(unittest.TestCase):
    TERRAIN = {"g": {"type": "grid", "params": {"columns": 32, "rows": 32, "size_x": 10, "size_y": 10}},
               "n": {"type": "noise", "inputs": {"geo": "g"}, "params": {"amplitude": [0, 0, 2], "frequency": 0.2, "octaves": 3, "seed": 3}},
               "m": {"type": "wrangle", "inputs": {"geo": "n"}, "params": {"snippet": "f@mask = @P.x > 0;"}}}

    def test_mask_protects_the_terrain(self):
        e = {"type": "heightfield_erode", "inputs": {"geo": "n", "mask": "m"},
             "params": {"iterations": 4, "droplets": 1500, "thermal_iterations": 5, "seed": 1}}
        g = cook({**self.TERRAIN, "e": e}, "e")
        src = cook(self.TERRAIN, "n")
        d = np.abs(g.P[:, 2] - src.P[:, 2])
        left = src.P[:, 0] <= 0
        self.assertEqual(float(d[left].max()), 0.0)             # mask 0: untouched
        self.assertGreater(float(d[~left].max()), 0.01)          # mask 1: eroded

    def test_mask_must_match(self):
        e = {"type": "heightfield_erode", "inputs": {"geo": "n", "mask": "g"}, "params": {"iterations": 1}}
        with self.assertRaises(Exception):
            cook({**self.TERRAIN, "e": e}, "e")


class CurvePassThroughTests(unittest.TestCase):
    def test_curve_and_bezier_add_to_the_geometry_they_are_given(self):
        for kind, params in (("curve", {"points": "0,0,0 1,0,0 1,1,0"}), ("bezier", {})):
            nodes = {"b": {"type": "box"}, "c": {"type": kind, "inputs": {"geo": "b"}, "params": params}}
            g = cook(nodes, "c")
            alone = cook({"c": {"type": kind, "params": params}}, "c")
            self.assertEqual(g.nprims, 6 + alone.nprims, kind)                      # the box's six faces and the curve
            self.assertEqual(g.npoints, 8 + alone.npoints, kind)


class FuseTargetsTests(unittest.TestCase):
    def test_second_input_is_the_snap_target(self):
        nodes = {"a": {"type": "grid", "params": {"size_x": 2, "size_y": 2, "columns": 3, "rows": 3}},
                 "t": {"type": "box", "params": {"size": (0.4, 0.4, 0.4), "center": (1, 1, 0), "primitive_type": "points"}},
                 "f": {"type": "fuse", "inputs": {"geo": "a", "targets": "t"}, "params": {"distance": 0.8, "outgrp_fused": True}}}
        g = cook(nodes, "f")
        src = cook({"a": nodes["a"]}, "a")
        moved = np.any(np.abs(g.P - src.P) > 1e-6, axis=1)
        self.assertEqual(g.npoints, src.npoints)                                  # nothing is welded away
        self.assertEqual(int(moved.sum()), 1)                                      # only the corner within 0.8 of a target point snapped...
        t = cook({"t": nodes["t"]}, "t").P
        self.assertLess(float(np.min(np.linalg.norm(t - g.P[moved][0], axis=1))), 1e-6)   # ...exactly onto one


if __name__ == "__main__":
    unittest.main()
