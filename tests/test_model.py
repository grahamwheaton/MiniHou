"""Polygon modelling nodes, checked by invariants: closed shells, outward winding, conserved area, valid topology."""
import unittest

import numpy as np

from minihou import Evaluator, Geometry, Graph, NodeError, topo
from minihou.nodes.normal import prim_normals


def cook(nodes):
    nodes = dict(nodes)
    last = list(nodes)[-1]
    nodes["out"] = {"type": "output", "inputs": {"geo": last}}
    g = Evaluator().cook(Graph.from_dict({"nodes": nodes, "display": "out"}))
    g.validate()
    return g


def chain(*steps, src=None):
    nodes = dict(src or {"g": {"type": "grid", "params": {"columns": 4, "rows": 4, "size_x": 3, "size_y": 3}}})
    prev = list(nodes)[-1]
    for i, (typ, params) in enumerate(steps):
        nodes[f"n{i}"] = {"type": typ, "inputs": {"geo": prev}, "params": params}
        prev = f"n{i}"
    return cook(nodes)


def closed(g):
    return bool(np.all(topo.edges(g).count == 2))


def volume(g):
    """Signed volume of a closed shell (positive when faces point outwards)."""
    c = topo.prim_centroids(g)
    return float(np.einsum("ij,ij->", prim_normals(g), c)) / 6.0


def area(g):
    return float(np.linalg.norm(prim_normals(g), axis=1).sum()) / 2


BOX = {"b": {"type": "box", "params": {"size": [2, 2, 2]}}}
SPHERE = {"s": {"type": "sphere", "params": {"radius": 1, "rows": 12, "columns": 16}}}


class ReverseFacetDivideTests(unittest.TestCase):
    def test_reverse_flips_winding_and_normals(self):
        g = chain(("normal", {}), ("reverse", {}), src=BOX)
        self.assertAlmostEqual(volume(g), -8.0, places=4)
        b = chain(("normal", {}), src=BOX)
        np.testing.assert_allclose(g.point["N"], -b.point["N"], atol=1e-6)
        part = chain(("reverse", {"group": "0"}), src=BOX)
        self.assertFalse(np.allclose(prim_normals(part)[1:], -prim_normals(cook(BOX))[1:]))

    def test_reverse_selected_corner_attributes_and_unaffected_faces(self):
        from minihou.nodes.model import reverse
        b = cook(BOX).copy()
        b.vertex["uv"] = np.arange(b.nvertices * 2, dtype=np.float32).reshape(-1, 2)
        b.prim["material_id"] = np.arange(b.nprims, dtype=np.int32)
        result = reverse(b, "0", True)
        result.validate()
        end = b.topology.prim_offsets[1]
        np.testing.assert_array_equal(result.vertex["uv"][:end], b.vertex["uv"][:end][::-1])
        np.testing.assert_array_equal(result.vertex["uv"][end:], b.vertex["uv"][end:])
        np.testing.assert_array_equal(result.topology.vertex_point[end:], b.topology.vertex_point[end:])
        np.testing.assert_array_equal(result.prim["material_id"], b.prim["material_id"])

    def test_facet_unique_points(self):
        g = chain(("facet", {"compute_normals": True}))
        self.assertEqual(g.npoints, g.nvertices)
        np.testing.assert_allclose(g.point["N"], np.tile([0, 0, 1], (36, 1)), atol=1e-6)
        self.assertIn("uv", g.point)

    def test_divide_conserves_area_and_handles_concave_ngons(self):
        g = chain(("divide", {}), src=SPHERE)
        self.assertTrue(np.all(g.topology.prim_sizes() == 3))
        self.assertAlmostEqual(area(g), area(cook(SPHERE)), places=4)
        # an L-shaped (concave) hexagon: fanning from the reflex corner's neighbour would overlap
        L = Geometry.from_arrays(np.array([[0, 0, 0], [2, 0, 0], [2, 1, 0], [1, 1, 0], [1, 2, 0], [0, 2, 0]], np.float32),
                                 [0, 1, 2, 3, 4, 5], [0, 6])
        from minihou.nodes.model import divide
        d = divide(L, "")
        self.assertEqual(d.nprims, 4)
        fn = prim_normals(d)
        self.assertTrue(np.all(fn[:, 2] > 0))                               # no flipped triangles
        self.assertAlmostEqual(float(np.linalg.norm(fn, axis=1).sum() / 2), 3.0, places=5)


