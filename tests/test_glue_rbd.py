"""glue_cluster, rbd_material_fracture, compound (cluster) bodies and breakable glue in the solver."""
import importlib.util
import unittest

import numpy as np

from minihou import Evaluator, Graph, NodeError, outgroups, topo
from minihou.nodes.fracture_tools import MATERIALS, mesh_volume


def cook(nodes, last, frame=None, ev=None):
    nodes = dict(nodes)
    nodes["o"] = {"type": "output", "inputs": {"geo": last}}
    g = Graph.from_dict({"nodes": nodes, "display": "o"})
    return (ev or Evaluator()).cook(g, frame=frame) if frame is not None else (ev or Evaluator()).cook(g)


def frac(count=8, **glue):
    return {"b": {"type": "box", "params": {"size": [2, 2, 2]}},
            "fp": {"type": "voronoi_fracture_points", "inputs": {"geo": "b"}, "params": {"count": count, "seed": 3}},
            "f": {"type": "voronoi_fracture", "inputs": {"geo": "b", "points": "fp"}},
            "asm": {"type": "assemble", "inputs": {"geo": "f"}},
            "g": {"type": "glue_cluster", "inputs": {"geo": "asm"}, "params": glue}}


class GlueClusterTests(unittest.TestCase):
    def test_constraints_are_one_link_per_touching_pair_with_strength_in_newtons(self):
        g = cook(frac(strength=1000.0, strength_variation=0.0, output="constraints"), "g")
        adj = cook(dict(frac(), a={"type": "voronoi_adjacency", "inputs": {"geo": "asm"}}), "a")
        self.assertEqual(g.nprims, adj.nprims)
        self.assertTrue(np.allclose(g.prim["strength"], 1000.0 * g.prim["area"], rtol=1e-4))
        self.assertTrue(np.allclose(g.prim["length"], np.sqrt(g.prim["area"]), rtol=1e-4))
        self.assertTrue(np.array_equal(g.prim["rbd_a"], g.prim["piece_a"]))                # no clustering: bodies are pieces
        self.assertEqual(g.npoints, 8)

    def test_variation_is_seeded_and_bounded(self):
        a = cook(frac(strength=1000.0, strength_variation=0.3, seed=1, output="constraints"), "g")
        b = cook(frac(strength=1000.0, strength_variation=0.3, seed=1, output="constraints"), "g")
        c = cook(frac(strength=1000.0, strength_variation=0.3, seed=2, output="constraints"), "g")
        self.assertTrue(np.array_equal(a.prim["strength"], b.prim["strength"]))
        self.assertFalse(np.array_equal(a.prim["strength"], c.prim["strength"]))
        ratio = a.prim["strength"] / (1000.0 * a.prim["area"])
        self.assertTrue((ratio >= 0.7 - 1e-3).all() and (ratio <= 1.3 + 1e-3).all())

    def test_zero_strength_never_breaks(self):
        g = cook(frac(strength=0.0, output="constraints"), "g")
        self.assertTrue((g.prim["strength"] == 0).all())

    def test_clusters_merge_pieces_and_internal_links_are_marked(self):
        n = frac(count=12, clusters=3, output="constraints", outgrp_internal=True, outgrp_external=True, outgrp_weak=True)
        g = cook(n, "g")
        self.assertEqual(sorted(set(g.point["cluster"].tolist())), [0, 1, 2])
        internal = g.prim["group_internal"].astype(bool)
        self.assertEqual(internal.tolist(), (g.prim["rbd_a"] == g.prim["rbd_b"]).tolist())
        self.assertTrue(np.all(g.prim["group_external"].astype(bool) == ~internal))
        self.assertTrue(internal.any() and (~internal).any())
        sizes = np.bincount(g.point["cluster"])
        self.assertLessEqual(int(sizes.max()), int(np.ceil(1.5 * 12 / 3)))                 # about even
        self.assertIn("weaklinks", outgroups.names_of(g)["prim"])

    def test_pieces_output_has_cluster_and_groups(self):
        n = frac(count=10, clusters=2, outgrp_glued=True, outgrp_loose=True, outgrp_interface=True)
        g = cook(n, "g")
        self.assertEqual(sorted(set(g.prim["cluster"].tolist())), [0, 1])
        glued, loose, face = (g.prim["group_" + k].astype(bool) for k in ("glued", "loose", "interface"))
        self.assertTrue(glued.all() and not loose.any())
        self.assertTrue(face.any() and not face.all())
        self.assertTrue(np.all(face <= glued))
        # a lone box has no neighbours: every face is loose
        lone = cook({"b": {"type": "box"}, "g": {"type": "glue_cluster", "inputs": {"geo": "b"}, "params": {"piece_attrib": "",
                                                                                                          "outgrp_loose": True,
                                                                                                          "outgrp_glued": True}}}, "g")
        self.assertTrue(lone.prim["group_loose"].all() and not lone.prim["group_glued"].any())

    def test_min_area_and_max_gap(self):
        everything = cook(frac(output="constraints"), "g").nprims
        self.assertEqual(cook(frac(output="constraints", min_area=1e9), "g").nprims, 0)
        gapped = frac()
        gapped["f"]["params"] = {"gap": 0.04}
        gapped["g"]["params"] = {"output": "constraints"}
        self.assertEqual(cook(gapped, "g").nprims, 0)
        gapped["g"]["params"] = {"output": "constraints", "max_gap": 0.05}
        self.assertEqual(cook(gapped, "g").nprims, everything)


