"""Output groups on the modelling nodes (Houdini's "Output Groups" section): switched on by checkbox, named by a field,
prim / point / edge domains, and usable by the nodes downstream."""
import unittest

import numpy as np

from minihou import Evaluator, Graph, NodeError, REGISTRY, edges, groups, outgroups, topo


def cook(nodes, out, **kw):
    nodes = dict(nodes)
    nodes["out"] = {"type": "output", "inputs": {"geo": out}}
    return Evaluator().cook(Graph.from_dict({"nodes": nodes, "display": "out"}), **kw)


def edge_group(geo, name):
    return edges.values(geo, edges.PREFIX + name) != 0


def edge_pairs(geo, name):
    return topo.edges(geo).points[edge_group(geo, name)]


BOX = {"b": {"type": "box", "params": {"size": [2, 2, 2]}}}
ALL_BEVEL = {f"outgrp_{k}": True for k in ("pointfilletpolys", "pointfilletedges", "edgefilletpolys", "offsetedges", "offsetpoints", "mergedpoints")}


def bevel(params=None, src=BOX, last="b"):
    n = dict(src)
    n["bv"] = {"type": "polybevel", "inputs": {"geo": last}, "params": dict({"distance": 0.2}, **(params or {}))}
    return cook(n, "bv")


class HelperTests(unittest.TestCase):
    def test_params_come_in_pairs_with_houdinis_labels_and_defaults(self):
        og = outgroups.OutGroups(outgroups.OutGroup("front", "Front", "extrudefront", "prim", default=True),
                                 outgroups.OutGroup("back", "Back", "extrudeback", "prim"))
        ps = og.params()
        self.assertEqual([p.name for p in ps], ["outgrp_front", "outgrp_front_name", "outgrp_back", "outgrp_back_name"])
        self.assertEqual([(p.kind, p.default, p.ui) for p in ps],
                         [("bool", True, "outgroup"), ("str", "extrudefront", "outgroup_name"), ("bool", False, "outgroup"), ("str", "extrudeback", "outgroup_name")])
        with self.assertRaises(ValueError):
            outgroups.OutGroups(outgroups.OutGroup("a", "A", "a", "weird"))
        with self.assertRaises(ValueError):
            outgroups.OutGroups(outgroups.OutGroup("a", "A", "a"), outgroups.OutGroup("a", "A2", "a2"))

    def test_off_empty_and_invalid_names(self):
        og = outgroups.OutGroups(outgroups.OutGroup("x", "X", "xs", "point"))
        g = cook(BOX, "b")
        self.assertIs(og.write(g, {}, "x", np.ones(8, bool)), g)                                       # off: nothing written
        self.assertIs(og.write(g, {"outgrp_x": True, "outgrp_x_name": "  "}, "x", np.ones(8, bool)), g)   # no name: nothing
        out = og.write(g, {"outgrp_x": True}, "x", np.arange(8) < 3)
        self.assertEqual(int(out.point["group_xs"].sum()), 3)                                          # the default name
        out = og.write(g, {"outgrp_x": True, "outgrp_x_name": "mine"}, "x", lambda: np.ones(8, bool))
        self.assertEqual(int(out.point["group_mine"].sum()), 8)
        with self.assertRaises(groups.GroupError):
            og.write(g, {"outgrp_x": True, "outgrp_x_name": "1bad name"}, "x", np.ones(8, bool))

    def test_edge_mask_matches_point_pairs_in_either_order(self):
        g = cook(BOX, "b")
        e = topo.edges(g)
        pairs = e.points[[1, 5, 7]][:, ::-1]                                                         # reversed on purpose
        m = outgroups.edge_mask(g, pairs)
        self.assertEqual(np.flatnonzero(m).tolist(), [1, 5, 7])
        self.assertFalse(outgroups.edge_mask(g, np.zeros((0, 2))).any())

    def test_every_node_that_has_output_groups_declares_them_as_pairs(self):
        found = {}
        for nt in REGISTRY:
            ps = [p for p in nt.params if p.name.startswith("outgrp_")]
            if ps:
                found[nt.name] = ps
                self.assertEqual(len(ps) % 2, 0, nt.name)
                for a, b in zip(ps[::2], ps[1::2]):
                    self.assertEqual((a.kind, b.kind, b.name), ("bool", "str", a.name + "_name"), nt.name)
        for name in ("polybevel", "polyextrude", "mirror", "boolean", "fuse", "clip", "sweep", "skin", "bridge", "polybridge"):
            self.assertIn(name, found)


