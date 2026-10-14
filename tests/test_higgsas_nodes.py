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

    def test_negative_thickness_is_the_other_way_and_stays_a_valid_solid(self):
        for offset in (-1.0, 0.0, 1.0):
            neg = one("solidify", {"thickness": -0.1, "offset": offset})
            pos = one("solidify", {"thickness": 0.1, "offset": -offset})
            closed(self, neg)
            self.assertGreater(volume(neg), 0.0)
            self.assertAlmostEqual(volume(neg), volume(pos), places=6)
            np.testing.assert_allclose(np.sort(neg.P, axis=0), np.sort(pos.P, axis=0), atol=1e-6)
        grown = one("solidify", {"thickness": 0.1, "offset": 1.0})
        self.assertAlmostEqual(volume(grown), 1.2 ** 3 - 1.0, places=5)
        closed(self, grown)
        plate = {"type": "grid", "params": {"size_x": 2, "size_y": 2, "rows": 3, "columns": 3}}
        g = one("solidify", {"thickness": -0.2}, src=plate)
        closed(self, g)
        self.assertGreater(volume(g), 0.0)

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

    def test_the_outline_is_an_ordinary_open_curve_for_sweep_resample_and_the_link(self):
        extra = with_h("f@h = length(@P);")
        extra["c"] = {"type": "contour", "inputs": {"geo": "w"}, "params": {"attribute": "h", "level": 3.0}}
        swept = one("sweep", {"radius": 0.1}, src=GRID, extra=extra, inputs={"geo": "c"})
        self.assertEqual(swept.npoints, 21 * 8)                                          # 20 segments + the point that closes it, 8 sides each
        resampled = one("resample", {"length": 0.5}, src=GRID, extra=extra, inputs={"geo": "c"})
        self.assertEqual(resampled.nprims, 1)
        from minihou.link.diff import DiffState
        from minihou.link.protocol import GeoUpdate, Mirror
        c = cook({"s": GRID, **extra}, "c")
        h, arrays = DiffState().build(c.freeze(), 1)
        m = Mirror()
        m.apply(GeoUpdate.from_message(h, arrays))
        self.assertEqual(m.prim_closed.tolist(), [False])                                # a curve in Blender too, not a face

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


def shape(**params):
    params.setdefault("voxel_size", 0.05)
    return cook({"n": {"type": "sdf_shape", "params": params}}, "n")


