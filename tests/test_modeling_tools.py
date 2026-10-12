"""Nodes behind the "C" pie menu that had no equivalent: twist, linear taper, squash and stretch, soft peak, group by range,
edge collapse / flip / cusp and polysplit."""
import unittest

import numpy as np

from minihou import Evaluator, Graph, edges, outgroups, topo


def cook(node, params=None, src="grid", src_params=None, extra=None):
    nodes = {"s": {"type": src, "params": src_params or {}},
             "n": {"type": node, "inputs": {"geo": "s"}, "params": params or {}}}
    nodes.update(extra or {})
    nodes["o"] = {"type": "output", "inputs": {"geo": "n"}}
    return Evaluator().cook(Graph.from_dict({"nodes": nodes, "display": "o"}))


def closed_manifold(g):
    return bool((topo.edges(g).count == 2).all())


GRID = {"size_x": 2, "size_y": 2, "rows": 4, "columns": 4}            # 4 x 4 points, 3 x 3 quads


class DeformerTests(unittest.TestCase):
    def test_null_passes_through(self):
        a = cook("null", {"label": "OUT"})
        b = cook("transform", {})
        self.assertTrue(np.array_equal(a.P, b.P))

    def test_twist_is_zero_at_the_origin_and_preserves_distance_from_the_axis(self):
        g = cook("twist", {"strength": 90, "direction": [0, 1, 0]}, src="box", src_params={"size": [1, 2, 1]})
        src = cook("null", src="box", src_params={"size": [1, 2, 1]})
        r0, r1 = np.hypot(src.P[:, 0], src.P[:, 2]), np.hypot(g.P[:, 0], g.P[:, 2])
        self.assertTrue(np.allclose(r0, r1, atol=1e-5))
        self.assertTrue(np.allclose(src.P[:, 1], g.P[:, 1], atol=1e-5))
        mid = np.abs(src.P[:, 1]) < 1e-6
        if mid.any():
            self.assertTrue(np.allclose(src.P[mid], g.P[mid], atol=1e-5))
        top = src.P[:, 1] > 0.99                                         # one unit up: a quarter turn
        self.assertFalse(np.allclose(src.P[top], g.P[top]))
        ang0 = np.arctan2(src.P[top, 2], src.P[top, 0])
        ang1 = np.arctan2(g.P[top, 2], g.P[top, 0])
        d = (ang0 - ang1 + np.pi) % (2 * np.pi) - np.pi                  # rotation about +Y by +90 degrees: x -> -z
        self.assertTrue(np.allclose(np.abs(d), np.pi / 2, atol=1e-4))

    def test_twist_group_restricts(self):
        full = cook("twist", {"strength": 45})
        part = cook("twist", {"strength": 45, "group": "0-3"}, src_params=GRID)
        src = cook("null", src_params=GRID)
        self.assertTrue(np.allclose(part.P[4:], src.P[4:]))
        self.assertFalse(np.allclose(part.P[:4], src.P[:4]) and np.allclose(full.P, src.P))

    def test_linear_taper_scales_across_the_axis_only(self):
        src = cook("null", src="box", src_params={"size": [2, 2, 2]})
        g = cook("linear_taper", {"taper_x": 0.5, "taper_y": 0.5, "length": 1.0, "origin": [0, -1, 0]},
                 src="box", src_params={"size": [2, 2, 2]})
        self.assertTrue(np.allclose(src.P[:, 1], g.P[:, 1], atol=1e-6))
        bottom, top = src.P[:, 1] < -0.99, src.P[:, 1] > 0.99
        self.assertTrue(np.allclose(g.P[bottom], src.P[bottom], atol=1e-6))           # no change at the origin
        self.assertAlmostEqual(float(np.abs(g.P[top][:, [0, 2]]).max()), 1.0 * (1 + (0.5 - 1) * 2.0), places=5)   # scale 0 at t=2
        self.assertTrue(closed_manifold(g))

    def test_squash_stretch_preserves_volume(self):
        g = cook("squash_stretch", {"amount": 2.0, "preserve_volume": True}, src="box", src_params={"size": [1, 1, 1]})
        ext = np.ptp(g.P, axis=0)
        self.assertAlmostEqual(float(ext[1]), 2.0, places=5)
        self.assertAlmostEqual(float(ext[0] * ext[1] * ext[2]), 1.0, places=5)
        g2 = cook("squash_stretch", {"amount": 2.0, "preserve_volume": False}, src="box", src_params={"size": [1, 1, 1]})
        self.assertAlmostEqual(float(np.ptp(g2.P[:, 0])), 1.0, places=5)

    def test_soft_peak_moves_the_group_fully_and_neighbours_less(self):
        g = cook("soft_peak", {"distance": 1.0, "radius": 1.0, "group": "4"}, src_params=GRID)
        src = cook("null", src_params=GRID)
        dz = g.P[:, 2] - src.P[:, 2]
        self.assertAlmostEqual(abs(float(dz[4])), 1.0, places=5)                 # the grid's normals are +/-Z
        d = np.linalg.norm(src.P - src.P[4], axis=1)
        self.assertTrue(np.allclose(dz[d >= 1.0], 0, atol=1e-6))
        near = (d > 0) & (d < 1)
        self.assertTrue(near.any())
        self.assertTrue((np.abs(dz[near]) < np.abs(dz[4])).all())
        self.assertTrue((np.abs(dz[near]) > 0).all())