class PolyBevelTests(unittest.TestCase):
    def test_nothing_is_made_unless_switched_on(self):
        g = bevel()
        self.assertEqual(outgroups.names_of(g), {"prim": ["bevel"], "point": [], "edge": []})         # only the older all-new-faces group

    def test_the_six_houdini_groups_on_a_box(self):
        g = bevel(ALL_BEVEL)
        self.assertEqual(outgroups.names_of(g), {
            "prim": ["bevel", "edgefilletpolys", "pointfilletpolys"],
            "point": ["mergedpoints", "offsetpoints"], "edge": ["offsetedges", "pointfilletedges"]})
        self.assertEqual((g.npoints, g.nprims), (24, 26))
        edge_polys = g.prim["group_edgefilletpolys"] != 0
        point_polys = g.prim["group_pointfilletpolys"] != 0
        self.assertEqual((int(edge_polys.sum()), int(point_polys.sum())), (12, 8))                    # a strip per edge, a corner per corner
        self.assertTrue(((edge_polys | point_polys) == (g.prim["group_bevel"] != 0)).all())           # together: every new face
        self.assertFalse((edge_polys & point_polys).any())
        sizes = g.topology.prim_sizes()
        self.assertTrue((sizes[edge_polys] == 4).all() and (sizes[point_polys] == 3).all())
        self.assertEqual(int(g.point["group_offsetpoints"].sum()), 24)                                # every wedge point of a box moved
        self.assertEqual(int(g.point["group_mergedpoints"].sum()), 0)                                 # this bevel never merges

    def test_offset_edges_join_a_strip_to_an_original_face_and_corner_edges_ring_a_corner(self):
        g = bevel(ALL_BEVEL)
        e = topo.edges(g)
        pv = topo.prim_of_vertex(g)
        strip = g.prim["group_edgefilletpolys"] != 0
        corner = g.prim["group_pointfilletpolys"] != 0
        original = ~(strip | corner)
        for name, a_mask, b_mask in (("offsetedges", strip, original), ("pointfilletedges", corner, strip)):
            sel = edge_group(g, name)
            self.assertEqual(int(sel.sum()), 24)
            for k in np.flatnonzero(sel):
                owners = set(pv[np.flatnonzero(e.of_vertex == k)].tolist())
                self.assertEqual(len(owners), 2, (name, k))
                self.assertTrue(any(a_mask[o] for o in owners) and any(b_mask[o] for o in owners), (name, k))   # one of each kind
        self.assertFalse((edge_group(g, "offsetedges") & edge_group(g, "pointfilletedges")).any())

    def test_custom_names_and_single_groups(self):
        g = bevel({"outgrp_offsetedges": True, "outgrp_offsetedges_name": "rim", "outgrp_offsetpoints": True, "output_group": ""})
        self.assertEqual(outgroups.names_of(g), {"prim": [], "point": ["offsetpoints"], "edge": ["rim"]})

    def test_groups_exist_but_are_empty_when_nothing_is_bevelled(self):
        g = bevel(dict(ALL_BEVEL, min_angle=179.0))                                                    # a box has no edge that sharp
        names = outgroups.names_of(g)
        self.assertEqual(names["prim"], ["edgefilletpolys", "pointfilletpolys"])
        self.assertEqual(names["edge"], ["offsetedges", "pointfilletedges"])
        self.assertFalse((g.prim["group_edgefilletpolys"] != 0).any() or edge_group(g, "offsetedges").any())

    def test_a_partial_bevel_marks_only_what_it_made(self):
        # one edge loop: the box's top face outline (an edge group of the top primitive)
        base = cook(BOX, "b")
        top = max(range(base.nprims), key=lambda p: base.P[base.topology.vertex_point[base.topology.prim_offsets[p]:base.topology.prim_offsets[p + 1]]][:, 2].mean())
        n = dict(BOX, eg={"type": "edge_group", "inputs": {"geo": "b"}, "params": {"name": "rim", "method": "primitives", "pattern": str(top)}})
        g = bevel(dict(ALL_BEVEL, group="rim", group_type="edges"), src=n, last="eg")
        self.assertEqual(int((g.prim["group_edgefilletpolys"] != 0).sum()), 4)                         # exactly the four top edges
        self.assertEqual(int((g.prim["group_pointfilletpolys"] != 0).sum()), 0)                        # no corner meets three bevelled edges
        self.assertEqual(int(edge_group(g, "offsetedges").sum()), 8)
        self.assertEqual(int(g.point["group_offsetpoints"].sum()), 8)
        h = bevel(dict(ALL_BEVEL, group=str(top), group_type="primitives"), src=BOX, last="b")         # the same through a primitive selection
        self.assertEqual(int((h.prim["group_edgefilletpolys"] != 0).sum()), 4)

    def test_the_groups_are_inputs_of_the_next_node_in_the_flow(self):
        n = dict(BOX)
        n["bv"] = {"type": "polybevel", "inputs": {"geo": "b"}, "params": dict({"distance": 0.15, "outgrp_edgefilletpolys": True, "outgrp_offsetedges": True})}
        n["ex"] = {"type": "polyextrude", "inputs": {"geo": "bv"}, "params": {"group": "edgefilletpolys", "distance": 0.1, "mode": "individual"}}
        g = cook(n, "ex")
        self.assertGreater(g.nprims, 26)                                                              # the strips were extruded
        n2 = dict(n)
        n2["cr"] = {"type": "crease", "inputs": {"geo": "bv"}, "params": {"domain": "edges", "group": "offsetedges", "sharpness": 3.0}}
        c = cook(n2, "cr")
        self.assertEqual(int((edges.values(c, edges.CREASE) == 3.0).sum()), 24)                       # an edge group drives an edge node
        n3 = dict(n)
        n3["again"] = {"type": "polybevel", "inputs": {"geo": "bv"}, "params": {"distance": 0.02, "group": "offsetedges", "group_type": "edges"}}
        g3 = cook(n3, "again")
        self.assertGreater(g3.nprims, 26)                                                              # bevelled the first bevel's offset edges

    def test_old_behaviour_is_unchanged_by_the_new_parameters(self):
        old = bevel({"output_group": "bevel"})
        self.assertEqual((old.npoints, old.nprims), (24, 26))
        pts = bevel({"group": "0 1 2 3", "group_type": "points"}, src=BOX)
        self.assertLess(pts.nprims, 26)                                                                # a point group still restricts it


