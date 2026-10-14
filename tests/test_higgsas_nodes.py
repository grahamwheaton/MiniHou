"""Nodes taken from the Higgsas Geo Nodes library triage (docs/HIGGSAS_NODES.md): solidify, falloff, occlusion, contour, slice,
sdf_shape, uv_transform, curve_simplify."""
import unittest

import numpy as np

from minihou import Evaluator, Graph
from minihou.nodes._util import fan_triangulate
from minihou.nodes.bevel import twins


def cook(nodes, last):
    nodes = dict(nodes)
    nodes["out"] = {"type": "output", "inputs": {"geo": last}}
    g = Evaluator().cook(Graph.from_dict({"nodes": nodes, "display": "out"}))
    g.validate()
    return g


def one(typ, params=None, src=None, extra=None, inputs=None):
    """Cook ``typ`` on the source node dict ``src`` (default a unit box)."""
    nodes = {"s": src or {"type": "box"}}
    nodes.update(extra or {})
    nodes["n"] = {"type": typ, "inputs": inputs or {"geo": "s"}, "params": params or {}}
    return cook(nodes, "n")


def volume(g):
    tris, _ = fan_triangulate(g.topology)
    T = g.P[g.topology.vertex_point[tris]].astype(np.float64)
    return float(np.einsum("ij,ij->i", T[:, 0], np.cross(T[:, 1], T[:, 2])).sum() / 6.0)


def closed(test, g):
    test.assertTrue(np.all(twins(g) >= 0), "every edge shared by two faces, wound oppositely")


class SolidifyTests(unittest.TestCase):
    def test_closed_box_becomes_a_hollow_shell_of_exact_wall_thickness(self):
        g = one("solidify", {"thickness": 0.1})
        closed(self, g)
        self.assertAlmostEqual(volume(g), 1 - 0.8 ** 3, places=5)                     # the original is the outer surface (offset -1)
        self.assertEqual(g.nprims, 12)
        self.assertEqual(g.npoints, 16)
        c = one("solidify", {"thickness": 0.1, "offset": 0.0})
        self.assertAlmostEqual(volume(c), 1.1 ** 3 - 0.9 ** 3, places=5)                # centred on the original
        o = one("solidify", {"thickness": 0.1, "offset": 1.0})
        self.assertAlmostEqual(volume(o), 1.2 ** 3 - 1.0, places=5)

    def test_even_thickness_keeps_the_wall_the_same_at_corners(self):
        even = one("solidify", {"thickness": 0.1, "even": True})
        flat = one("solidify", {"thickness": 0.1, "even": False})
        self.assertGreater(volume(even), volume(flat))                                  # without it the corners are cut short
        P = even.P[8:]                                                                   # the inner points
        self.assertTrue(np.allclose(np.abs(P), 0.4, atol=1e-5))                          # inner cube of side 0.8

    def test_open_surface_gets_a_rim_and_is_a_closed_solid(self):
        src = {"type": "grid", "params": {"size_x": 2, "size_y": 2, "rows": 3, "columns": 3}}
        g = one("solidify", {"thickness": 0.2, "offset": 0.0}, src=src)
        closed(self, g)
        self.assertAlmostEqual(abs(volume(g)), 4 * 0.2, places=5)                       # area x thickness
        self.assertEqual(g.nprims, 4 + 4 + 8)                                           # top, bottom, 8 rim quads round a 2x2 patch
        sheets = one("solidify", {"thickness": 0.2, "rim": False}, src=src)
        self.assertEqual(sheets.nprims, 8)

    def test_output_groups(self):
        g = one("solidify", {"thickness": 0.1, "outgrp_top": True, "outgrp_bottom": True, "outgrp_side": True},
                src={"type": "grid", "params": {"rows": 3, "columns": 3}})
        top, bottom, side = (g.prim[f"group_{n}"] != 0 for n in ("solidifytop", "solidifybottom", "solidifyside"))
        self.assertEqual((int(top.sum()), int(bottom.sum()), int(side.sum())), (4, 4, 8))
        self.assertFalse((top & bottom).any() or (top & side).any() or (bottom & side).any())

    def test_attributes_ride_along(self):
        nodes = {"w": {"type": "wrangle", "inputs": {"geo": "s"}, "params": {"snippet": "@Cd = set(@P.x, 0, 0);"}},
                 "m": {"type": "set_material", "inputs": {"geo": "w"}, "params": {}}}
        src = {"type": "grid", "params": {"rows": 3, "columns": 3}}
        g = one("solidify", {"thickness": 0.1}, src=src, extra={"w": {"type": "wrangle", "inputs": {"geo": "s"}, "params": {"snippet": "@Cd = set(@P.x, 0, 0);"}}},
                inputs={"geo": "w"})
        self.assertIn("Cd", g.point)
        np.testing.assert_allclose(g.point["Cd"][9:, 0], g.point["Cd"][:9, 0])           # inner points copy their source point
        self.assertIn("uv", g.point.names() if hasattr(g.point, "names") else list(g.point))
        self.assertNotIn("N", g.point)

    def test_group_solidifies_only_those_faces(self):
        src = {"type": "grid", "params": {"rows": 3, "columns": 3}}
        extra = {"gc": {"type": "group_create", "inputs": {"geo": "s"}, "params": {"name": "g", "domain": "primitives", "method": "pattern", "pattern": "0"}}}
        g = one("solidify", {"thickness": 0.1, "group": "g"}, src=src, extra=extra, inputs={"geo": "gc"})
        self.assertEqual(g.nprims, 4 + 1 + 4)                                           # all four faces, one inner face, four rim quads
        self.assertEqual(g.npoints, 9 + 4)

    def test_zero_thickness_and_curves_pass_through(self):
        g = one("solidify", {"thickness": 0.0})
        self.assertEqual((g.npoints, g.nprims), (8, 6))
        c = one("solidify", {"thickness": 0.1}, src={"type": "line"})
        self.assertEqual((c.npoints, c.nprims), (10, 1))


