"""boolean_fracture, convex_decomposition and assemble."""
import unittest

import numpy as np

from minihou import Evaluator, Graph, NodeError, edges, outgroups, topo
from minihou.nodes.bevel import twins
from minihou.nodes.fracture_tools import concave_volume, mesh_volume
from minihou.nodes.group import keep_prims


def cook(nodes, last):
    nodes = dict(nodes)
    nodes["o"] = {"type": "output", "inputs": {"geo": last}}
    g = Evaluator().cook(Graph.from_dict({"nodes": nodes, "display": "o"}))
    g.validate()
    return g


def edge_count(g, name):
    return int((edges.values(g, edges.PREFIX + name) != 0).sum())


def part(g, k, attr="piece"):
    return keep_prims(g, g.prim[attr] == k)


CUTTERS = {"b": {"type": "box", "params": {"size": [2, 2, 2]}},
           "c1": {"type": "sphere", "params": {"radius": 0.7, "center": [0.6, 0, 0]}},
           "c2": {"type": "box", "params": {"size": [0.8, 3, 0.8], "center": [-0.6, 0, 0]}},
           "cm": {"type": "merge", "inputs": {"geos": ["c1", "c2"]}}}


def bf(**params):
    n = dict(CUTTERS)
    n["bf"] = {"type": "boolean_fracture", "inputs": {"geo": "b", "cutters": "cm"}, "params": params}
    return n


class BooleanFractureTests(unittest.TestCase):
    def test_pieces_are_closed_and_add_up_to_the_input(self):
        g = cook(bf(), "bf")
        self.assertTrue(np.all(twins(g) >= 0))
        self.assertAlmostEqual(mesh_volume(g), 8.0, places=4)
        self.assertEqual(sorted(set(g.prim["cutter"].tolist())), [-1, 0, 1])         # two cutters and the remainder
        self.assertEqual(len(set(g.prim["piece"].tolist())), 3)
        for k in set(g.prim["piece"].tolist()):
            self.assertGreater(mesh_volume(part(g, k)), 0.1)

    def test_default_groups_and_all_the_optional_ones(self):
        self.assertEqual(outgroups.names_of(cook(bf(), "bf")), {"prim": ["inside"], "point": [], "edge": []})
        g = cook(bf(outgrp_outside=True, outgrp_cutter=True, outgrp_remainder=True, outgrp_cutpoints=True, outgrp_cutedges=True), "bf")
        ins, out = g.prim["group_inside"], g.prim["group_outside"]
        self.assertTrue(np.all(ins + out == 1))
        self.assertTrue(np.array_equal(g.prim["group_cutterpieces"].astype(bool), g.prim["cutter"] >= 0))
        self.assertTrue(np.array_equal(g.prim["group_remainder"].astype(bool), g.prim["cutter"] < 0))
        self.assertGreater(edge_count(g, "cutedges"), 0)
        e = topo.edges(g)
        marked = edges.values(g, edges.PREFIX + "cutedges") != 0
        self.assertTrue(g.point["group_cutpoints"].astype(bool)[e.points[marked]].all())
        # a cutter's piece has the cutter's own surface as its inside faces; the outer surface of the box is outside

    def test_keep_inside_or_outside_only(self):
        inside = cook(bf(keep="inside"), "bf")
        outside = cook(bf(keep="outside"), "bf")
        self.assertTrue((inside.prim["cutter"] >= 0).all())
        self.assertTrue((outside.prim["cutter"] < 0).all())
        self.assertAlmostEqual(mesh_volume(inside) + mesh_volume(outside), 8.0, places=4)

    def test_union_mode_gives_one_inside_piece(self):
        g = cook(bf(mode="union"), "bf")
        self.assertEqual(sorted(set(g.prim["cutter"].tolist())), [-1, 0])
        self.assertAlmostEqual(mesh_volume(g), 8.0, places=4)

    def test_overlapping_cutters_do_not_double_count(self):
        n = dict(CUTTERS)
        n["c2"] = {"type": "sphere", "params": {"radius": 0.7, "center": [0.9, 0, 0]}}      # overlaps c1
        n["bf"] = {"type": "boolean_fracture", "inputs": {"geo": "b", "cutters": "cm"}, "params": {}}
        g = cook(n, "bf")
        self.assertAlmostEqual(mesh_volume(g), 8.0, places=3)

    def test_a_cutter_that_misses_leaves_only_the_remainder(self):
        n = dict(CUTTERS)
        n["c1"] = {"type": "sphere", "params": {"radius": 0.3, "center": [9, 0, 0]}}
        n["c2"] = {"type": "sphere", "params": {"radius": 0.3, "center": [9, 3, 0]}}
        n["bf"] = {"type": "boolean_fracture", "inputs": {"geo": "b", "cutters": "cm"}, "params": {}}
        g = cook(n, "bf")
        self.assertEqual(sorted(set(g.prim["cutter"].tolist())), [-1])
        self.assertAlmostEqual(mesh_volume(g), 8.0, places=4)

    def test_open_input_is_an_error(self):
        n = dict(CUTTERS)
        n["b"] = {"type": "circle", "params": {}}
        n["bf"] = {"type": "boolean_fracture", "inputs": {"geo": "b", "cutters": "cm"}, "params": {}}
        with self.assertRaises(NodeError):
            cook(n, "bf")

    def test_feeds_the_rbd_nodes(self):
        n = bf(keep="both")
        n["a"] = {"type": "assemble", "inputs": {"geo": "bf"}, "params": {}}
        n["r"] = {"type": "rbd_configure", "inputs": {"geo": "a"}, "params": {"piece_attrib": "piece"}}
        g = cook(n, "r")
        self.assertEqual(len(set(g.prim["rbd_id"].tolist())), 3)


