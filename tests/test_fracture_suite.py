"""RBD fracture toolbox: output groups on voronoi_fracture, voronoi_split, voronoi_fracture_points, voronoi_adjacency."""
import unittest

import numpy as np

from minihou import Evaluator, Graph, NodeError, edges, outgroups, topo
from minihou.nodes.bevel import twins
from minihou.nodes.normal import prim_normals


def cook(nodes, last):
    nodes = dict(nodes)
    nodes["o"] = {"type": "output", "inputs": {"geo": last}}
    g = Evaluator().cook(Graph.from_dict({"nodes": nodes, "display": "o"}))
    g.validate()
    return g


BOX = {"b": {"type": "box", "params": {"size": [2, 2, 2]}}}


def fracture_graph(count=10, fp=None, f=None):
    n = dict(BOX)
    n["fp"] = {"type": "voronoi_fracture_points", "inputs": {"geo": "b"}, "params": dict({"count": count, "seed": 3}, **(fp or {}))}
    n["f"] = {"type": "voronoi_fracture", "inputs": {"geo": "b", "points": "fp"}, "params": f or {}}
    return n


def edge_count(g, name):
    return int((edges.values(g, edges.PREFIX + name) != 0).sum())


class FractureGroupTests(unittest.TestCase):
    def test_inside_is_on_by_default_and_the_others_are_off(self):
        g = cook(fracture_graph(), "f")
        self.assertEqual(outgroups.names_of(g), {"prim": ["inside"], "point": [], "edge": []})
        self.assertGreater(int(g.prim["group_inside"].sum()), 0)

    def test_the_legacy_inside_group_parameter_still_names_or_removes_it(self):
        g = cook(fracture_graph(f={"inside_group": "cut"}), "f")
        self.assertEqual(outgroups.names_of(g)["prim"], ["cut"])
        g = cook(fracture_graph(f={"inside_group": ""}), "f")
        self.assertEqual(outgroups.names_of(g)["prim"], [])

    def test_outside_cut_points_and_cut_edges(self):
        g = cook(fracture_graph(f={"outgrp_outside": True, "outgrp_cutpoints": True, "outgrp_cutedges": True}), "f")
        ins, out = g.prim["group_inside"], g.prim["group_outside"]
        self.assertTrue(np.all(ins + out == 1))                          # every face is one or the other
        self.assertGreater(int(out.sum()), 0)
        self.assertGreater(edge_count(g, "cutedges"), 0)
        # the cut edges are exactly the seams between inside and outside faces
        from minihou.nodes.fracture import cut_edge_mask
        seam = cut_edge_mask(g, ins.astype(bool))
        marked = edges.values(g, edges.PREFIX + "cutedges") != 0
        self.assertTrue(np.all(marked >= seam))
        e = topo.edges(g)
        cp = g.point["group_cutpoints"].astype(bool)
        self.assertTrue(cp[e.points[seam]].all())                        # every seam edge's points are cut points

    def test_pieces_still_closed_with_groups_on(self):
        g = cook(fracture_graph(f={"outgrp_outside": True, "outgrp_cutpoints": True, "outgrp_cutedges": True}), "f")
        self.assertTrue(np.all(twins(g) >= 0))


class VoronoiSplitTests(unittest.TestCase):
    def graph(self, **params):
        return {"g": {"type": "grid", "params": {"size_x": 2, "size_y": 2, "rows": 6, "columns": 6}},
                "p": {"type": "scatter", "inputs": {"geo": "g"}, "params": {"count": 5, "seed": 2}},
                "s": {"type": "voronoi_split", "inputs": {"geo": "g", "points": "p"}, "params": params}}

    def test_splits_an_open_sheet_without_capping_or_losing_area(self):
        g = cook(self.graph(), "s")
        self.assertGreater(len(set(g.prim["piece"].tolist())), 1)
        area = 0.5 * np.linalg.norm(prim_normals(g), axis=1).sum()
        self.assertAlmostEqual(area, 4.0, places=5)                      # nothing added, nothing lost
        self.assertEqual(outgroups.names_of(g), {"prim": [], "point": [], "edge": []})

    def test_cut_groups(self):
        g = cook(self.graph(outgrp_cutpoints=True, outgrp_cutedges=True), "s")
        self.assertGreater(int(g.point["group_cutpoints"].sum()), 0)
        self.assertGreater(edge_count(g, "cutedges"), 0)
        e = topo.edges(g)
        cp = g.point["group_cutpoints"].astype(bool)
        marked = edges.values(g, edges.PREFIX + "cutedges") != 0
        self.assertTrue(cp[e.points[marked]].all())

    def test_solid_input_is_left_open_where_it_was_cut(self):
        n = dict(BOX)
        n["fp"] = {"type": "voronoi_fracture_points", "inputs": {"geo": "b"}, "params": {"count": 4, "seed": 1}}
        n["s"] = {"type": "voronoi_split", "inputs": {"geo": "b", "points": "fp"}, "params": {}}
        g = cook(n, "s")
        self.assertTrue(np.any(twins(g) < 0))                            # cut open, no new faces
        self.assertAlmostEqual(0.5 * np.linalg.norm(prim_normals(g), axis=1).sum(), 24.0, places=4)


