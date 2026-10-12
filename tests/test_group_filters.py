"""Group Create in Houdini's filter style: a base group plus bounding region, normals, edges and random chance sections, each
enabled separately (an element must pass every enabled one), for points, primitives and edges, with Initial Merge and boundary groups."""
import unittest

import numpy as np

from minihou import Evaluator, Graph, NodeError, edges, outgroups, topo


def cook(params, size=4, rows=5, extra=None, last="gc"):
    n = {"g": {"type": "grid", "params": {"size_x": size, "size_y": size, "rows": rows, "columns": rows}},
         "gc": {"type": "group_create", "inputs": {"geo": "g"}, "params": dict({"name": "sel", "method": "filters", "domain": "primitives"}, **params)}}
    n.update(extra or {})
    n["o"] = {"type": "output", "inputs": {"geo": last}}
    return Evaluator().cook(Graph.from_dict({"nodes": n, "display": "o"}))


def count(g, dom="prim", name="sel"):
    if dom == "edge":
        return int((edges.values(g, edges.PREFIX + name) != 0).sum())
    return int((g.table(dom)["group_" + name] != 0).sum())


class FilterTests(unittest.TestCase):
    def test_nothing_enabled_selects_everything_and_the_base_group_comes_first(self):
        self.assertEqual(count(cook({})), 16)
        self.assertEqual(count(cook({"pattern": "0-3"})), 4)
        self.assertEqual(count(cook({"pattern": "0-3", "base_enable": False})), 16)           # the base group can be switched off

    def test_bounding_box_full_and_partial_and_sphere(self):
        box = {"bounds_enable": True, "box_size": [2, 2, 2], "box_center": [1, 1, 0]}         # covers the 2 x 2 cells around (1, 1)
        self.assertEqual(count(cook(box)), 4)
        self.assertEqual(count(cook(dict(box, bounds_partial=True))), 9)                      # touching cells count when partial
        pts = cook(dict(box, domain="points"))
        self.assertEqual(count(pts, "point"), 9)
        self.assertEqual(count(cook(dict(box, bounds_type="sphere", box_size=[4, 4, 4], domain="points")), "point"), 11)   # radius 2 round (1, 1): 11 lattice points
        small = cook({"bounds_enable": True, "bounds_type": "sphere", "box_size": [2.2, 2.2, 2.2], "box_center": [0, 0, 0], "domain": "points"})
        self.assertEqual(count(small, "point"), 5)                                            # the centre and its four neighbours at distance 1

    def test_filters_intersect(self):
        both = cook({"pattern": "0-11", "bounds_enable": True, "box_size": [4, 2, 2], "box_center": [0, 1, 0]})
        self.assertEqual(count(both), 4)                                                      # three rows of the grid AND the region: only the shared row
        self.assertEqual(count(cook({"pattern": "0-11", "bounds_enable": True, "box_size": [4, 2, 2], "box_center": [0, 1, 0],
                                     "random_enable": True, "percent": 0})), 0)

    def test_normals_direction_spread_opposite_and_backface(self):
        up = {"normals_enable": True, "direction": [0, 0, 1], "angle": 10}
        self.assertEqual(count(cook(up)), 16)                                                 # a grid faces +Z
        self.assertEqual(count(cook(dict(up, direction=[0, 0, -1]))), 0)
        self.assertEqual(count(cook(dict(up, direction=[0, 0, -1], normals_opposite=True))), 16)
        self.assertEqual(count(cook(dict(up, backface_from="0 0 5"))), 16)                    # seen from above: all face the viewer
        self.assertEqual(count(cook(dict(up, backface_from="0 0 -5"))), 0)
        with self.assertRaisesRegex(NodeError, "backface_from"):
            cook(dict(up, backface_from="nonsense"))

    def test_random_is_repeatable_proportional_and_seedable_by_an_attribute(self):
        a, b = cook({"random_enable": True, "percent": 50, "seed": 1}, rows=21), cook({"random_enable": True, "percent": 50, "seed": 1}, rows=21)
        np.testing.assert_array_equal(a.prim["group_sel"], b.prim["group_sel"])
        c = cook({"random_enable": True, "percent": 50, "seed": 2}, rows=21)
        self.assertFalse((a.prim["group_sel"] == c.prim["group_sel"]).all())
        self.assertAlmostEqual(count(a) / a.nprims, 0.5, delta=0.1)
        self.assertEqual(count(cook({"random_enable": True, "percent": 100}, rows=21)), 400)
        # a seed attribute gives equal values the same fate, whatever their element number
        extra = {"w": {"type": "wrangle", "inputs": {"geo": "gc"}, "params": {"snippet": "i@cell = @primnum % 4;", "run_over": "primitives"}},
                 "gc2": {"type": "group_create", "inputs": {"geo": "w"}, "params": {"name": "r", "method": "filters", "domain": "primitives",
                                                                                    "random_enable": True, "percent": 50, "seed_attrib": "cell"}}}
        g = cook({}, rows=9, extra=extra, last="gc2")
        cell, r = g.prim["cell"], g.prim["group_r"] != 0
        for k in range(4):
            self.assertEqual(len(set(r[cell == k].tolist())), 1, k)                           # every prim with the same cell agrees

    def test_edge_groups_by_angle_length_and_unshared(self):
        border = cook({"domain": "edges", "edges_enable": True, "unshared_edges": True})
        self.assertEqual(count(border, "edge"), 16)
        long_only = cook({"domain": "edges", "edges_enable": True, "min_edge_length_on": True, "min_edge_length": 1.2}, rows=3)        # cells of 2
        self.assertEqual(count(long_only, "edge"), 12)
        short_only = cook({"domain": "edges", "edges_enable": True, "max_edge_length_on": True, "max_edge_length": 1.0}, rows=3)
        self.assertEqual(count(short_only, "edge"), 0)
        # a folded sheet: edges at the fold are the only ones off flat
        folded = {"b": {"type": "box", "params": {"size": [2, 2, 2]}}, "gc": {"type": "group_create", "inputs": {"geo": "b"},
                  "params": {"name": "sel", "method": "filters", "domain": "edges", "edges_enable": True, "min_edge_angle_on": True, "min_edge_angle": 45}}}
        box = Evaluator().cook(Graph.from_dict({"nodes": dict(folded, o={"type": "output", "inputs": {"geo": "gc"}}), "display": "o"}))
        self.assertEqual(count(box, "edge"), 12)                                              # all twelve edges meet at 90 degrees
        flat = cook({"domain": "edges", "edges_enable": True, "min_edge_angle_on": True, "min_edge_angle": 5})
        self.assertEqual(count(flat, "edge"), 0)                                              # nothing is off flat on a grid
        either = cook({"domain": "edges", "edges_enable": True, "max_edge_angle_on": True, "max_edge_angle": 5})
        self.assertEqual(count(either, "edge"), len(topo.edges(either).points))

    def test_edge_groups_with_bounds_and_base_patterns(self):
        g = cook({"domain": "edges", "bounds_enable": True, "box_size": [2.1, 2.1, 2.1], "box_center": [1, 1, 0]})
        self.assertEqual(count(g, "edge"), 12)                                                # the 2 x 2 block of cells has 12 edges
        some = cook({"domain": "edges", "pattern": "0-4"})
        self.assertEqual(count(some, "edge"), 5)

    def test_initial_merge_with_an_existing_group(self):
        extra = {"first": {"type": "group_create", "inputs": {"geo": "g"}, "params": {"name": "sel", "method": "pattern", "pattern": "0-5", "domain": "primitives"}}}
        merged = lambda how: count(cook({"pattern": "4-9", "combine": how}, extra=dict(extra, gc={"type": "group_create", "inputs": {"geo": "first"},
                                                        "params": {"name": "sel", "method": "filters", "domain": "primitives", "pattern": "4-9", "combine": how}})))   # noqa: E731
        self.assertEqual([merged(h) for h in ("replace", "union", "intersect", "subtract")], [6, 10, 2, 4])

    def test_boundary_groups_outline_the_selection(self):
        g = cook({"bounds_enable": True, "box_size": [2, 2, 2], "box_center": [1, 1, 0], "boundary_groups": True})
        self.assertEqual(count(g, "edge", "sel_boundary"), 8)                                 # the perimeter of a 2 x 2 block
        pts = cook({"bounds_enable": True, "box_size": [2, 2, 2], "box_center": [1, 1, 0], "boundary_groups": True, "domain": "points"})
        self.assertIn("sel_boundary", outgroups.names_of(pts)["edge"])
        off = cook({"bounds_enable": True, "box_size": [2, 2, 2], "box_center": [1, 1, 0]})
        self.assertEqual(outgroups.names_of(off)["edge"], [])

    def test_unshared_edges_for_points_and_primitives_means_the_border(self):
        self.assertEqual(count(cook({"edges_enable": True, "unshared_edges": True})), 12)
        self.assertEqual(count(cook({"edges_enable": True, "unshared_edges": True, "domain": "points"}), "point"), 16)

    def test_the_older_methods_still_work_on_every_domain(self):
        old = cook({"method": "bounding_box", "box_size": [2, 2, 2], "box_center": [1, 1, 0]})
        self.assertEqual(count(old), 4)
        e = cook({"method": "boundary", "domain": "edges"})
        self.assertEqual(count(e, "edge"), 16)
        self.assertEqual(count(cook({"method": "random", "percent": 100, "domain": "edges"}), "edge"), len(topo.edges(e).points))

    def test_a_group_made_here_is_usable_downstream_including_the_edge_group(self):
        extra = {"bv": {"type": "polybevel", "inputs": {"geo": "gc"}, "params": {"distance": 0.1, "group": "sel", "group_type": "edges", "min_angle": 0}}}
        n = {"b": {"type": "box", "params": {"size": [2, 2, 2]}},
             "gc": {"type": "group_create", "inputs": {"geo": "b"}, "params": {"name": "sel", "method": "filters", "domain": "edges",
                                                                               "edges_enable": True, "min_edge_angle_on": True, "min_edge_angle": 45}},
             "bv": extra["bv"], "o": {"type": "output", "inputs": {"geo": "bv"}}}
        out = Evaluator().cook(Graph.from_dict({"nodes": n, "display": "o"}))
        self.assertGreater(out.nprims, 6)                                                     # a box with every edge bevelled


if __name__ == "__main__":
    unittest.main()