def l_shape(**params):
    return {"a": {"type": "box", "params": {"size": [2, 1, 1]}},
            "b": {"type": "box", "params": {"size": [1, 1, 2], "center": [-0.5, 0, 0.5]}},
            "u": {"type": "boolean", "inputs": {"a": "a", "b": "b"}, "params": {"operation": "union"}},
            "cd": {"type": "convex_decomposition", "inputs": {"geo": "u"}, "params": params}}


class ConvexDecompositionTests(unittest.TestCase):
    def test_an_l_shape_becomes_two_convex_boxes(self):
        g = cook(l_shape(), "cd")
        ids = sorted(set(g.prim["piece"].tolist()))
        self.assertEqual(ids, [0, 1])
        self.assertTrue(np.all(twins(g) >= 0))
        self.assertAlmostEqual(mesh_volume(g), 3.0, places=4)
        for k in ids:
            v, missing = concave_volume(part(g, k))
            self.assertLess(missing / v, 1e-3)

    def test_groups(self):
        self.assertEqual(outgroups.names_of(cook(l_shape(), "cd"))["prim"], ["from_a", "from_b", "inside"])   # boolean's own, plus inside
        g = cook(l_shape(outgrp_outside=True, outgrp_cutpoints=True, outgrp_cutedges=True), "cd")
        self.assertTrue(np.all(g.prim["group_inside"] + g.prim["group_outside"] == 1))
        self.assertEqual(int(g.prim["group_inside"].sum()), 2)                         # one cap on each side of the cut
        self.assertGreater(edge_count(g, "cutedges"), 0)
        self.assertGreater(int(g.point["group_cutpoints"].sum()), 0)

    def test_a_convex_shape_is_left_alone(self):
        n = {"b": {"type": "box", "params": {}}, "cd": {"type": "convex_decomposition", "inputs": {"geo": "b"}, "params": {}}}
        g = cook(n, "cd")
        self.assertEqual(g.nprims, 6)
        self.assertEqual(set(g.prim["piece"].tolist()), {0})

    def test_a_ring_is_cut_into_more_convex_parts_as_pieces_are_allowed(self):
        results = {}
        for mp in (4, 12):
            n = {"t": {"type": "torus", "params": {}}, "cd": {"type": "convex_decomposition", "inputs": {"geo": "t"},
                                                               "params": {"max_pieces": mp, "max_concavity": 0.05}}}
            g = cook(n, "cd")
            ids = sorted(set(g.prim["piece"].tolist()))
            self.assertLessEqual(len(ids), mp)
            self.assertTrue(np.all(twins(g) >= 0))
            results[mp] = (len(ids), max(concave_volume(part(g, k))[1] / mesh_volume(part(g, k)) for k in ids))
        self.assertEqual(results[4][0], 4)
        self.assertGreater(results[12][0], 4)
        self.assertLess(results[12][1], results[4][1])                                 # more pieces: less concave

    def test_open_or_empty_input_is_an_error(self):
        n = {"g": {"type": "grid", "params": {}}, "cd": {"type": "convex_decomposition", "inputs": {"geo": "g"}, "params": {}}}
        with self.assertRaises(NodeError):
            cook(n, "cd")

    def test_pieces_work_with_the_rbd_solver_input_rules(self):
        n = l_shape()
        n["asm"] = {"type": "assemble", "inputs": {"geo": "cd"}, "params": {}}
        n["r"] = {"type": "rbd_configure", "inputs": {"geo": "asm"}, "params": {"piece_attrib": "piece"}}
        g = cook(n, "r")
        self.assertEqual(len(set(g.prim["rbd_id"].tolist())), 2)


