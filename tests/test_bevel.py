"""tube, torus and polybevel: closed, consistently wound shells with the volumes geometry predicts."""
import unittest

import numpy as np

from minihou import Evaluator, Graph, NodeError
from minihou.nodes._util import fan_triangulate
from minihou.nodes.bevel import twins


def cook(nodes, display):
    nodes = dict(nodes)
    nodes["out"] = {"type": "output", "inputs": {"geo": display}}
    g = Evaluator().cook(Graph.from_dict({"nodes": nodes, "display": "out"}))
    g.validate()
    return g


def volume(g):
    tris, _ = fan_triangulate(g.topology)
    T = g.P[g.topology.vertex_point[tris]].astype(np.float64)
    return float(np.einsum("ij,ij->i", T[:, 0], np.cross(T[:, 1], T[:, 2])).sum() / 6.0)


def closed_shell(test, g):
    test.assertTrue(np.all(twins(g) >= 0), "every edge shared by exactly two faces, wound oppositely")


def bevel(src, **params):
    return cook({"s": src, "b": {"type": "polybevel", "inputs": {"geo": "s"}, "params": params}}, "b")


class PrimitiveTests(unittest.TestCase):
    def test_tube_and_cone(self):
        g = cook({"t": {"type": "tube", "params": {"columns": 24, "rows": 3, "height": 2}}}, "t")
        closed_shell(self, g)
        self.assertAlmostEqual(volume(g), 0.5 * 24 * np.sin(2 * np.pi / 24) * 2, places=5)     # outward-facing
        self.assertEqual(g.vertex["uv"].shape, (g.topology.nvertices, 2))
        cone = cook({"t": {"type": "tube", "params": {"radius_top": 0, "columns": 16, "rows": 2}}}, "t")
        closed_shell(self, cone)
        self.assertEqual(int((cone.P[:, 2] > 0.999).sum()), 1)                          # one tip point
        self.assertAlmostEqual(volume(cone), 0.5 * 16 * np.sin(2 * np.pi / 16) * 2 / 3, places=5)
        open_tube = cook({"t": {"type": "tube", "params": {"caps": False}}}, "t")
        self.assertEqual(int((twins(open_tube) < 0).sum()), 2 * 24)

    def test_torus(self):
        g = cook({"t": {"type": "torus", "params": {"radius": 2, "thickness": 0.5, "rows": 64, "columns": 128}}}, "t")
        closed_shell(self, g)
        self.assertAlmostEqual(volume(g), 2 * np.pi ** 2 * 2 * 0.25, delta=0.02)         # 2 pi^2 R r^2
        u = g.vertex["uv"]
        self.assertTrue(u.min() >= 0 and u.max() <= 1)


class SegmentedBevelTests(unittest.TestCase):
    def test_round_divisions_make_closed_shell_and_preserve_attributes(self):
        counts=[]
        for divisions in (1,2,4,8):
            g=bevel({'type':'box'},distance=.1,divisions=divisions,outgrp_edgefilletpolys=True,outgrp_pointfilletpolys=True,outgrp_offsetedges=True,outgrp_pointfilletedges=True,outgrp_offsetpoints=True,outgrp_mergedpoints=True)
            closed_shell(self,g)
            self.assertTrue(np.isfinite(g.P).all())
            self.assertGreater(volume(g),.94)
            self.assertLess(volume(g),1.)
            if 'uv' in g.vertex:
                self.assertEqual(g.vertex['uv'].shape,(g.nvertices,2))
            from minihou import groups,edges
            self.assertEqual(int(groups.mask(g,'prim','edgefilletpolys').sum()),12*divisions)
            self.assertEqual(int(groups.mask(g,'point','offsetpoints').sum()),24)
            self.assertGreater(int(edges.mask(g,'pointfilletedges').sum()),0)
            self.assertFalse(groups.mask(g,'point','mergedpoints').any())
            counts.append(g.nprims)
        self.assertEqual(counts,sorted(set(counts)))

    def test_flat_profile_subdivision_preserves_chamfer_volume(self):
        src={'type':'box'}
        one=bevel(src,distance=.1)
        flat=bevel(src,distance=.1,divisions=4,shape='flat')
        zero=bevel(src,distance=.1,divisions=4,convexity=0)
        self.assertAlmostEqual(volume(one),volume(flat),places=6)
        np.testing.assert_allclose(flat.P,zero.P)
        closed_shell(self,flat)

    def test_subdivided_box_and_tube_remain_closed(self):
        for src in ({'type':'box','params':{'divisions':3}}, {'type':'tube','params':{'columns':12}}):
            g=bevel(src,distance=.04,divisions=4)
            closed_shell(self,g)
            self.assertGreater(volume(g),0)


