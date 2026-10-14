"""Guides (outlines of the regions a node works on) and the handles declared on more nodes."""
import json
import unittest

import numpy as np

from minihou import Evaluator, Guide, Handle, Registry, node
from minihou import handles
from minihou.sdk import Float, Vec3, Bool, Enum
from tests.test_nodes import run

REG = Evaluator().registry


def items(node_type, ins, out, **kw):
    params = REG.get(node_type).coerce_params(kw)
    return handles.handles_for(node_type, ins, out, params, REG.get(node_type))


def guides_of(hs):
    return {h["id"].split(":", 1)[1]: h for h in hs if h["kind"] == "guide"}


def extent(h):
    pts = np.vstack([np.asarray(l) for l in h["lines"]])
    return pts.min(0), pts.max(0)


class GuideShapeTests(unittest.TestCase):
    def test_soft_transform_shows_its_falloff_sphere(self):
        src = run("box")
        hs = items("soft_transform", [src], src, center=[1, 2, 3], radius=0.5)
        g = guides_of(hs)["region"]
        lo, hi = extent(g)
        np.testing.assert_allclose(lo, [0.5, 1.5, 2.5], atol=1e-9)
        np.testing.assert_allclose(hi, [1.5, 2.5, 3.5], atol=1e-9)
        self.assertEqual(len(g["lines"]), 3)                                           # three great circles
        json.dumps(hs)                                                                 # it travels as JSON

    def test_clip_and_mirror_planes_follow_the_normal_and_origin(self):
        src = run("box", size=[2, 2, 2])
        for t in ("clip", "mirror"):
            hs = items(t, [src], src, origin=[0, 0, 0.5], normal=[0, 0, 1])
            g = guides_of(hs)["plane"]
            ring = np.asarray(g["lines"][0])
            np.testing.assert_allclose(ring[:, 2], 0.5, atol=1e-9)                     # a square lying in z = 0.5
            self.assertGreater(float(np.ptp(ring[:, 0])), 0.5)
            tick = np.asarray(g["lines"][1])
            self.assertGreater(float(tick[1, 2] - tick[0, 2]), 0)                      # and a tick along the normal
            self.assertEqual(sorted(h["id"] for h in hs if h["kind"] == "arrow"), ["origin_x", "origin_y", "origin_z"])

    def test_bend_marks_its_capture_region(self):
        src = run("box")
        hs = items("bend", [src], src, origin=[0, 0, 1], direction=[0, 0, 1], length=3)
        g = guides_of(hs)
        np.testing.assert_allclose(g["region"]["lines"][0], [[0, 0, 1], [0, 0, 4]], atol=1e-9)
        self.assertIn("start", g)

    def test_twist_squash_taper_and_revolve_show_their_axes(self):
        src = run("box")
        for t, kw, key in (("twist", dict(origin=[1, 0, 0], direction=[0, 1, 0]), "axis"),
                           ("squash_stretch", dict(center=[0, 0, 0], direction=[1, 0, 0]), "axis"),
                           ("linear_taper", dict(origin=[0, 0, 0], direction=[0, 1, 0], length=2), "axis"),
                           ("revolve", dict(origin=[0, 0, 0], axis=[0, 1, 0]), "axis")):
            hs = items(t, [src], src, **kw)
            self.assertIn(key, guides_of(hs), t)
            self.assertTrue(any(h["kind"] in ("ring", "scale", "arrow") for h in hs), t)

    def test_when_hides_a_guide_until_its_condition_holds(self):
        src = run("box")
        self.assertNotIn("target", guides_of(items("match_size", [src], src, scale=False)))
        g = guides_of(items("match_size", [src], src, scale=True, center=[0, 0, 0], size=[2, 4, 6]))["target"]
        lo, hi = extent(g)
        np.testing.assert_allclose(hi - lo, [2, 4, 6], atol=1e-9)


class GroupCreateRegionTests(unittest.TestCase):
    def test_the_bounding_box_shows_for_the_box_method_and_the_filter(self):
        src = run("grid")
        kw = dict(box_center=[1, 1, 0], box_size=[2, 2, 2])
        self.assertEqual(set(guides_of(items("group_create", [src], src, method="pattern", **kw))), set())
        self.assertEqual(set(guides_of(items("group_create", [src], src, method="bounding_box", **kw))), {"region"})
        self.assertEqual(set(guides_of(items("group_create", [src], src, method="filters", bounds_enable=True, **kw))), {"region_f"})
        self.assertEqual(set(guides_of(items("group_create", [src], src, method="filters", bounds_enable=True, bounds_type="sphere", **kw))), {"region_s"})
        hs = items("group_create", [src], src, method="bounding_box", **kw)
        self.assertEqual(len([h for h in hs if h["kind"] == "arrow"]), 6)               # centre and size arrows

    def test_a_connected_bounding_object_replaces_the_parameters(self):
        src = run("grid")
        obj = run("box", size=[2, 2, 2], center=[5, 5, 5])
        hs = items("group_create", [src, obj], src, method="bounding_box", box_center=[0, 0, 0], box_size=[1, 1, 1])
        lo, hi = extent(guides_of(hs)["region"])
        np.testing.assert_allclose(lo, [4, 4, 4], atol=1e-6)
        np.testing.assert_allclose(hi, [6, 6, 6], atol=1e-6)
        self.assertFalse([h for h in hs if h["kind"] == "arrow"])                      # their parameters are ignored now: no arrows