class SdfShapeTests(unittest.TestCase):
    def test_sphere_matches_the_exact_volume_and_is_a_closed_quad_mesh(self):
        g = shape(shape="sphere", radius=0.5)
        closed(self, g)
        self.assertTrue((g.topology.prim_sizes() == 4).all())
        self.assertAlmostEqual(volume(g), 4 / 3 * np.pi * 0.5 ** 3, delta=0.03 * 4 / 3 * np.pi * 0.5 ** 3)
        r = np.linalg.norm(g.P, axis=1)
        self.assertLess(float(np.abs(r - 0.5).max()), 0.03)

    def test_box_size_center_rotation_and_bevel(self):
        g = shape(shape="box", size=[1.0, 2.0, 0.5], center=[1.0, 0.0, 0.0])
        lo, hi = g.P.min(0), g.P.max(0)
        np.testing.assert_allclose(hi - lo, [1.0, 2.0, 0.5], atol=0.06)
        np.testing.assert_allclose((hi + lo) / 2, [1.0, 0.0, 0.0], atol=0.03)
        rot = shape(shape="box", size=[1.0, 2.0, 0.5], rotate=[0, 0, 90])
        np.testing.assert_allclose(rot.P.max(0) - rot.P.min(0), [2.0, 1.0, 0.5], atol=0.06)
        sharp = volume(shape(shape="box", size=[1, 1, 1]))
        round_ = volume(shape(shape="box", size=[1, 1, 1], bevel=0.2))
        exact = 1 - (12 * (1 - 2 * 0.2) * 0.2 ** 2 * (1 - np.pi / 4) + 8 * 0.2 ** 3 * (1 - np.pi / 6))
        self.assertLess(round_, sharp)
        self.assertAlmostEqual(round_, exact, delta=0.04)

    def test_cylinder_capsule_ellipsoid_torus_cone_prism(self):
        cyl = shape(shape="cylinder", radius=0.4, depth=1.0)
        self.assertAlmostEqual(volume(cyl), np.pi * 0.16, delta=0.04 * np.pi * 0.16)
        cap = shape(shape="capsule", radius=0.3, depth=0.8)
        self.assertAlmostEqual(volume(cap), np.pi * 0.09 * 0.8 + 4 / 3 * np.pi * 0.027, delta=0.05)
        self.assertAlmostEqual(float(cap.P[:, 2].max()), 0.7, delta=0.04)
        ell = shape(shape="ellipsoid", size=[1.0, 0.6, 0.4])
        self.assertAlmostEqual(volume(ell), 4 / 3 * np.pi * 0.5 * 0.3 * 0.2, delta=0.02)
        tor = shape(shape="torus", radius=0.5, minor_radius=0.15)
        self.assertAlmostEqual(volume(tor), 2 * np.pi ** 2 * 0.5 * 0.15 ** 2, delta=0.03)
        closed(self, tor)
        cone = shape(shape="round_cone", radius=0.3, radius2=0.1, depth=1.0)
        self.assertAlmostEqual(float(cone.P[:, 2].min()), -0.5 - 0.3, delta=0.05)
        self.assertAlmostEqual(float(cone.P[:, 2].max()), 0.5 + 0.1, delta=0.05)
        sq = shape(shape="prism", sides=4, radius=0.5, depth=1.0)
        self.assertAlmostEqual(volume(sq), 0.5 * 1.0, delta=0.04)                           # a square of circumradius 0.5 has area 0.5

    def test_smoothing_and_limits(self):
        g = shape(shape="box", size=[1, 1, 1], smooth=3)
        sharp = shape(shape="box", size=[1, 1, 1])
        self.assertLess(volume(g), volume(sharp))
        with self.assertRaises(Exception):
            shape(shape="sphere", radius=50.0, voxel_size=0.01)                               # more voxels than allowed
        with self.assertRaises(Exception):
            shape(shape="sphere", voxel_size=0.0)


def corner_uv(g):
    return g.vertex["uv"].astype(np.float64) if "uv" in g.vertex else g.point["uv"][g.topology.vertex_point].astype(np.float64)