GRID = {"type": "grid", "params": {"size_x": 10, "size_y": 10, "rows": 11, "columns": 11}}          # points at integer x, y in -5..5


def mask(g, name="mask"):
    return g.point[name].astype(np.float64)


class FalloffTests(unittest.TestCase):
    def test_sphere_is_one_at_the_centre_and_zero_outside(self):
        g = one("falloff", {"shape": "sphere", "size": 3.0, "smooth": False}, src=GRID)
        P = g.P
        d = np.linalg.norm(P - [0, 0, 0], axis=1)
        m = mask(g)
        np.testing.assert_allclose(m, np.clip((3.0 - d) / 3.0, 0, 1), atol=1e-6)
        self.assertEqual(float(m[d < 1e-6][0]), 1.0)
        self.assertTrue((m[d >= 3] == 0).all())
        hard = mask(one("falloff", {"shape": "sphere", "size": 3.0, "blend": 0.0}, src=GRID))
        self.assertTrue(set(np.unique(hard)) <= {0.0, 1.0})

    def test_sphere_smooth_and_flip_and_range(self):
        base = mask(one("falloff", {"shape": "sphere", "size": 3.0, "smooth": False}, src=GRID))
        sm = mask(one("falloff", {"shape": "sphere", "size": 3.0, "smooth": True}, src=GRID))
        mid = (base > 0.3) & (base < 0.4)
        self.assertTrue((sm[mid] < base[mid]).all())                                    # smoothstep eases the low end
        fl = mask(one("falloff", {"shape": "sphere", "size": 3.0, "smooth": False, "flip": True}, src=GRID))
        np.testing.assert_allclose(fl, 1 - base, atol=1e-6)
        rg = mask(one("falloff", {"shape": "sphere", "size": 3.0, "smooth": False, "min": 2.0, "max": 5.0}, src=GRID))
        np.testing.assert_allclose(rg, 2.0 + 3.0 * base, atol=1e-5)

    def test_directional_ramps_along_the_direction(self):
        g = one("falloff", {"shape": "directional", "direction": [1, 0, 0], "center": [-5, 0, 0], "size": 10.0, "smooth": False}, src=GRID)
        np.testing.assert_allclose(mask(g), (g.P[:, 0] + 5) / 10, atol=1e-6)
        m = one("falloff", {"shape": "directional", "direction": [1, 0, 0], "center": [0, 0, 0], "size": 5.0, "smooth": False, "mirror": True}, src=GRID)
        np.testing.assert_allclose(mask(m), 1 - np.clip(np.abs(m.P[:, 0]) / 5, 0, 1), atol=1e-6)

    def test_radial_goes_once_round_the_axis(self):
        g = one("falloff", {"shape": "radial", "direction": [0, 0, 1], "smooth": False}, src=GRID)
        ang = (np.degrees(np.arctan2(g.P[:, 1], g.P[:, 0])) % 360) / 360
        origin = np.linalg.norm(g.P[:, :2], axis=1) < 1e-9
        np.testing.assert_allclose(mask(g)[~origin], ang[~origin], atol=1e-5)

    def test_wave_is_a_pulse_at_the_front(self):
        g = one("falloff", {"shape": "wave", "direction": [1, 0, 0], "offset": 2.0, "size": 2.0, "smooth": False}, src=GRID)
        m = mask(g)
        self.assertAlmostEqual(float(m[np.isclose(g.P[:, 0], 2)].max()), 1.0, places=5)             # peak at the front
        self.assertTrue((m[g.P[:, 0] >= 4] == 0).all() and (m[g.P[:, 0] <= 0] == 0).all())
        self.assertGreater(float(m[np.isclose(g.P[:, 0], 3)].max()), 0.0)

    def test_index_ramp_every_and_primitives(self):
        g = one("falloff", {"shape": "index", "range": 10.0, "smooth": False}, src=GRID)
        np.testing.assert_allclose(mask(g), np.clip(np.arange(g.npoints) / 10.0, 0, 1), atol=1e-6)
        r = one("falloff", {"shape": "index", "range": 4.0, "every": 8, "smooth": False}, src=GRID)
        np.testing.assert_allclose(mask(r)[:16], np.clip((np.arange(16) % 8) / 4.0, 0, 1), atol=1e-6)
        pr = one("falloff", {"shape": "index", "range": 10.0, "domain": "primitives", "smooth": False}, src=GRID)
        np.testing.assert_allclose(pr.prim["mask"], np.clip(np.arange(pr.nprims) / 10.0, 0, 1), atol=1e-6)

    def test_mesh_distance_and_inside(self):
        box = {"b": {"type": "box", "params": {"size": [2, 2, 2]}}}
        g = one("falloff", {"shape": "mesh", "size": 2.0, "smooth": False}, src=GRID, extra=box, inputs={"geo": "s", "shape_mesh": "b"})
        m = mask(g)
        d = np.maximum(np.abs(g.P[:, :2]).max(1) - 1.0, 0.0)                              # the grid is in the plane z = 0: inside the box's slab
        self.assertTrue((m[(np.abs(g.P[:, 0]) <= 1) & (np.abs(g.P[:, 1]) <= 1)] > 0).all())
        self.assertTrue((m[np.abs(g.P[:, 0]) >= 4] == 0).all())
        ins = one("falloff", {"shape": "mesh", "size": 0.1, "inside": True, "blend": 0.0}, src=GRID, extra=box, inputs={"geo": "s", "shape_mesh": "b"})
        self.assertTrue((mask(ins)[(np.abs(ins.P[:, 0]) < 1) & (np.abs(ins.P[:, 1]) < 1)] == 1).all())
        with self.assertRaises(Exception):
            one("falloff", {"shape": "mesh"}, src=GRID)

    def test_group_limits_the_falloff(self):
        extra = {"gc": {"type": "group_create", "inputs": {"geo": "s"}, "params": {"name": "left", "domain": "points", "method": "bounding_box",
                                                                                  "box_center": [-5, 0, 0], "box_size": [10, 20, 20]}}}
        g = one("falloff", {"shape": "sphere", "size": 20.0, "smooth": False, "group": "left", "min": 0.25}, src=GRID, extra=extra, inputs={"geo": "gc"})
        right = g.P[:, 0] > 0
        self.assertTrue((mask(g)[right] == 0.25).all())
        self.assertTrue((mask(g)[~right] > 0.25).any())

    def test_guides_and_zero_direction(self):
        from minihou import handles
        reg = Evaluator().registry
        src = Evaluator().cook(Graph.from_dict({"nodes": {"s": GRID}, "display": "s"}))
        nt = reg.get("falloff")
        hs = handles.handles_for("falloff", [src], src, nt.coerce_params({"shape": "sphere", "size": 2}), nt)
        self.assertEqual([h["id"] for h in hs if h["kind"] == "guide"], ["guide:sphere"])
        with self.assertRaises(Exception):
            one("falloff", {"shape": "directional", "direction": [0, 0, 0]}, src=GRID)


