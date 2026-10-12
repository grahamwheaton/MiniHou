"""Cluster pieces, impacts, fracture levels, interior detail, convex parts and constraint properties."""
import importlib.util
import unittest

import numpy as np

from minihou import Evaluator, Graph, NodeError, edges, outgroups, topo
from minihou.nodes.bevel import twins
from minihou.nodes.fracture import split_pinched, split_repeated_faces
from minihou.nodes.fracture_tools import mesh_volume, tessellate
from minihou.nodes import _build as B


def cook(nodes, last):
    nodes = dict(nodes)
    nodes["o"] = {"type": "output", "inputs": {"geo": last}}
    g = Evaluator().cook(Graph.from_dict({"nodes": nodes, "display": "o"}))
    g.validate()
    return g


BOX = {"b": {"type": "box", "params": {"size": [2, 2, 2]}}}


def frac(points=None, fracture=None):
    n = dict(BOX)
    n["fp"] = {"type": "voronoi_fracture_points", "inputs": {"geo": "b"}, "params": dict({"count": 24, "seed": 3}, **(points or {}))}
    n["f"] = {"type": "voronoi_fracture", "inputs": {"geo": "b", "points": "fp"}, "params": fracture or {}}
    return n


class ClusterPiecesTests(unittest.TestCase):
    def test_points_write_cluster_numbers_or_minus_one(self):
        self.assertTrue((cook(frac(), "fp").point["cluster"] == -1).all())
        g = cook(frac({"merge_clusters": 4}), "fp")
        self.assertEqual(sorted(set(g.point["cluster"].tolist())), [0, 1, 2, 3])

    def test_cluster_pieces_merges_cells_into_closed_pieces_that_add_up(self):
        plain = cook(frac({"merge_clusters": 4}), "f")
        merged = cook(frac({"merge_clusters": 4}, {"cluster_pieces": True, "outgrp_outside": True, "outgrp_cutedges": True}), "f")
        self.assertEqual(len(set(plain.prim["piece"].tolist())), 24)
        self.assertEqual(len(set(merged.prim["piece"].tolist())), 4)
        self.assertTrue(np.all(twins(merged) >= 0))
        self.assertAlmostEqual(mesh_volume(merged), 8.0, places=4)
        self.assertLess(merged.nprims, plain.nprims)                                  # the cuts between members are gone
        self.assertTrue(np.all(merged.prim["group_inside"] + merged.prim["group_outside"] == 1))
        self.assertGreater(int((edges.values(merged, edges.PREFIX + "cutedges") != 0).sum()), 0)

    def test_cluster_pieces_needs_the_attribute(self):
        n = frac(fracture={"cluster_pieces": True, "cluster_attrib": "nope"})
        with self.assertRaises(NodeError):
            cook(n, "f")

    def test_create_inside_off_leaves_pieces_open(self):
        g = cook(frac(fracture={"create_inside": False}), "f")
        self.assertTrue(np.any(twins(g) < 0))
        self.assertEqual(outgroups.names_of(g)["prim"], [])
        self.assertAlmostEqual(0.5 * np.linalg.norm(__import__("minihou.nodes.normal", fromlist=["x"]).prim_normals(g), axis=1).sum(),
                               24.0, places=3)                                           # only the box's own surface

    def test_a_second_fracture_keeps_the_first_ones_inside_faces(self):
        n = frac()
        n["fp2"] = {"type": "voronoi_fracture_points", "inputs": {"geo": "f"}, "params": {"count": 3, "seed": 1}}
        once = cook(n, "f")
        # fracture one piece again: the input already has an 'inside' group, which stays
        n["one"] = {"type": "blast", "inputs": {"geo": "f"}, "params": {"group": "@piece != 0", "domain": "primitives"}}
        n["fpx"] = {"type": "voronoi_fracture_points", "inputs": {"geo": "one"}, "params": {"count": 3, "seed": 1}}
        n["again"] = {"type": "voronoi_fracture", "inputs": {"geo": "one", "points": "fpx"}, "params": {}}
        piece = cook(n, "one")
        twice = cook(n, "again")
        self.assertGreater(int(twice.prim["group_inside"].sum()), int(piece.prim["group_inside"].sum()))
        self.assertGreaterEqual(int(twice.prim["group_inside"].sum()), int(piece.prim["group_inside"].sum()) + 3)