class BevelTests(unittest.TestCase):
    def test_box_volume_and_counts(self):
        d = 0.1
        g = bevel({"type": "box"}, distance=d)
        closed_shell(self, g)
        self.assertEqual((g.npoints, g.nprims), (24, 6 + 12 + 8))
        # edges lose a prism d^2/2 x (1 - 2d) each; corners lose 5/6 d^3 (inclusion-exclusion over the corner cube)
        self.assertAlmostEqual(volume(g), 1 - 12 * d * d / 2 * (1 - 2 * d) - 8 * 5 / 6 * d ** 3, places=5)
        self.assertEqual(int(g.prim["group_bevel"].sum()), 20)

    def test_subdivided_box_only_bevels_sharp_edges(self):
        plain = bevel({"type": "box"}, distance=0.05)
        sub = bevel({"type": "box", "params": {"divisions": 3}}, distance=0.05)
        closed_shell(self, sub)
        self.assertEqual(sub.nprims, 54 + 36 + 8)
        self.assertAlmostEqual(volume(sub), volume(plain), places=5)
        every = bevel({"type": "box", "params": {"divisions": 3}}, distance=0.05, min_angle=0)
        self.assertEqual(every.nprims, 54 + 108 + 56)
        self.assertAlmostEqual(volume(every), volume(plain), places=5)                 # flat bevels keep volume

    def test_tube_rims(self):
        d = 0.05
        g = bevel({"type": "tube", "params": {"columns": 24, "rows": 2}}, distance=d)
        closed_shell(self, g)
        full = 0.5 * 24 * np.sin(2 * np.pi / 24) * 2
        rim = 24 * 2 * np.sin(np.pi / 24)
        self.assertAlmostEqual(full - volume(g), 2 * rim * d * d / 2, delta=0.1 * 2 * rim * d * d / 2)

    def test_concave_edges_and_open_meshes(self):
        nodes = {"b": {"type": "box", "params": {"divisions": 2}},
                 "e": {"type": "polyextrude", "inputs": {"geo": "b"}, "params": {"group": "0", "distance": 0.5}}}
        before = cook(nodes, "e")
        g = cook({**nodes, "v": {"type": "polybevel", "inputs": {"geo": "e"}, "params": {"distance": 0.05}}}, "v")
        closed_shell(self, g)
        self.assertNotAlmostEqual(volume(g), volume(before), places=4)
        # a bent open grid: only the fold is bevelled, borders stay borders
        grid = {"g": {"type": "grid", "params": {"columns": 5, "rows": 3, "size_x": 4, "size_y": 2}},
                "w": {"type": "wrangle", "inputs": {"geo": "g"}, "params": {"snippet": "@P.z = abs(@P.x)"}}}
        before = cook(grid, "w")
        g = cook({**grid, "v": {"type": "polybevel", "inputs": {"geo": "w"}, "params": {"distance": 0.2}}}, "v")
        self.assertEqual(g.nprims, before.nprims + 2)                  # the fold's two edges become quads
        self.assertEqual(int((twins(g) < 0).sum()), int((twins(before) < 0).sum()) + 2)
        self.assertTrue(np.all(np.abs(g.P[:, 1]) <= 1 + 1e-6))         # border points slid along the border

    def test_point_group_and_errors(self):
        g = bevel({"type": "box"}, distance=0.1, group="@P.z > 0")
        closed_shell(self, g)
        self.assertEqual(int(g.prim["group_bevel"].sum()), 4)          # the top face's four edges, no corners
        self.assertIs(bevel({"type": "box"}, distance=0.0).nprims, 6)
        with self.assertRaises(NodeError):
            cook({"c": {"type": "helix"}, "b": {"type": "polybevel", "inputs": {"geo": "c"}}}, "b")