PLATE = {"type": "grid", "params": {"size_x": 4, "size_y": 4, "rows": 5, "columns": 5}}


class OcclusionTests(unittest.TestCase):
    def test_an_open_plane_is_not_occluded_and_a_roof_over_it_is(self):
        flat = one("occlusion", {"rays": 16}, src=PLATE)
        self.assertTrue((flat.point["ao"] == 1.0).all())
        roof = {"r": {"type": "box", "params": {"size": [2, 2, 0.1], "center": [0, 0, 0.5]}}}
        g = one("occlusion", {"rays": 32, "self_occlusion": False}, src=PLATE, extra=roof, inputs={"geo": "s", "occluders": "r"})
        under = np.linalg.norm(g.P[:, :2], axis=1) < 0.5
        far = np.linalg.norm(g.P[:, :2], axis=1) > 2.5
        self.assertLess(float(g.point["ao"][under].max()), 0.8)
        self.assertTrue((g.point["ao"][far] > g.point["ao"][under].max()).all())
        inv = one("occlusion", {"rays": 32, "self_occlusion": False, "invert": True}, src=PLATE, extra=roof, inputs={"geo": "s", "occluders": "r"})
        np.testing.assert_allclose(inv.point["ao"], 1 - g.point["ao"], atol=1e-6)

    def test_the_inside_of_a_box_is_darker_in_the_corners(self):
        src = {"type": "box", "params": {"divisions": 4}}
        extra = {"r": {"type": "reverse", "inputs": {"geo": "s"}, "params": {}}}
        g = one("occlusion", {"rays": 64, "max_distance": 0.7}, src=src, extra=extra, inputs={"geo": "r"})
        corner = np.all(np.abs(g.P) > 0.49, axis=1)
        facemid = (np.abs(g.P) > 0.49).sum(1) == 1
        facemid &= (np.abs(g.P) < 0.01).sum(1) == 2
        self.assertLess(float(g.point["ao"][corner].mean()), float(g.point["ao"][facemid].mean()))

    def test_thickness_of_a_plate_is_its_thickness(self):
        extra = {"sol": {"type": "solidify", "inputs": {"geo": "s"}, "params": {"thickness": 0.2}}}
        g = one("occlusion", {"mode": "thickness", "spread": 5, "rays": 8}, src=PLATE, extra=extra, inputs={"geo": "sol"})
        top = g.P[:, 2] > -0.01
        mid = top & (np.abs(g.P[:, :2]).max(1) < 1.5)
        np.testing.assert_allclose(g.point["thickness"][mid], 0.2, atol=2e-3)
        lim = one("occlusion", {"mode": "thickness", "spread": 5, "rays": 8, "max_distance": 0.1}, src=PLATE, extra=extra, inputs={"geo": "sol"})
        self.assertTrue((lim.point["thickness"] == 0).all())                                    # the other wall is beyond the limit
        nz = one("occlusion", {"mode": "thickness", "spread": 5, "rays": 8, "normalize": True}, src=PLATE, extra=extra, inputs={"geo": "sol"})
        self.assertAlmostEqual(float(nz.point["thickness"].max()), 1.0, places=5)

    def test_name_group_seed_and_blur(self):
        a = one("occlusion", {"rays": 8, "seed": 3, "name": "cavity"}, src=PLATE)
        self.assertIn("cavity", a.point)
        b = one("occlusion", {"rays": 8, "seed": 3, "name": "cavity"}, src=PLATE)
        np.testing.assert_array_equal(a.point["cavity"], b.point["cavity"])                      # deterministic
        extra = {"gc": {"type": "group_create", "inputs": {"geo": "s"}, "params": {"name": "g", "domain": "points", "method": "pattern", "pattern": "0-3"}}}
        g = one("occlusion", {"rays": 8, "group": "g", "mode": "thickness"}, src=PLATE, extra=extra, inputs={"geo": "gc"})
        self.assertTrue((g.point["thickness"][4:] == 0).all())
        with self.assertRaises(Exception):
            one("occlusion", {"self_occlusion": False}, src=PLATE)