class PolyExtrudeTests(unittest.TestCase):
    def extrude(self, params):
        n = dict(BOX)
        n["ex"] = {"type": "polyextrude", "inputs": {"geo": "b"}, "params": dict({"group": "0", "distance": 0.5}, **params)}
        return cook(n, "ex")

    def test_front_and_side_exist_as_before_and_back_follows_the_switch(self):
        g = self.extrude({})
        self.assertEqual(outgroups.names_of(g)["prim"], ["extrudeFront", "extrudeSide"])
        self.assertEqual((int(g.prim["group_extrudeFront"].sum()), int(g.prim["group_extrudeSide"].sum())), (1, 4))
        g = self.extrude({"output_back": True, "outgrp_back": True})
        self.assertEqual(int(g.prim["group_extrudeBack"].sum()), 1)
        self.assertEqual(g.nprims, 6 + 4 + 1)
        front, back = g.prim["group_extrudeFront"] != 0, g.prim["group_extrudeBack"] != 0
        n = g.topology.polys()
        P = g.P.astype(float)
        zf = P[g.topology.vertex_point[g.topology.prim_offsets[np.flatnonzero(front)[0]]:g.topology.prim_offsets[np.flatnonzero(front)[0] + 1]]][:, 2].mean()
        zb = P[g.topology.vertex_point[g.topology.prim_offsets[np.flatnonzero(back)[0]]:g.topology.prim_offsets[np.flatnonzero(back)[0] + 1]]][:, 2].mean()
        self.assertNotAlmostEqual(zf, zb, places=3)                                                   # front moved, back stayed

    def test_switching_off_and_renaming(self):
        g = self.extrude({"outgrp_front": False, "outgrp_side_name": "walls"})
        self.assertEqual(outgroups.names_of(g)["prim"], ["walls"])

    def test_an_empty_selection_still_makes_the_switched_on_groups(self):
        g = self.extrude({"group": "@P.z > 99"})
        self.assertEqual(outgroups.names_of(g)["prim"], ["extrudeFront", "extrudeSide"])
        self.assertFalse((g.prim["group_extrudeFront"] != 0).any())

    def test_individual_mode_gives_a_wall_per_edge(self):
        g = self.extrude({"mode": "individual", "group": "0 1"})
        self.assertEqual(int(g.prim["group_extrudeFront"].sum()), 2)
        self.assertEqual(int(g.prim["group_extrudeSide"].sum()), 8)