class MirrorClipFillTests(unittest.TestCase):
    def test_mirror_welds_the_seam_into_a_closed_shell(self):
        half = chain(("clip", {"normal": [1, 0, 0]}), src=SPHERE)            # x >= 0 half, open at x = 0
        self.assertFalse(closed(half))
        whole = chain(("clip", {"normal": [1, 0, 0]}), ("mirror", {"normal": [1, 0, 0]}), src=SPHERE)
        self.assertTrue(closed(whole))
        self.assertGreater(volume(whole), 3.0)                              # outward-facing (sphere ~ 4.19)

    def test_clip_cuts_faces_and_interpolates(self):
        g = chain(("clip", {"origin": [0.25, 0, 0], "normal": [1, 0, 0]}))
        self.assertAlmostEqual(float(g.P[:, 0].min()), 0.25, places=5)
        self.assertAlmostEqual(area(g), 1.25 * 3, places=4)                 # x in [0.25, 1.5] of a 3x3 grid
        np.testing.assert_allclose(g.point["uv"][np.argmin(g.P[:, 0])][0], (0.25 + 1.5) / 3, atol=1e-5)
        both = chain(("clip", {"origin": [0.25, 0, 0], "normal": [1, 0, 0], "keep": "both"}))
        self.assertAlmostEqual(area(both), 9.0, places=4)
        self.assertEqual(int(both.prim["group_above"].sum()), 6)
        self.assertEqual(both.npoints, 16 + 4)                              # the cut's 4 new points are shared

    def test_clip_then_polyfill_closes_a_sphere(self):
        g = chain(("clip", {"origin": [0, 0, 0.3]}), ("polyfill", {"fill_group": "cap"}), src=SPHERE)
        self.assertTrue(closed(g))
        self.assertGreater(volume(g), 0)
        self.assertEqual(int(g.prim["group_cap"].sum()), 1)

    def test_polyfill_on_a_closed_mesh_does_nothing(self):
        self.assertEqual(chain(("polyfill", {}), src=BOX).nprims, 6)

    def test_polyfill_cap_inherits_boundary_corner_ids(self):
        from minihou.nodes.model import polyfill
        g = Geometry.from_arrays(np.array([[0, 0, 0], [1, 0, 0], [1, 1, 0], [0, 1, 0]], np.float32),
                                 [0, 1, 2, 3], [0, 4])
        g.vertex['corner_id'] = np.array([10, 11, 12, 13], np.int32)
        out = polyfill(g, '')
        np.testing.assert_array_equal(out.vertex['corner_id'][4:], [10, 13, 12, 11])


class ExtrudeTests(unittest.TestCase):
    def test_connected_extrude_with_back_is_a_closed_outward_box(self):
        g = chain(("polyextrude", {"distance": 2, "output_back": True}))
        self.assertTrue(closed(g))
        self.assertAlmostEqual(volume(g), 9 * 2, places=3)
        self.assertEqual(int(g.prim["group_extrudeFront"].sum()), 9)
        self.assertEqual(int(g.prim["group_extrudeSide"].sum()), 12)
        self.assertAlmostEqual(float(g.P[g.point["uv"][:, 0] >= 0][:, 2].max()), 2, places=5)

    def test_extruding_part_of_a_closed_mesh_stays_closed(self):
        for mode in ("connected", "individual"):
            g = chain(("polyextrude", {"group": "0 2", "distance": 0.5, "mode": mode, "inset": 0.2}),
                      src={"b": {"type": "box", "params": {"size": [2, 2, 2], "divisions": 2}}})
            self.assertTrue(closed(g), mode)
            self.assertGreater(volume(g), 8.0, mode)                       # grew outwards

    def test_negative_distance_goes_inwards(self):
        g = chain(("polyextrude", {"group": "0", "distance": -0.5}), src=BOX)
        self.assertTrue(closed(g))
        self.assertLess(volume(g), 8.0)


class SubdivideTests(unittest.TestCase):
    def test_catmull_clark_box_converges_inside_and_stays_closed(self):
        g = chain(("subdivide", {"depth": 3}), src=BOX)
        self.assertEqual(g.nprims, 6 * 4 ** 3)
        self.assertTrue(closed(g))
        r = np.linalg.norm(g.P, axis=1)
        self.assertTrue(0.8 < r.min() and r.max() < 0.9)                   # the CC limit of a 2-cube
        self.assertGreater(volume(g), 0)

    def test_linear_keeps_shape_and_uvs_split_per_face(self):
        g = chain(("wrangle", {"snippet": "u@uv = set(@P.x, @P.y)", "run_over": "vertices"}),
                  ("subdivide", {"depth": 1, "algorithm": "linear"}))
        self.assertEqual(g.nprims, 36)
        self.assertAlmostEqual(area(g), 9.0, places=4)
        corners = g.P[g.topology.vertex_point][:, :2]
        np.testing.assert_allclose(g.vertex["uv"], corners, atol=1e-5)    # uv still equals position everywhere

    def test_open_boundary_and_groups(self):
        g = chain(("group_create", {"name": "left", "pattern": "@P.x < 0"}), ("subdivide", {"depth": 2}))
        self.assertAlmostEqual(float(g.P[:, 2].max()), 0.0)
        self.assertTrue(np.any(np.all(np.isclose(g.P[:, :2], [1.5, 1.5]), axis=1)))   # corners stay sharp
        self.assertAlmostEqual(area(g), 9.0, places=4)                     # a flat grid keeps its outline
        self.assertTrue(set(np.unique(g.point["group_left"]).tolist()) <= {0, 1})

    def test_size_guard(self):
        with self.assertRaises(NodeError):
            chain(("subdivide", {"depth": 6}), src={"g": {"type": "grid", "params": {"columns": 300, "rows": 300}}})


if __name__ == "__main__":
    unittest.main()