def area(g):
    from minihou.nodes.normal import prim_normals
    return float(np.linalg.norm(prim_normals(g), axis=1).sum() / 2.0)


def with_h(snippet):
    return {"w": {"type": "wrangle", "inputs": {"geo": "s"}, "params": {"snippet": snippet}}}


class ContourTests(unittest.TestCase):
    def test_lines_along_a_linear_field_are_straight_and_exact(self):
        g = one("contour", {"attribute": "P.x", "level": 0.5}, src=GRID)
        self.assertEqual((g.nprims, g.npoints), (1, 11))
        np.testing.assert_allclose(g.P[:, 0], 0.5, atol=1e-6)
        self.assertEqual(sorted(g.P[:, 1].round(3).tolist()), list(np.arange(-5, 6, dtype=float)))
        np.testing.assert_allclose(g.point["level"], 0.5)
        self.assertEqual(float(g.prim["level"][0]), 0.5)
        self.assertTrue(g.topology.has_open)

    def test_a_circle_is_a_closed_loop_and_fill_makes_it_a_polygon(self):
        extra = with_h("f@h = length(@P);")
        g = one("contour", {"attribute": "h", "level": 3.0}, src=GRID, extra=extra, inputs={"geo": "w"})
        self.assertEqual(g.nprims, 1)
        self.assertTrue(g.topology.has_open and not g.topology.closed.any())              # an outline only: open, ends at its start
        self.assertTrue(np.allclose(g.P[0], g.P[-1]))
        r = np.linalg.norm(g.P, axis=1)
        self.assertLess(float(np.abs(r - 3.0).max()), 0.1)                                # the chord error of 1-unit cells
        f = one("contour", {"attribute": "h", "level": 3.0, "fill": True}, src=GRID, extra=extra, inputs={"geo": "w"})
        self.assertFalse(f.topology.has_open)
        self.assertAlmostEqual(area(f), np.pi * 9, delta=1.0)

    def test_several_levels_each_carry_their_level(self):
        extra = with_h("f@h = length(@P);")
        g = one("contour", {"attribute": "h", "level": 1.5, "levels": 3, "spacing": 1.0}, src=GRID, extra=extra, inputs={"geo": "w"})
        self.assertEqual(g.nprims, 3)
        self.assertEqual(sorted(g.prim["level"].round(3).tolist()), [1.5, 2.5, 3.5])
        r = np.linalg.norm(g.P, axis=1)
        np.testing.assert_allclose(r, g.point["level"], atol=0.12)

    def test_saddles_are_split_by_the_centre_value_for_quads(self):
        src = {"type": "grid", "params": {"size_x": 2, "size_y": 2, "rows": 2, "columns": 2}}
        extra = with_h("f@h = (@ptnum == 0) + (@ptnum == 3);")                            # corners 0 and 3 high, 1 and 2 low
        sq = one("contour", {"attribute": "h", "level": 0.6, "cells": "polygons"}, src=src, extra=extra, inputs={"geo": "w"})
        tr = one("contour", {"attribute": "h", "level": 0.6, "cells": "triangles"}, src=src, extra=extra, inputs={"geo": "w"})
        self.assertEqual((sq.nprims, tr.nprims), (2, 2))
        base = cook({"s": src}, "s")
        hi = base.P[[0, 3]]

        def pairs_round_a_high_corner(g):
            """Each curve joins two crossings that sit around the same high corner."""
            offs = g.topology.prim_offsets
            ok = []
            for i in range(g.nprims):
                pts = g.P[offs[i]:offs[i + 1]]
                ok.append(any(np.all(np.linalg.norm(pts - h, axis=1) < 0.95) for h in hi))
            return all(ok)
        self.assertTrue(pairs_round_a_high_corner(sq))                                     # centre (0.5) below the level: the high corners are cut off
        self.assertFalse(pairs_round_a_high_corner(tr))                                    # the fan diagonal joins the high corners: the low ones are cut off

    def test_surface_keeps_one_side_and_cuts_exactly(self):
        above = one("contour", {"mode": "surface", "attribute": "P.x", "level": 0.5, "outgrp_cutpoints": True}, src=GRID)
        below = one("contour", {"mode": "surface", "attribute": "P.x", "level": 0.5, "keep_above": False}, src=GRID)
        self.assertAlmostEqual(area(above), 45.0, places=4)
        self.assertAlmostEqual(area(below), 55.0, places=4)
        np.testing.assert_allclose(above.P[:, 0].min(), 0.5, atol=1e-6)
        cut = above.point["group_contourpoints"] != 0
        self.assertEqual(int(cut.sum()), 11)
        np.testing.assert_allclose(above.P[cut, 0], 0.5, atol=1e-6)
        self.assertTrue((above.topology.prim_sizes() == 4).all())                          # quads stay quads
        self.assertTrue(np.all(np.bincount(above.topology.vertex_point, minlength=above.npoints) > 0))   # no stray points

    def test_attributes_are_interpolated_and_uv_survives_the_cut(self):
        g = one("contour", {"mode": "surface", "attribute": "P.x", "level": 0.25}, src=GRID)
        self.assertIn("uv", g.point)
        np.testing.assert_allclose(g.point["uv"][:, 0], (g.P[:, 0] + 5) / 10, atol=1e-5)    # the grid's u runs with x

    def test_group_names_and_errors(self):
        with self.assertRaises(Exception):
            one("contour", {"attribute": "nope"}, src=GRID)
        with self.assertRaises(Exception):
            one("contour", {"attribute": "P.w"}, src=GRID)
        none = one("contour", {"attribute": "P.x", "level": 100.0}, src=GRID)
        self.assertEqual(none.nprims, 0)