class FracturePointsTests(unittest.TestCase):
    def pts(self, **params):
        return cook(fracture_graph(fp=params), "fp")

    def test_count_inside_the_shape_and_seed(self):
        g = self.pts(count=30)
        self.assertEqual(g.npoints, 30)
        self.assertTrue((np.abs(g.P) <= 1.0 + 1e-6).all())
        self.assertTrue(np.array_equal(g.P, self.pts(count=30).P))
        self.assertFalse(np.array_equal(g.P, self.pts(count=30, seed=9).P))
        self.assertEqual(g.point["piece"].tolist(), list(range(30)))

    def test_surface_fraction_and_groups(self):
        g = self.pts(count=20, surface_fraction=0.5, outgrp_surface=True, outgrp_interior=True)
        on_surface = (np.abs(np.abs(g.P) - 1.0) < 1e-5).any(axis=1)
        self.assertEqual(int(g.point["group_surface"].sum()), 10)
        self.assertEqual(int(g.point["group_interior"].sum()), 10)
        self.assertTrue(np.array_equal(on_surface, g.point["group_surface"].astype(bool)))

    def test_min_distance_thins_and_relax_evens_out(self):
        g = self.pts(count=60, min_distance=0.5)
        i, j, d = __import__("minihou").spatial.pairs_within(g.P.astype(float), 0.5)
        self.assertEqual(len(i), 0)
        self.assertLess(g.npoints, 60)

        def spread(q):
            from minihou.spatial import nearest
            s = []
            for k in range(len(q)):
                rest = np.delete(q, k, axis=0)
                s.append(nearest(rest, q[k:k + 1])[1][0])
            return float(np.std(s))
        raw, relaxed = self.pts(count=20), self.pts(count=20, relax=8)
        self.assertLess(spread(relaxed.P.astype(float)), spread(raw.P.astype(float)))

    def test_clusters_concentrate_points(self):
        g = self.pts(count=40, cluster_amount=1.0, clusters=1, cluster_radius=0.1, outgrp_clustered=True)
        self.assertEqual(int(g.point["group_clustered"].sum()), 40)
        self.assertLess(float(np.ptp(g.P, axis=0).max()), 1.8)           # a tight blob, not the whole box

    def test_extra_input_points_are_kept_and_grouped(self):
        n = dict(BOX)
        n["x"] = {"type": "box", "params": {"size": [0.1, 0.1, 0.1], "center": [0.5, 0.5, 0.5]}}
        n["fp"] = {"type": "voronoi_fracture_points", "inputs": {"geo": "b", "points": "x"},
                   "params": {"count": 3, "outgrp_extra": True, "min_distance": 5.0}}
        g = cook(n, "fp")
        self.assertEqual(int(g.point["group_extrapoints"].sum()), 8)     # all eight corners survive thinning
        self.assertEqual(g.npoints, 8 + 1)

    def test_open_input_is_rejected(self):
        n = {"g": {"type": "circle", "params": {}}, "fp": {"type": "voronoi_fracture_points", "inputs": {"geo": "g"}}}
        with self.assertRaises(NodeError):
            cook(n, "fp")


class AdjacencyTests(unittest.TestCase):
    def test_a_row_of_slabs_links_neighbours_only(self):
        # three Voronoi cells in a line: 0-1 and 1-2 touch, 0-2 do not
        n = dict(BOX)
        n["p"] = {"type": "curve", "params": {"points": "-1.2,0,0 0,0,0 1.2,0,0"}}
        n["f"] = {"type": "voronoi_fracture", "inputs": {"geo": "b", "points": "p"}}
        n["a"] = {"type": "voronoi_adjacency", "inputs": {"geo": "f", "points": "p"},
                  "params": {"outgrp_connected": True, "outgrp_isolated": True}}
        g = cook(n, "a")
        pairs = sorted(zip(g.prim["piece_a"].tolist(), g.prim["piece_b"].tolist()))
        self.assertEqual(pairs, [(0, 1), (1, 2)])
        self.assertTrue(np.allclose(g.prim["area"], 4.0, atol=1e-4))     # each contact is a full 2 x 2 face
        self.assertEqual(g.point["neighbors"].tolist(), [1, 2, 1])
        self.assertEqual(int(g.point["group_connected"].sum()), 3)
        self.assertEqual(int(g.point["group_isolated"].sum()), 0)
        self.assertTrue(np.allclose(g.P[:, 0], [-1.2, 0, 1.2]))          # centres come from the points input

    def test_isolated_pieces_and_strong_links(self):
        n = dict(BOX)
        n["c"] = {"type": "box", "params": {"size": [1, 1, 1], "center": [10, 0, 0]}}
        n["m"] = {"type": "merge", "inputs": {"geos": ["b", "c"]}}
        n["a"] = {"type": "voronoi_adjacency", "inputs": {"geo": "m"},
                  "params": {"piece_attrib": "", "outgrp_isolated": True, "outgrp_connected": True}}
        g = cook(n, "a")
        self.assertEqual(g.nprims, 0)                                    # two separate boxes: no shared faces
        self.assertEqual(int(g.point["group_isolated"].sum()), 2)

    def test_fracture_gap_needs_max_gap(self):
        n = fracture_graph(count=6, f={"gap": 0.05})
        n["a0"] = {"type": "voronoi_adjacency", "inputs": {"geo": "f"}, "params": {}}
        n["a1"] = {"type": "voronoi_adjacency", "inputs": {"geo": "f"}, "params": {"max_gap": 0.06}}
        self.assertEqual(cook(n, "a0").nprims, 0)
        self.assertGreater(cook(n, "a1").nprims, 3)

    def test_min_area_and_missing_attribute(self):
        n = fracture_graph(count=10)
        n["a"] = {"type": "voronoi_adjacency", "inputs": {"geo": "f"}, "params": {"min_area": 1e9}}
        self.assertEqual(cook(n, "a").nprims, 0)
        n["a"]["params"] = {"piece_attrib": "nope"}
        with self.assertRaises(NodeError):
            cook(n, "a")


if __name__ == "__main__":
    unittest.main()