class OtherNodeTests(unittest.TestCase):
    def test_mirror(self):
        n = {"b": {"type": "box", "params": {"size": [2, 2, 2], "center": [1, 0, 0]}},
             "m": {"type": "mirror", "inputs": {"geo": "b"}, "params": {"outgrp_mirrored": True, "outgrp_mirroredpoints": True, "outgrp_seam": True}}}
        g = cook(n, "m")
        self.assertEqual((g.npoints, g.nprims), (12, 12))
        self.assertEqual(int(g.prim["group_mirrored"].sum()), 6)
        self.assertEqual(int(g.point["group_mirroredpoints"].sum()), 8)                                 # four copies and the four welded seam points
        seam = g.point["group_seampoints"] != 0
        self.assertEqual(int(seam.sum()), 4)
        np.testing.assert_allclose(g.P[seam][:, 0], 0.0, atol=1e-6)                                    # they lie on the plane
        n["m"]["params"]["keep_original"] = False
        h = cook(n, "m")
        self.assertEqual((int(h.prim["group_mirrored"].sum()), int(h.prim["group_mirrored"].size)), (6, 6))

    def test_boolean(self):
        n = {"a": {"type": "box", "params": {"size": [2, 2, 2]}}, "s": {"type": "sphere", "params": {"radius": 1.2, "rows": 12, "columns": 16}},
             "bo": {"type": "boolean", "inputs": {"a": "a", "b": "s"}, "params": {"operation": "subtract", "outgrp_seam": True, "outgrp_seampoints": True}}}
        g = cook(n, "bo")
        fa, fb = g.prim["group_from_a"] != 0, g.prim["group_from_b"] != 0
        self.assertTrue((fa ^ fb).all())                                                               # every face from exactly one input
        e = topo.edges(g)
        pv = topo.prim_of_vertex(g)
        seam = edge_group(g, "intersection")
        self.assertGreater(int(seam.sum()), 20)
        for k in np.flatnonzero(seam)[:60]:
            owners = pv[np.flatnonzero(e.of_vertex == k)]
            self.assertTrue(fa[owners].any() and fb[owners].any())                                     # an A face meets a B face along it
        # and no edge between an A and a B face is missing from the group
        both = np.bincount(e.of_vertex[(e.of_vertex >= 0) & fa[pv]], minlength=len(e.points)) * np.bincount(e.of_vertex[(e.of_vertex >= 0) & fb[pv]], minlength=len(e.points))
        self.assertTrue(((both > 0) == seam).all())
        pts = g.point["group_intersectionpoints"] != 0
        self.assertEqual(set(np.flatnonzero(pts).tolist()), set(np.unique(e.points[seam]).tolist()))

    def test_fuse(self):
        n = {"a": {"type": "box", "params": {"size": [1, 1, 1]}}, "b": {"type": "box", "params": {"size": [1, 1, 1], "center": [1, 0, 0]}},
             "m": {"type": "merge", "inputs": {"geos": ["a", "b"]}},
             "f": {"type": "fuse", "inputs": {"geo": "m"}, "params": {"outgrp_fused": True, "outgrp_unique": True}}}
        g = cook(n, "f")
        self.assertEqual(g.npoints, 12)
        self.assertEqual((int(g.point["group_fusedpoints"].sum()), int(g.point["group_uniquepoints"].sum())), (4, 8))

    def test_clip(self):
        n = {"a": {"type": "box", "params": {"size": [2, 2, 2]}},
             "c": {"type": "clip", "inputs": {"geo": "a"}, "params": {"keep": "both", "origin": [0, 0, 0.3], "outgrp_above": True, "outgrp_below": True,
                                                                        "outgrp_cutpolys": True, "outgrp_cutpoints": True, "outgrp_cutedges": True}}}
        g = cook(n, "c")
        above, below = g.prim["group_above"] != 0, g.prim["group_below"] != 0
        self.assertTrue((above ^ below).all())
        self.assertEqual(int((g.prim["group_cutpolys"] != 0).sum()), 8)                                # four sides, in two halves each
        self.assertEqual(int((g.point["group_cutpoints"] != 0).sum()), 4)
        ce = edge_pairs(g, "cutedges")
        self.assertEqual(len(ce), 4)
        np.testing.assert_allclose(g.P[ce.ravel()][:, 2], 0.3, atol=1e-5)                              # the cut edges lie in the plane

    def test_clip_defaults_leave_other_nodes_alone(self):
        n = {"a": {"type": "box", "params": {"size": [2, 2, 2]}}, "c": {"type": "clip", "inputs": {"geo": "a"}, "params": {"keep": "both"}}}
        self.assertIn("above", outgroups.names_of(cook(n, "c"))["prim"])                               # the group keep=both always had
        n["c"]["params"]["outgrp_above"] = False
        self.assertEqual(outgroups.names_of(cook(n, "c"))["prim"], [])

    def test_sweep_skin_and_bridges(self):
        n = {"l": {"type": "line", "params": {"length": 2, "points": 4}},
             "s": {"type": "sweep", "inputs": {"geo": "l"}, "params": {"divisions": 6, "outgrp_sides": True, "outgrp_startcap": True, "outgrp_endcap": True}}}
        g = cook(n, "s")
        self.assertEqual({k: int(g.prim[k].sum()) for k in ("group_sweepsides", "group_startcap", "group_endcap")},
                         {"group_sweepsides": 18, "group_startcap": 1, "group_endcap": 1})
        n = {"a": {"type": "circle", "params": {"radius": 1}}, "b": {"type": "circle", "params": {"radius": 1, "center": [0, 0, 2]}},
             "m": {"type": "merge", "inputs": {"geos": ["a", "b"]}},
             "k": {"type": "skin", "inputs": {"geo": "m"}, "params": {"cap": True, "outgrp_surface": True, "outgrp_cap": True}}}
        k = cook(n, "k")
        self.assertEqual((int(k.prim["group_skinsurface"].sum()), int(k.prim["group_skincap"].sum())), (32, 2))
        # bridge: two tubes' open ends
        t = {"t1": {"type": "tube", "params": {"caps": False, "height": 1, "center": [0, 0, 0]}},
             "t2": {"type": "tube", "params": {"caps": False, "height": 1, "center": [0, 0, 2]}},
             "m": {"type": "merge", "inputs": {"geos": ["t1", "t2"]}},
             "br": {"type": "bridge", "inputs": {"geo": "m"}, "params": {"outgrp_bridge": True}}}
        b = cook(t, "br")
        self.assertGreater(int(b.prim["group_bridgepolys"].sum()), 0)
        self.assertEqual(int(b.prim["group_bridgepolys"].sum()), b.nprims - 48)


if __name__ == "__main__":
    unittest.main()