class MaterialFractureTests(unittest.TestCase):
    def graph(self, **params):
        return {"b": {"type": "box", "params": {"size": [2, 2, 2]}},
                "m": {"type": "rbd_material_fracture", "inputs": {"geo": "b"}, "params": dict({"pieces": 8, "seed": 2}, **params)}}

    def test_pieces_carry_material_properties_and_mass(self):
        g = cook(self.graph(material="wood"), "m")
        density, friction, bounce, _ = MATERIALS["wood"]
        self.assertEqual(g.detail["rbd_material"], "wood")
        self.assertTrue(np.allclose(g.prim["rbd_friction"], friction))
        self.assertTrue(np.allclose(g.prim["rbd_restitution"], bounce))
        masses = {int(k): float(m) for k, m in zip(g.prim["rbd_id"], g.prim["rbd_mass"])}
        self.assertAlmostEqual(sum(masses.values()), density * 8.0, delta=1.0)             # density x the box's 8 m3
        self.assertEqual(len(masses), 8)
        self.assertTrue(np.all(topo.edges(g).count == 2))

    def test_mass_scale_and_materials_differ(self):
        wood = cook(self.graph(material="wood"), "m").prim["rbd_mass"].sum()
        metal = cook(self.graph(material="metal"), "m").prim["rbd_mass"].sum()
        double = cook(self.graph(material="wood", mass_scale=2.0), "m").prim["rbd_mass"].sum()
        self.assertGreater(metal, wood * 5)
        self.assertAlmostEqual(double / wood, 2.0, places=3)

    def test_clusters_become_compound_bodies_with_summed_mass(self):
        g = cook(self.graph(material="concrete", pieces=12, clusters=3), "m")
        self.assertEqual(len(set(g.prim["rbd_id"].tolist())), 3)
        self.assertTrue(g.prim["rbd_compound"].all())
        per_body = {int(k): float(m) for k, m in zip(g.prim["rbd_id"], g.prim["rbd_mass"])}
        self.assertAlmostEqual(sum(per_body.values()), 2400.0 * 8.0, delta=2.0)

    def test_constraints_output_matches_the_pieces_bodies(self):
        n = self.graph(material="concrete", pieces=12, clusters=3)
        n["c"] = {"type": "rbd_material_fracture", "inputs": {"geo": "b"}, "params": dict(n["m"]["params"], output="constraints")}
        pieces, links = cook(n, "m"), cook(n, "c")
        bodies = set(pieces.prim["rbd_id"].tolist())
        self.assertTrue(set(links.prim["rbd_a"].tolist()) | set(links.prim["rbd_b"].tolist()) <= bodies)
        self.assertGreater(links.nprims, 0)

    def test_groups_and_errors(self):
        g = cook(self.graph(outgrp_outside=True, outgrp_cutedges=True), "m")
        self.assertTrue(np.all(g.prim["group_inside"] + g.prim["group_outside"] == 1))
        bad = self.graph(glue=False, output="constraints")
        with self.assertRaises(NodeError):
            cook(bad, "m")