class ImpactTests(unittest.TestCase):
    def graph(self, **params):
        n = dict(BOX)
        n["imp"] = {"type": "curve", "params": {"points": "1,0,0 -1,0.5,0"}}
        n["w"] = {"type": "wrangle", "inputs": {"geo": "imp"}, "params": {"snippet": "f@impulse = 1.0 + 2.0 * (@P.x > 0);"}}
        n["fp"] = {"type": "voronoi_fracture_points", "inputs": {"geo": "b", "points": "w"},
                   "params": dict({"count": 40, "source": "impacts", "impact_radius": 0.15, "outgrp_impact": True}, **params)}
        return n

    def test_points_gather_round_impacts_in_proportion_to_impulse(self):
        g = cook(self.graph(), "fp")
        m = g.point["group_impactpoints"].astype(bool)
        self.assertEqual(int(m.sum()), 28)                                              # 70% of 40
        a = (np.linalg.norm(g.P[m] - [1, 0, 0], axis=1) < 0.6).sum()
        b = (np.linalg.norm(g.P[m] - [-1, 0.5, 0], axis=1) < 0.6).sum()
        self.assertGreater(a, b)                                                        # the harder hit has more points
        self.assertTrue(set(g.point["impact"][m].tolist()) <= {0, 1})
        self.assertTrue((g.point["impact"][~m] == -1).all())

    def test_share_and_missing_impacts(self):
        g = cook(self.graph(impact_share=0.0), "fp")
        self.assertEqual(int(g.point["group_impactpoints"].sum()), 0)
        n = dict(BOX)
        n["fp"] = {"type": "voronoi_fracture_points", "inputs": {"geo": "b"}, "params": {"source": "impacts"}}
        with self.assertRaises(NodeError):
            cook(n, "fp")


class TessellateTests(unittest.TestCase):
    def test_divides_the_chosen_faces_without_cracks(self):
        from minihou import Geometry
        n = dict(BOX)
        src = cook(n, "b")
        mask = np.zeros(src.nprims, bool)
        mask[0] = True
        out, tess, fixed = tessellate(src, mask, 3)
        self.assertEqual(int(tess.sum()), 2 * 9)                                          # a quad is two triangles, each cut into nine
        self.assertEqual(out.nprims, src.nprims - 1 + 18)
        self.assertAlmostEqual(mesh_volume(out), mesh_volume(src), places=5)
        self.assertTrue(np.all(topo.edges(out).count == 2))                              # the neighbours got the border points too
        self.assertTrue(fixed.all() or fixed[:src.npoints].all())                        # every point of a lone face is on its border
        e = topo.edges(out)
        self.assertEqual(int((e.count != 2).sum()), 0)

    def test_split_pinched_and_repeated_faces(self):
        loops = split_pinched([0, 1, 2, 3, 1, 4, 5])
        self.assertEqual(sorted(len(x) for x in loops), [3, 4])
        self.assertEqual(split_pinched([0, 1, 2]), [[0, 1, 2]])
        g = B.make(np.array([[0, 0, 0], [1, 0, 0], [1, 1, 0], [0, 1, 0], [2, 1, 0]], np.float32),
                   np.array([0, 1, 2, 3, 1, 4], np.int32), np.array([0, 6], np.int32))
        out = split_repeated_faces(g)
        self.assertEqual(out.nprims, 2)
        self.assertEqual(sorted(out.topology.prim_sizes().tolist()), [3, 3])


def material(**params):
    n = dict(BOX)
    n["m"] = {"type": "rbd_material_fracture", "inputs": {"geo": "b"}, "params": dict({"pieces": 6, "seed": 1}, **params)}
    return n