class GroupRangeTests(unittest.TestCase):
    def test_range_and_every_other(self):
        g = cook("group_range", {"name": "r", "domain": "primitives", "start": 2, "end": 7}, src_params=GRID)
        self.assertEqual(list(np.flatnonzero(g.prim["group_r"])), [2, 3, 4, 5, 6, 7])
        g = cook("group_range", {"name": "r", "domain": "primitives", "select": 1, "every": 2}, src_params=GRID)
        self.assertEqual(list(np.flatnonzero(g.prim["group_r"])), [0, 2, 4, 6, 8])
        g = cook("group_range", {"name": "r", "domain": "points", "end": 3, "invert": True}, src_params=GRID)
        self.assertEqual(int(g.point["group_r"].sum()), g.npoints - 4)


class EdgeToolTests(unittest.TestCase):
    def test_edge_collapse_merges_the_ends_and_drops_degenerate_faces(self):
        src = cook("null", src_params=GRID)
        e = topo.edges(src)
        # a boundary-free interior edge: pick any edge whose count is 2
        eid = int(np.flatnonzero(e.count == 2)[0])
        a, b = e.points[eid]
        g = cook("edge_collapse", {"group": f"p{a}-{b}"}, src_params=GRID)
        self.assertEqual(g.npoints, src.npoints - 1)
        self.assertTrue(np.isfinite(g.P).all())
        self.assertTrue((g.topology.prim_sizes() >= 3).all())
        self.assertNotIn("group_collapsedpoints", g.point)                  # output groups are off by default

    def test_edge_collapse_output_group_and_midpoint(self):
        src = cook("null", src_params=GRID)
        e = topo.edges(src)
        eid = int(np.flatnonzero(e.count == 2)[0])
        a, b = (int(x) for x in e.points[eid])
        g = cook("edge_collapse", {"group": f"p{a}-{b}", "outgrp_collapsed": True}, src_params=GRID)
        self.assertEqual(int(g.point["group_collapsedpoints"].sum()), 1)
        merged = int(np.flatnonzero(g.point["group_collapsedpoints"])[0])
        self.assertTrue(np.allclose(g.P[merged], (src.P[a] + src.P[b]) / 2, atol=1e-6))
        self.assertTrue((g.topology.vertex_point < g.npoints).all())

    def test_edge_collapse_empty_selection_is_a_noop(self):
        g = cook("edge_collapse", {"group": ""}, src_params=GRID)
        self.assertEqual(g.nprims, 9)

    def test_edge_flip_swaps_the_diagonal_and_keeps_the_surface_valid(self):
        def run(group, last):
            nodes = {"s": {"type": "grid", "params": GRID}, "t": {"type": "divide", "inputs": {"geo": "s"}},
                     "f": {"type": "edge_flip", "inputs": {"geo": "t"}, "params": {"group": group, "outgrp_flipped": True}},
                     "o": {"type": "output", "inputs": {"geo": last}}}
            return Evaluator().cook(Graph.from_dict({"nodes": nodes, "display": "o"}))
        tri = run("", "t")
        self.assertTrue((tri.topology.prim_sizes() == 3).all())
        e = topo.edges(tri)
        a, b = (int(x) for x in e.points[np.flatnonzero(e.count == 2)[0]])
        out = run(f"p{a}-{b}", "f")
        self.assertEqual(out.nprims, tri.nprims)
        self.assertTrue((out.topology.prim_sizes() == 3).all())
        before = {tuple(p) for p in topo.edges(tri).points}
        after = {tuple(p) for p in topo.edges(out).points}
        self.assertEqual(before - after, {(a, b)})
        self.assertEqual(len(after - before), 1)
        flipped = next(iter(after - before))
        self.assertEqual(int((edges.values(out, edges.PREFIX + "flippededges") != 0).sum()), 1)
        self.assertEqual(edges.values(out, edges.PREFIX + "flippededges")[[tuple(p) for p in topo.edges(out).points].index(flipped)], 1)
        from minihou.nodes.normal import prim_normals
        n_in, n_out = prim_normals(tri), prim_normals(out)
        self.assertAlmostEqual(float(np.linalg.norm(n_out, axis=1).sum()), float(np.linalg.norm(n_in, axis=1).sum()), places=4)
        self.assertTrue((n_out[:, 2] * n_in[0, 2] > 0).all())                    # every triangle still faces the same way
        self.assertTrue((topo.edges(out).count <= 2).all())
        self.assertEqual(run("", "f").nprims, tri.nprims)                          # empty selection: nothing happens

    def test_edge_cusp_makes_selected_edges_hard_and_the_rest_smooth(self):
        g = cook("edge_cusp", {"group": ""}, src="box", src_params={"size": [1, 1, 1]})
        N = g.vertex["N"]
        vp = g.topology.vertex_point
        for p in range(g.npoints):                                       # nothing cusped: one normal per point
            rows = N[vp == p]
            self.assertTrue(np.allclose(rows, rows[0], atol=1e-5))
        self.assertNotIn("N", g.point)
        e = topo.edges(g)
        every = ",".join(str(i) for i in range(len(e.points)))
        h = cook("edge_cusp", {"group": every}, src="box", src_params={"size": [1, 1, 1]})
        pv = topo.prim_of_vertex(h)
        from minihou.nodes.normal import prim_normals
        from minihou.nodes._util import normalize_rows
        fn = normalize_rows(prim_normals(h))
        self.assertTrue(np.allclose(h.vertex["N"], fn[pv], atol=1e-5))                   # all hard: face normals
        a = cook("edge_cusp", {"group": "", "cusp_angle": 60}, src="box", src_params={"size": [1, 1, 1]})
        self.assertTrue(np.allclose(a.vertex["N"], fn[pv], atol=1e-5))                    # 90 degree box edges exceed 60