class UvTransformTests(unittest.TestCase):
    SRC = {"type": "grid", "params": {"size_x": 2, "size_y": 2, "rows": 3, "columns": 3}}              # point uv 0..1, u along x

    def base(self):
        return corner_uv(cook({"s": self.SRC}, "s"))

    def test_scale_translate_rotate_flip_about_the_pivot(self):
        b = self.base()
        g = one("uv_transform", {"scale": [2, 3, 1], "translate": [0.1, 0.2, 0], "pivot": "custom", "pivot_uv": [0, 0, 0]}, src=self.SRC)
        np.testing.assert_allclose(corner_uv(g), b * [2, 3] + [0.1, 0.2], atol=1e-6)
        r = one("uv_transform", {"rotate": 90.0}, src=self.SRC)                                       # about the centre of the uv bounds
        d = b - 0.5
        np.testing.assert_allclose(corner_uv(r), np.stack([-d[:, 1], d[:, 0]], 1) + 0.5, atol=1e-6)
        f = one("uv_transform", {"flip_u": True}, src=self.SRC)
        np.testing.assert_allclose(corner_uv(f), np.stack([1 - b[:, 0], b[:, 1]], 1), atol=1e-6)
        v = one("uv_transform", {"flip_v": True}, src=self.SRC)
        np.testing.assert_allclose(corner_uv(v), np.stack([b[:, 0], 1 - b[:, 1]], 1), atol=1e-6)
        self.assertNotIn("uv", f.point)                                                                 # now a face-corner attribute

    def test_islands_each_scale_about_their_own_centre(self):
        extra = {"p": {"type": "uv_project", "inputs": {"geo": "s"}, "params": {"method": "box"}}}
        src = {"type": "box"}
        p = one("uv_project", {"method": "box"}, src=src)
        g = one("uv_transform", {"scope": "island", "scale": [0.5, 0.5, 1]}, src=src, extra=extra, inputs={"geo": "p"})
        t = g.topology
        for i in range(g.nprims):
            rows = slice(t.prim_offsets[i], t.prim_offsets[i + 1])
            before, after = corner_uv(p)[rows], corner_uv(g)[rows]
            np.testing.assert_allclose((before.max(0) + before.min(0)) / 2, (after.max(0) + after.min(0)) / 2, atol=1e-6)   # each face is its own island
            np.testing.assert_allclose(np.ptp(after, axis=0), np.ptp(before, axis=0) * 0.5, atol=1e-6)
        from minihou.nodes.uv import uv_islands
        self.assertEqual(int(uv_islands(p, corner_uv(p)).max()) + 1, 6)
        self.assertEqual(int(uv_islands(cook({"s": self.SRC}, "s"), self.base()).max()) + 1, 1)

    def test_random_per_face_is_seeded_and_group_limited(self):
        a = one("uv_transform", {"scope": "face", "random_translate": [0.3, 0.3, 0], "random_rotate": 45.0, "random_scale": 0.3, "seed": 5}, src=self.SRC)
        b = one("uv_transform", {"scope": "face", "random_translate": [0.3, 0.3, 0], "random_rotate": 45.0, "random_scale": 0.3, "seed": 5}, src=self.SRC)
        c = one("uv_transform", {"scope": "face", "random_translate": [0.3, 0.3, 0], "random_rotate": 45.0, "random_scale": 0.3, "seed": 6}, src=self.SRC)
        np.testing.assert_array_equal(a.vertex["uv"], b.vertex["uv"])
        self.assertGreater(float(np.abs(a.vertex["uv"] - c.vertex["uv"]).max()), 0.01)
        extra = {"gc": {"type": "group_create", "inputs": {"geo": "s"}, "params": {"name": "g", "domain": "primitives", "method": "pattern", "pattern": "0"}}}
        g = one("uv_transform", {"translate": [1, 0, 0], "scope": "face", "group": "g"}, src=self.SRC, extra=extra, inputs={"geo": "gc"})
        moved = np.abs(corner_uv(g) - self.base()).max(1) > 1e-6
        self.assertEqual(moved.reshape(-1, 4).all(1).tolist(), [True, False, False, False])

    def test_align_turns_the_islands_so_a_direction_follows_a_uv_axis(self):
        g = one("uv_transform", {"align": True, "direction": [1, 0, 0], "align_axis": "v"}, src=self.SRC)
        uv, P = corner_uv(g), cook({"s": self.SRC}, "s").P[cook({"s": self.SRC}, "s").topology.vertex_point]
        self.assertGreater(float(np.corrcoef(P[:, 0], uv[:, 1])[0, 1]), 0.999)                           # v now grows with x
        self.assertLess(abs(float(np.corrcoef(P[:, 0], uv[:, 0])[0, 1])), 1e-6)
        same = one("uv_transform", {"align": True, "direction": [0, 1, 0], "align_axis": "v"}, src=self.SRC)
        np.testing.assert_allclose(corner_uv(same), self.base(), atol=1e-6)                              # already up
        u = one("uv_transform", {"align": True, "direction": [0, 1, 0], "align_axis": "u"}, src=self.SRC)
        self.assertGreater(float(np.corrcoef(P[:, 1], corner_uv(u)[:, 0])[0, 1]), 0.999)

    def test_errors(self):
        with self.assertRaises(Exception):
            one("uv_transform", {"uv": "nope"}, src=self.SRC)
        with self.assertRaises(Exception):
            one("uv_transform", {"align": True, "direction": [0, 0, 0]}, src=self.SRC)