class AssembleTests(unittest.TestCase):
    def fractured(self, **asm):
        n = {"b": {"type": "box", "params": {"size": [2, 2, 2]}},
             "fp": {"type": "voronoi_fracture_points", "inputs": {"geo": "b"}, "params": {"count": 5, "seed": 1}},
             "f": {"type": "voronoi_fracture", "inputs": {"geo": "b", "points": "fp"}},
             "a": {"type": "assemble", "inputs": {"geo": "f"}, "params": asm}}
        return n

    def test_attribute_source_numbers_names_and_pivots(self):
        g = cook(self.fractured(pivot="centroid"), "a")
        self.assertEqual(sorted(set(g.prim["piece"].tolist())), [0, 1, 2, 3, 4])
        self.assertEqual(g.detail["piece_names"], [f"piece{k}" for k in range(5)])
        self.assertEqual(g.prim["pivot"].shape, (g.nprims, 3))
        for k in range(5):
            p = part(g, k)
            self.assertTrue(np.allclose(g.prim["pivot"][g.prim["piece"] == k][0], p.P.mean(0), atol=1e-5))

    def test_pivot_modes(self):
        o = cook(self.fractured(pivot="origin"), "a")
        self.assertTrue(np.allclose(o.prim["pivot"], 0))
        b = cook(self.fractured(pivot="bounds_bottom"), "a")
        for k in range(5):
            p = part(b, k)
            self.assertAlmostEqual(float(b.prim["pivot"][b.prim["piece"] == k][0][2]), float(p.P[:, 2].min()), places=5)

    def test_renumber_and_prefix_and_piece_groups(self):
        n = self.fractured(renumber=True, name_prefix="chunk", piece_groups=True, output_attrib="id")
        n["f"]["params"] = {"piece_attrib": "piece"}
        g = cook(n, "a")
        self.assertIn("id", g.prim)
        self.assertEqual(g.detail["piece_names"][0], "chunk0")
        self.assertEqual(outgroups.names_of(g)["prim"][:2], ["chunk0", "chunk1"])
        self.assertTrue(np.array_equal(g.prim["group_chunk2"].astype(bool), g.prim["id"] == 2))

    def test_connectivity_source_and_unsharing_points(self):
        n = self.fractured(source="connectivity", outgrp_unshared=True, outgrp_shared=True)
        n["a"]["inputs"] = {"geo": "w"}
        n["w"] = {"type": "fuse", "inputs": {"geo": "f"}, "params": {"distance": 1e-4}}
        n["a2"] = {"type": "assemble", "inputs": {"geo": "w"}, "params": {"source": "attribute", "outgrp_unshared": True,
                                                                         "outgrp_shared": True}}
        welded = cook(n, "w")
        g = cook(n, "a2")
        self.assertGreater(g.npoints, welded.npoints)                                   # shared points were copied
        self.assertEqual(int(g.point["group_splitpoints"].sum()), g.npoints - welded.npoints)
        self.assertGreater(int(g.point["group_sharedpoints"].sum()), 0)
        # every piece now has points of its own: no point is used by two pieces
        owner = {}
        for p, k in zip(g.topology.vertex_point[topo.prim_of_vertex(g) * 0 + np.arange(g.nvertices)], g.prim["piece"][topo.prim_of_vertex(g)]):
            self.assertEqual(owner.setdefault(int(p), int(k)), int(k))
        # and the rbd solver's own "pieces share points" check passes
        n["r"] = {"type": "rbd_configure", "inputs": {"geo": "a2"}, "params": {"piece_attrib": "piece"}}
        from minihou.rbd import pieces
        self.assertEqual(len(pieces(cook(n, "r"))), 5)

    def test_groups_source(self):
        n = {"b": {"type": "box", "params": {}},
             "g1": {"type": "group_create", "inputs": {"geo": "b"}, "params": {"name": "top", "domain": "primitives", "pattern": "0-1"}},
             "g2": {"type": "group_create", "inputs": {"geo": "g1"}, "params": {"name": "side", "domain": "primitives", "pattern": "2-3"}},
             "a": {"type": "assemble", "inputs": {"geo": "g2"}, "params": {"source": "groups", "groups": "top side",
                                                                         "remove_source_groups": True}}}
        g = cook(n, "a")
        self.assertEqual(g.prim["piece"].tolist(), [0, 0, 1, 1, 2, 2])                  # the two groups and the faces in neither
        self.assertEqual(outgroups.names_of(g)["prim"], [])
        n["a"]["params"]["groups"] = "nothing*"
        with self.assertRaises(NodeError):
            cook(n, "a")

    def test_missing_attribute_is_an_error(self):
        n = self.fractured(piece_attrib="nope")
        with self.assertRaises(NodeError):
            cook(n, "a")


if __name__ == "__main__":
    unittest.main()