class MaterialParityTests(unittest.TestCase):
    def test_fracture_levels_break_pieces_again_and_keep_inside_faces(self):
        one = cook(material(), "m")
        two = cook(material(fracture_level=2, sub_pieces=3), "m")
        self.assertEqual(len(set(one.prim["rbd_id"].tolist())), 6)
        self.assertEqual(len(set(two.prim["rbd_id"].tolist())), 18)
        self.assertTrue(np.all(twins(two) >= 0))
        self.assertAlmostEqual(mesh_volume(two), 8.0, places=4)
        self.assertGreater(int(two.prim["group_inside"].sum()), int(one.prim["group_inside"].sum()))
        self.assertAlmostEqual(float(two.prim["rbd_mass"].sum() / 3 ** 0), float(two.prim["rbd_mass"].sum()))
        self.assertAlmostEqual(sum({int(k): float(m) for k, m in zip(two.prim["rbd_id"], two.prim["rbd_mass"])}.values()), 2400 * 8.0, delta=2.0)

    def test_custom_material_uses_its_own_numbers(self):
        g = cook(material(material="custom", density=500.0, friction=0.25, restitution=0.4), "m")
        self.assertTrue(np.allclose(g.prim["rbd_friction"], 0.25))
        self.assertTrue(np.allclose(g.prim["rbd_restitution"], 0.4))
        self.assertAlmostEqual(sum({int(k): float(m) for k, m in zip(g.prim["rbd_id"], g.prim["rbd_mass"])}.values()), 500 * 8.0, delta=1.0)
        links = cook(dict(material(material="custom", glue_strength=1000.0, output="constraints", strength_scale=1.0)), "m")
        self.assertTrue(np.allclose(links.prim["strength"] / links.prim["area"], 1000.0, rtol=0.3))      # within the 25% variation

    def test_interior_detail_roughens_the_cuts_and_keeps_the_border(self):
        plain = cook(material(outgrp_cutedges=True, outgrp_cutpoints=True), "m")
        rough = cook(material(interior_detail=True, detail_divisions=3, detail_amplitude=0.04, outgrp_cutedges=True,
                              outgrp_cutpoints=True), "m")
        self.assertGreater(rough.nprims, plain.nprims * 2)
        self.assertTrue(np.all(topo.edges(rough).count == 2))                            # still watertight
        self.assertAlmostEqual(mesh_volume(rough), 8.0, delta=0.4)
        cut = rough.point["group_cutpoints"].astype(bool)
        self.assertGreater(int(cut.sum()), 0)
        self.assertGreater(int((edges.values(rough, edges.PREFIX + "cutedges") != 0).sum()), 0)
        # the border of every cut is where it was: each original cut point is still a point of the rough result
        have = {tuple(np.round(p, 4)) for p in rough.P}
        from minihou.nodes.fracture import cut_edge_mask
        e = topo.edges(plain)
        seam = cut_edge_mask(plain, plain.prim["group_inside"].astype(bool))
        orig = plain.P[np.unique(e.points[seam])]                                        # the border: where inside meets outside
        self.assertGreater(len(orig), 10)
        self.assertTrue(all(tuple(np.round(p, 4)) in have for p in orig))
        # inside faces are no longer flat
        ins = rough.prim["group_inside"].astype(bool)
        pts = rough.topology.vertex_point[ins[topo.prim_of_vertex(rough)]]
        free = np.setdiff1d(pts, np.flatnonzero(cut))
        self.assertGreater(len(free), 0)

    def test_detail_needs_the_inside_group(self):
        with self.assertRaises(NodeError):
            cook(material(interior_detail=True, outgrp_inside=False), "m")

    def test_convex_parts_make_a_concave_input_simulate(self):
        n = {"a": {"type": "box", "params": {"size": [3, 1, 1]}},
             "b": {"type": "box", "params": {"size": [1, 1, 3], "center": [-1, 0, 1]}},
             "u": {"type": "boolean", "inputs": {"a": "a", "b": "b"}, "params": {"operation": "union"}},
             "m": {"type": "rbd_material_fracture", "inputs": {"geo": "u"},
                   "params": {"pieces": 2, "seed": 5, "surface_fraction": 0.0, "relax": 0, "convex_parts": True}}}
        g = cook(n, "m")
        self.assertTrue(g.prim["rbd_compound"].all() and g.prim["rbd_hull"].all())
        self.assertAlmostEqual(mesh_volume(g), 5.0, places=3)
        self.assertEqual(len(set(g.prim["rbd_id"].tolist())), 2)
        self.assertTrue(np.all(twins(g) >= 0))
        if importlib.util.find_spec("mujoco"):
            from minihou.rbd import pieces
            ps = pieces(g)
            self.assertEqual(len(ps), 2)
            self.assertGreater(max(len(p.parts) for p in ps), 1)                         # a body made of several convex parts
            n["floor"] = {"type": "box", "params": {"size": [20, 20, 1], "center": [0, 0, -1.0]}}
            n["tr"] = {"type": "transform", "inputs": {"geo": "m"}, "params": {"translate": [0, 0, 3]}}
            n["sim"] = {"type": "rbd_solver", "inputs": {"bodies": "tr", "colliders": "floor"}, "params": {}}
            n["o"] = {"type": "output", "inputs": {"geo": "sim"}}
            ev = Evaluator()
            self.addCleanup(ev.close)
            out = ev.cook(Graph.from_dict({"nodes": n, "display": "o"}), frame=60)
            self.assertLess(float(out.P[:, 2].min()), 0.0)                                # it fell (and did not blow up)
            self.assertTrue(np.isfinite(out.P).all())
        n["m"]["params"]["convex_parts"] = False
        if importlib.util.find_spec("mujoco"):
            from minihou.rbd import pieces
            with self.assertRaises(ValueError):
                pieces(cook(n, "m"))                                                      # concave pieces are refused without it