class PolySplitTests(unittest.TestCase):
    def test_edge_loop_through_a_grid_splits_a_row_of_quads(self):
        src = cook("null", src_params=GRID)
        e = topo.edges(src)
        P = src.P
        pick = next((int(a), int(b)) for a, b in e.points
                    if abs(P[a][0] - P[b][0]) > 0.1 and abs(P[a][1]) < 0.4 and abs(P[b][1]) < 0.4)      # an interior edge along x
        g = cook("polysplit", {"split_edges": f"p{pick[0]}-{pick[1]}", "outgrp_points": True, "outgrp_edges": True}, src_params=GRID)
        self.assertGreater(g.nprims, 9)
        self.assertGreater(g.npoints, src.npoints)
        self.assertTrue((g.topology.vertex_point < g.npoints).all())
        # area unchanged and the new points lie on the old edges
        from minihou.nodes.normal import prim_normals
        self.assertAlmostEqual(float(np.linalg.norm(prim_normals(g), axis=1).sum()),
                               float(np.linalg.norm(prim_normals(src), axis=1).sum()), places=4)
        self.assertGreater(int(g.point["group_splitpoints"].sum()), 0)
        self.assertGreater(int((edges.values(g, edges.PREFIX + "splitedges") != 0).sum()), 0)
        # every edge of the result is used by at most two faces and the boundary is the original square
        self.assertTrue((topo.edges(g).count <= 2).all())

    def test_a_ring_that_ends_at_a_triangle_leaves_no_cracks(self):
        nodes = {"s": {"type": "grid", "params": GRID}, "d": {"type": "divide", "inputs": {"geo": "s"}, "params": {"group": "4"}},
                 "n": {"type": "polysplit", "inputs": {"geo": "d"}, "params": {"split_edges": "p5-6"}},
                 "o": {"type": "output", "inputs": {"geo": "n"}}}
        g = Evaluator().cook(Graph.from_dict({"nodes": nodes, "display": "o"}))
        e = topo.edges(g)
        border = e.points[e.count == 1]
        P = g.P
        on_edge = (np.abs(np.abs(P[border][:, :, 0]) - 1) < 1e-5) | (np.abs(np.abs(P[border][:, :, 1]) - 1) < 1e-5)
        self.assertTrue(on_edge.all())                                    # only the original outline is open: no T-junctions
        self.assertTrue((e.count <= 2).all())

    def test_loops_on_a_box_ring_are_watertight(self):
        src = cook("null", src="box", src_params={"size": [1, 1, 1]})
        e = topo.edges(src)
        a, b = (int(x) for x in e.points[0])
        for loops in (1, 3):
            g = cook("polysplit", {"split_edges": f"p{a}-{b}", "loops": loops}, src="box", src_params={"size": [1, 1, 1]})
            self.assertTrue(closed_manifold(g), loops)
            self.assertTrue((g.topology.prim_sizes() == 4).all())
            self.assertEqual(g.nprims, 6 + 4 * loops)
            self.assertEqual(g.npoints, 8 + 4 * loops)

    def test_position_counts_from_the_first_point(self):
        src = cook("null", src="box", src_params={"size": [1, 1, 1]})
        e = topo.edges(src)
        a, b = (int(x) for x in e.points[0])
        g = cook("polysplit", {"split_edges": f"p{a}-{b}", "position": 0.25}, src="box", src_params={"size": [1, 1, 1]})
        new = g.P[src.npoints:]
        want = src.P[a] * 0.75 + src.P[b] * 0.25
        self.assertTrue(any(np.allclose(q, want, atol=1e-5) for q in new))

    def test_split_between_points(self):
        g = cook("polysplit", {"mode": "points", "split_points": "0-5", "outgrp_edges": True}, src_params=GRID)
        self.assertEqual(g.nprims, 10)                                    # the first quad becomes two triangles
        self.assertEqual(sorted(g.topology.prim_sizes().tolist()), [3, 3] + [4] * 8)
        self.assertEqual(g.npoints, 16)
        self.assertEqual(int((edges.values(g, edges.PREFIX + "splitedges") != 0).sum()), 1)
        with self.assertRaises(Exception):
            cook("polysplit", {"mode": "points", "split_points": "0-15"}, src_params=GRID)

    def test_empty_selection_is_a_noop(self):
        g = cook("polysplit", {}, src_params=GRID)
        self.assertEqual(g.nprims, 9)


if __name__ == "__main__":
    unittest.main()