def polyline_distance(points, line, closed):
    """Distance of each point from the polyline through ``line``."""
    seg_a = line
    seg_b = np.roll(line, -1, axis=0) if closed else line[1:]
    seg_a = line if closed else line[:-1]
    d = seg_b - seg_a
    ln = (d * d).sum(1)
    q = points[:, None, :] - seg_a[None]
    u = np.clip((q * d[None]).sum(2) / np.maximum(ln, 1e-30)[None], 0, 1)
    return np.linalg.norm(q - u[..., None] * d[None], axis=2).min(1)


class CurveSimplifyTests(unittest.TestCase):
    CIRCLE = {"type": "circle", "params": {"divisions": 360, "radius": 1.0}}

    def test_distance_method_stays_within_the_tolerance(self):
        orig = cook({"s": self.CIRCLE}, "s")
        for tol in (0.05, 0.01):
            g = one("curve_simplify", {"tolerance": tol}, src=self.CIRCLE)
            self.assertLess(g.npoints, orig.npoints // 5)
            self.assertFalse(g.topology.has_open)                                                    # still a closed loop
            self.assertLessEqual(float(polyline_distance(orig.P.astype(np.float64), g.P.astype(np.float64), True).max()), tol + 1e-6)

    def test_open_curves_keep_their_ends_and_a_straight_line_becomes_two_points(self):
        noisy = {"type": "line", "params": {"points": 50, "length": 4.0}}
        g = one("curve_simplify", {"tolerance": 0.001}, src=noisy)
        self.assertEqual(g.npoints, 2)
        np.testing.assert_allclose(g.P[[0, 1], 2], [0.0, 4.0], atol=1e-6)
        h = one("curve_simplify", {"tolerance": 0.01}, src={"type": "helix", "params": {"points": 200}})
        self.assertTrue(h.topology.has_open)
        orig = cook({"s": {"type": "helix", "params": {"points": 200}}}, "s")
        np.testing.assert_allclose(h.P[0], orig.P[0], atol=1e-6)
        np.testing.assert_allclose(h.P[-1], orig.P[-1], atol=1e-6)
        self.assertLess(h.npoints, 100)

    def test_angle_method_counts_turning(self):
        g = one("curve_simplify", {"method": "angle", "angle": 10.0}, src=self.CIRCLE)
        self.assertTrue(30 <= g.npoints <= 40)
        coarse = one("curve_simplify", {"method": "angle", "angle": 45.0}, src=self.CIRCLE)
        self.assertLess(coarse.npoints, g.npoints)

    def test_attributes_ride_with_the_kept_points_and_a_group_pins_points(self):
        extra = {"w": {"type": "wrangle", "inputs": {"geo": "s"}, "params": {"snippet": "i@orig = @ptnum;"}}}
        g = one("curve_simplify", {"tolerance": 0.01}, src=self.CIRCLE, extra=extra, inputs={"geo": "w"})
        orig = cook({"s": self.CIRCLE}, "s")
        np.testing.assert_allclose(g.P, orig.P[g.point["orig"]], atol=1e-6)
        self.assertEqual(g.point["orig"][0], 0)
        extra["gc"] = {"type": "group_create", "inputs": {"geo": "w"}, "params": {"name": "pin", "domain": "points", "method": "pattern", "pattern": "7"}}
        k = one("curve_simplify", {"tolerance": 0.01, "keep": "pin"}, src=self.CIRCLE, extra=extra, inputs={"geo": "gc"})
        self.assertIn(7, k.point["orig"].tolist())

    def test_smooth_moves_interior_points_but_not_the_ends(self):
        src = {"type": "helix", "params": {"points": 120}}
        a = one("curve_simplify", {"tolerance": 0.02}, src=src)
        b = one("curve_simplify", {"tolerance": 0.02, "smooth": 4}, src=src)
        self.assertEqual(a.npoints, b.npoints)
        np.testing.assert_allclose(a.P[[0, -1]], b.P[[0, -1]], atol=1e-6)
        self.assertGreater(float(np.abs(a.P[1:-1] - b.P[1:-1]).max()), 1e-4)

    def test_errors(self):
        with self.assertRaises(Exception):
            one("curve_simplify", {}, src={"type": "curve", "params": {"points": "0,0,0 1,0,0"}})              # two points: nothing to simplify


if __name__ == "__main__":
    unittest.main()