@unittest.skipUnless(importlib.util.find_spec("mujoco"), "optional physics extra not installed")
class SolverGlueTests(unittest.TestCase):
    def setUp(self):
        self.ev = Evaluator()
        self.addCleanup(self.ev.close)

    def cantilever(self, strength):
        return {
            "a": {"type": "box", "params": {"size": [1, 1, 1], "center": [0, 0, 0]}},
            "ra": {"type": "rbd_configure", "inputs": {"geo": "a"}, "params": {"mass": 100}},
            "b": {"type": "box", "params": {"size": [1, 1, 1], "center": [1, 0, 1]}},
            "rb": {"type": "rbd_configure", "inputs": {"geo": "b"}, "params": {"mass": 100, "id_offset": 1}},
            "m": {"type": "merge", "inputs": {"geos": ["ra", "rb"]}},
            "floor": {"type": "box", "params": {"size": [10, 10, 0.2], "center": [0, 0, -0.6]}},
            "link": {"type": "curve", "params": {"points": "0,0,0 1,0,1"}},
            "lc": {"type": "wrangle", "inputs": {"geo": "link"},
                   "params": {"run_over": "primitives", "snippet": f"i@rbd_a = 0; i@rbd_b = 1; f@strength = {strength}; f@length = 1.0;"}},
            "sim": {"type": "rbd_solver", "inputs": {"bodies": "m", "colliders": "floor", "constraints": "lc"}, "params": {}},
        }

    def pose(self, strength, frame):
        g = Graph.from_dict({"nodes": self.cantilever(strength), "display": "sim"})
        r = self.ev.cook(g, frame=frame)
        return r.P[8:16, 2].mean(), r.detail["rbd"]["glue"]

    def test_a_strong_glue_holds_a_block_in_the_air_and_a_weak_one_lets_go(self):
        z, glue = self.pose(1e9, 60)
        self.assertGreater(z, 0.9)
        self.assertEqual(glue["broken"], 0)
        z, glue = self.pose(100, 60)
        self.assertLess(z, 0.1)                                                           # fell to the floor beside the other block
        self.assertEqual(glue["broken_links"], [0])

    def test_no_strength_means_unbreakable_and_scrubbing_replays_breakage(self):
        z, glue = self.pose(0, 60)
        self.assertGreater(z, 0.9)
        g = Graph.from_dict({"nodes": self.cantilever(100), "display": "sim"})
        late = self.ev.cook(g, frame=40).P.copy()
        self.assertEqual(self.ev.cook(g, frame=1).detail["rbd"]["glue"]["broken"], 0)     # not broken yet at the start
        np.testing.assert_array_equal(late, self.ev.cook(g, frame=40).P)                  # same answer after going back

    def test_a_constraint_naming_an_unknown_body_is_an_error(self):
        n = self.cantilever(100)
        n["lc"]["params"]["snippet"] = "i@rbd_a = 0; i@rbd_b = 7; f@strength = 5.0;"
        with self.assertRaises(NodeError):
            self.ev.cook(Graph.from_dict({"nodes": n, "display": "sim"}), frame=3)

    def test_compound_cluster_is_one_rigid_body(self):
        def graph(compound):
            return Graph.from_dict({"nodes": {
                "a": {"type": "box", "params": {"size": [1, 1, 1], "center": [0, 0, 3]}},
                "b": {"type": "box", "params": {"size": [1, 1, 1], "center": [0.2, 0, 5]}},      # not touching: two separate pieces
                "m": {"type": "merge", "inputs": {"geos": ["a", "b"]}},
                "one": {"type": "attribute_create", "inputs": {"geo": "m"},
                        "params": {"name": "cluster", "domain": "primitives", "type": "int", "value": [0, 0, 0]}},
                "r": {"type": "rbd_configure", "inputs": {"geo": "one"}, "params": {"piece_attrib": "cluster", "compound": compound,
                                                                                    "mass": 10}},
                "floor": {"type": "box", "params": {"size": [20, 20, 1], "center": [0, 0, -0.5]}},
                "sim": {"type": "rbd_solver", "inputs": {"bodies": "r", "colliders": "floor"}, "params": {}}}, "display": "sim"})
        start, end = self.ev.cook(graph(True), frame=1), self.ev.cook(graph(True), frame=120)
        a0, b0, a1, b1 = start.P[:8], start.P[8:16], end.P[:8], end.P[8:16]
        self.assertGreater(float(a0[:, 2].min()- a1[:, 2].min()), 2.4)                   # the cluster fell...
        self.assertAlmostEqual(float(b1[:, 2].min() - a1[:, 2].min()), 2.0, delta=0.02)   # ...and the upper block stayed 2 m up on it
        np.testing.assert_allclose(np.linalg.norm(a1[:, None] - b1, axis=2), np.linalg.norm(a0[:, None] - b0, axis=2), atol=1e-4)
        with self.assertRaises(NodeError):                                                 # one id for two pieces, without 'compound'
            self.ev.cook(graph(False), frame=2)

    def test_glued_wall_pipeline_cooks_and_falls_apart_when_the_glue_is_weak(self):
        def pipeline(material_strength):
            return {
                "b": {"type": "box", "params": {"size": [2, 0.6, 1.2], "center": [0, 0, 1.0]}},
                "m": {"type": "rbd_material_fracture", "inputs": {"geo": "b"},
                      "params": {"pieces": 6, "seed": 1, "material": "concrete", "strength_scale": material_strength}},
                "c": {"type": "rbd_material_fracture", "inputs": {"geo": "b"},
                      "params": {"pieces": 6, "seed": 1, "material": "concrete", "strength_scale": material_strength,
                                 "output": "constraints"}},
                "floor": {"type": "box", "params": {"size": [20, 20, 1], "center": [0, 0, -0.5]}},
                "sim": {"type": "rbd_solver", "inputs": {"bodies": "m", "colliders": "floor", "constraints": "c"}, "params": {}},
            }
        weak = self.ev.cook(Graph.from_dict({"nodes": pipeline(0.0001), "display": "sim"}), frame=45)
        self.assertGreater(weak.detail["rbd"]["glue"]["broken"], 0)
        strong = self.ev.cook(Graph.from_dict({"nodes": pipeline(0.0), "display": "sim"}), frame=45)   # 0: never breaks
        self.assertEqual(strong.detail["rbd"]["glue"]["broken"], 0)
        self.assertEqual(len(weak.detail["rbd"].keys() & {"glue"}), 1)


if __name__ == "__main__":
    unittest.main()