class DeclaredHandlesTests(unittest.TestCase):
    def test_new_handles_edit_the_parameters_they_name(self):
        src = run("box")
        cases = {"circle": ["radius"], "grid": ["size_x", "size_y"], "torus": ["radius"], "soft_transform": ["radius", "translate", "rotate", "scale"],
                 "twist": ["strength"], "squash_stretch": ["amount"], "linear_taper": ["length"], "revolve": ["angle", "origin"],
                 "uv_project": ["center", "rotate", "scale"], "copy_transform": ["translate"]}
        for t, params in cases.items():
            hs = items(t, [src], src)
            names = {h["param"] for h in hs if h["kind"] != "guide"}
            for p in params:
                self.assertIn(p, names, t)

    def test_circle_radius_arrow_rides_on_the_circle(self):
        src = run("circle", radius=2.0, center=[1, 0, 0])
        h = by(items("circle", [], src, radius=2.0, center=[1, 0, 0]), "radius")
        np.testing.assert_allclose(h["origin"], [3, 0, 0], atol=1e-9)                  # centre + radius along x

    def test_grid_arrows_stand_on_the_edges(self):
        src = run("grid", size_x=4, size_y=6)
        hs = items("grid", [], src, size_x=4, size_y=6)
        np.testing.assert_allclose(by(hs, "size_x")["origin"], [2, 0, 0], atol=1e-9)
        np.testing.assert_allclose(by(hs, "size_y")["origin"], [0, 3, 0], atol=1e-9)


def by(hs, param):
    return next(h for h in hs if h["param"] == param)


class DeclarationTests(unittest.TestCase):
    def reg(self):
        return Registry()

    def test_bad_guides_are_refused_when_the_node_is_declared(self):
        reg = self.reg()
        for guide, why in ((Guide("hexagon"), "shape"), (Guide("box", size="@nope"), "names no parameter"),
                           (Guide("box", when="nope"), "names no parameter"), (Guide("box", when="mode"), "not a Bool")):
            with self.subTest(why):
                with self.assertRaisesRegex(ValueError, why):
                    @node("g_bad", params=[Vec3("size", (1, 1, 1)), Enum("mode", ["a", "b"])], guides=[guide], registry=reg)
                    def g_bad(size, mode):
                        return None
        with self.assertRaisesRegex(ValueError, "unique"):
            @node("g_dup", params=[Vec3("size", (1, 1, 1))], guides=[Guide("box"), Guide("box")], registry=reg)
            def g_dup(size):
                return None

    def test_a_declared_node_shows_its_guides_and_conditional_handles(self):
        reg = self.reg()

        @node("g_ok", params=[Float("radius", 1.0), Bool("show", True), Enum("mode", ["a", "b"])],
              handles=[Handle("radius", axis="x", at="origin", reach=True, when="mode=b")],
              guides=[Guide("circle", at="origin", size="@radius", when="show"), Guide("bounds", id="bb", when="mode!=a")], registry=reg)
        def g_ok(radius, show, mode):
            return run("box")

        nt = reg.get("g_ok")
        out = run("box")
        self.assertTrue(handles.has_handles("g_ok", nt))
        hs = handles.handles_for("g_ok", [], out, {"radius": 2.0, "show": True, "mode": "a"}, nt)
        self.assertEqual([h["kind"] for h in hs], ["guide"])                            # mode a: no arrow, no bounds guide
        hs = handles.handles_for("g_ok", [], out, {"radius": 2.0, "show": False, "mode": "b"}, nt)
        self.assertEqual(sorted(h["kind"] for h in hs), ["arrow", "guide"])
        self.assertEqual(sorted(guides_of(hs)), ["bb"])

    def test_guide_points_are_capped_and_plain_numbers(self):
        g = handles.guide("big", "big", [np.zeros((100000, 3))])
        out = handles._clean(g)
        self.assertLessEqual(len(out["lines"][0]), handles.MAX_GUIDE_POINTS)
        json.dumps(out)


if __name__ == "__main__":
    unittest.main()