class EdgeSelectionTests(unittest.TestCase):
    """polybevel `edges`: exactly the listed edges (point pairs), the way the 3-D view's selection writes them."""

    def box(self):
        return Evaluator().cook(Graph.from_dict({"nodes": {"b": {"type": "box", "params": {"size": [2, 2, 2]}}},
                                                 "display": "b"}))

    def edges_of_prim(self, geo, prim):
        vp, offs = geo.topology.vertex_point.astype(int), geo.topology.prim_offsets
        v = vp[offs[prim]:offs[prim + 1]]
        return [(min(a, b), max(a, b)) for a, b in zip(v, np.roll(v, -1))]

    def bevel(self, geo, edges, **p):
        from minihou import REGISTRY
        from minihou.groups import format_edges
        nt = REGISTRY.get("polybevel")
        return nt.fn(geo, **nt.coerce_params(dict(edges=format_edges(edges), **p)))

    def test_a_face_ring_is_bevelled_and_the_rest_is_left_alone(self):
        g = self.box()
        out = self.bevel(g, self.edges_of_prim(g, 0), distance=0.2)
        self.assertEqual(out.nprims, 6 + 4)                                  # four bevel quads, nothing else
        closed_shell(self, out)
        self.assertLess(volume(out), volume(g))
        every = Evaluator().cook(Graph.from_dict({"nodes": {"b": {"type": "box", "params": {"size": [2, 2, 2]}},
                                                            "v": {"type": "polybevel", "inputs": {"geo": "b"}}},
                                                  "display": "v"}))
        self.assertGreater(every.nprims, out.nprims)                         # the angle mode bevels all twelve edges

    def test_the_selection_ignores_the_angle_and_flat_edges_are_allowed(self):
        from minihou.groups import parse_edges
        g = Evaluator().cook(Graph.from_dict({"nodes": {"b": {"type": "box", "params": {"size": [2, 2, 2], "divisions": 2}}},
                                              "display": "b"}))
        ring = self.edges_of_prim(g, 0)
        out = self.bevel(g, ring, distance=0.05, min_angle=179.0)            # the angle would exclude everything
        self.assertGreater(out.nprims, g.nprims)
        self.assertEqual(len(parse_edges("3-1 1-3 2-2 5-4", 10)), 2)         # sorted, deduplicated, no self edges

    def test_single_box_edge_terminates_without_cracks(self):
        from minihou import topo
        for edge in topo.edges(self.box()).points:
            for divisions in (1,4):
                g=self.bevel(self.box(),[tuple(edge)],distance=.1,divisions=divisions)
                closed_shell(self,g)
                self.assertEqual(len(np.unique(g.topology.vertex_point)),g.npoints)
                self.assertGreater(volume(g),0)
                self.assertTrue(np.isfinite(g.P).all())

    def test_clear_errors(self):
        g = self.box()
        for bad, msg in (("1-", "bad edge"), ("a-b", "bad edge"), ("0-99", "only 8 points")):
            with self.assertRaisesRegex(ValueError, msg):
                from minihou import REGISTRY
                REGISTRY.get("polybevel").fn(g, **REGISTRY.get("polybevel").coerce_params({"edges": bad}))
        same = self.bevel(g, [], distance=0.1)                               # an empty selection means the angle mode
        self.assertGreater(same.nprims, 6)


class VertexNormalTests(unittest.TestCase):
    def test_cusp_angle(self):
        box = cook({"b": {"type": "box"}, "n": {"type": "normal", "inputs": {"geo": "b"}, "params": {"domain": "vertex"}}},
                   "n")
        from minihou.nodes.normal import prim_normals
        from minihou.nodes._util import normalize_rows
        face = normalize_rows(prim_normals(box))[np.repeat(np.arange(6), 4)]
        np.testing.assert_allclose(box.vertex["N"], face, atol=1e-6)          # 90 degrees > 60: flat faces
        self.assertNotIn("N", box.point)
        nodes = {"s": {"type": "sphere"}, "p": {"type": "normal", "inputs": {"geo": "s"}},
                 "v": {"type": "normal", "inputs": {"geo": "s"}, "params": {"domain": "vertex", "cusp_angle": 60}}}
        point, vert = cook(nodes, "p"), cook(nodes, "v")
        np.testing.assert_allclose(vert.vertex["N"], point.point["N"][vert.topology.vertex_point], atol=1e-5)
        bev = cook({"b": {"type": "box"}, "v": {"type": "polybevel", "inputs": {"geo": "b"}},
                    "n": {"type": "normal", "inputs": {"geo": "v"}, "params": {"domain": "vertex", "cusp_angle": 30}}}, "n")
        f = normalize_rows(prim_normals(bev))[np.repeat(np.arange(bev.nprims), bev.topology.prim_sizes())]
        np.testing.assert_allclose(bev.vertex["N"], f, atol=1e-6)            # 45 degree chamfers stay crisp


if __name__ == "__main__":
    unittest.main()