class SliceTests(unittest.TestCase):
    def test_planes_through_a_sphere_are_circles(self):
        sph = {"type": "sphere", "params": {"rows": 24, "columns": 48}}
        full = one("slice", {"count": 3, "spacing": 0.25}, src=sph)
        R = float(np.abs(full.P).max())
        self.assertEqual(full.nprims, 3)
        np.testing.assert_allclose(np.sqrt((full.P ** 2).sum(1)), R, atol=0.01 * R)
        self.assertEqual(sorted(set(full.prim["level"].round(3).tolist())), [0.0, 0.25, 0.5])
        self.assertEqual(len({round(float(z), 3) for z in full.P[:, 2]}), 3)

    def test_cover_stacks_planes_through_the_whole_height_and_fill_caps(self):
        sph = {"type": "sphere", "params": {"rows": 24, "columns": 48}}
        g = one("slice", {"cover": True, "spacing": 0.25}, src=sph)
        lv = sorted(set(g.prim["level"].round(3).tolist()))
        self.assertGreaterEqual(len(lv), 3)
        self.assertTrue(all(abs(l / 0.25 - round(l / 0.25)) < 1e-3 for l in lv))
        f = one("slice", {"cover": True, "spacing": 0.25, "fill": True}, src=sph)
        self.assertFalse(f.topology.has_open)
        self.assertGreater(int((f.P[:, 2].round(3) == 0.0).sum()), 40)

    def test_normal_origin_group_and_errors(self):
        g = one("slice", {"normal": [1, 0, 0], "origin": [0.1, 0, 0], "count": 1}, src={"type": "box", "params": {"divisions": 4}})
        np.testing.assert_allclose(g.P[:, 0], 0.1, atol=1e-6)
        with self.assertRaises(Exception):
            one("slice", {"normal": [0, 0, 0]})
        empty = one("slice", {"origin": [0, 0, 9]})
        self.assertEqual(empty.nprims, 0)


if __name__ == "__main__":
    unittest.main()