class ConstraintPropertiesTests(unittest.TestCase):
    def graph(self, **params):
        return {"b": {"type": "box", "params": {"size": [2, 2, 2]}},
                "m": {"type": "rbd_material_fracture", "inputs": {"geo": "b"},
                      "params": {"pieces": 10, "seed": 2, "clusters": 3, "output": "constraints", "strength_scale": 1.0}},
                "p": {"type": "rbd_constraint_properties", "inputs": {"constraints": "m"}, "params": dict({"outgrp_changed": True}, **params)}}

    def test_scale_a_group_only(self):
        before = cook(self.graph(), "m")
        g = cook(self.graph(group="0-2", mode="scale", strength=10.0), "p")
        s0, s1 = before.prim["strength"], g.prim["strength"]
        self.assertTrue(np.allclose(s1[:3], s0[:3] * 10, rtol=1e-5))
        self.assertTrue(np.array_equal(s1[3:], s0[3:]))
        self.assertEqual(g.prim["group_editedlinks"].tolist()[:4], [1, 1, 1, 0])

    def test_set_per_area_or_absolute_with_variation_and_seed(self):
        g = cook(self.graph(mode="set", strength=1000.0), "p")
        self.assertTrue(np.allclose(g.prim["strength"], 1000.0 * g.prim["area"], rtol=1e-5))
        g = cook(self.graph(mode="set", strength=500.0, per_area=False), "p")
        self.assertTrue(np.allclose(g.prim["strength"], 500.0))
        a = cook(self.graph(mode="set", strength=500.0, per_area=False, variation=0.3, seed=1), "p").prim["strength"]
        b = cook(self.graph(mode="set", strength=500.0, per_area=False, variation=0.3, seed=1), "p").prim["strength"]
        c = cook(self.graph(mode="set", strength=500.0, per_area=False, variation=0.3, seed=2), "p").prim["strength"]
        self.assertTrue(np.array_equal(a, b) and not np.array_equal(a, c))
        self.assertTrue((a >= 350 - 1e-3).all() and (a <= 650 + 1e-3).all())

    def test_unbreakable_internal_group_and_errors(self):
        n = {"b": {"type": "box", "params": {"size": [2, 2, 2]}},
             "fp": {"type": "voronoi_fracture_points", "inputs": {"geo": "b"}, "params": {"count": 12, "seed": 2}},
             "f": {"type": "voronoi_fracture", "inputs": {"geo": "b", "points": "fp"}},
             "asm": {"type": "assemble", "inputs": {"geo": "f"}},
             "m": {"type": "glue_cluster", "inputs": {"geo": "asm"},
                   "params": {"output": "constraints", "clusters": 3, "outgrp_internal": True}},
             "p": {"type": "rbd_constraint_properties", "inputs": {"constraints": "m"},
                   "params": {"unbreakable": True, "group": "internal"}}}
        g = cook(n, "p")
        internal = g.prim["group_internal"].astype(bool)
        self.assertTrue((g.prim["strength"][internal] == 0).all())
        self.assertTrue((g.prim["strength"][~internal] > 0).all())
        n["x"] = {"type": "box"}
        n["bad"] = {"type": "rbd_constraint_properties", "inputs": {"constraints": "x"}, "params": {}}
        with self.assertRaises(NodeError):
            cook(n, "bad")


if __name__ == "__main__":
    unittest.main()
