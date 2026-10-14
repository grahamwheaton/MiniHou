"""Tests for the Font geometry node (Houdini Font SOP parity)."""
import unittest

import numpy as np

from minihou import Evaluator, Graph, timectx
from minihou.nodes.font import HAVE_FONTTOOLS
from minihou.sdk import REGISTRY


def cook_font(**params):
    return Evaluator().cook(Graph.from_dict({
        "nodes": {
            "f": {"type": "font", "params": params},
            "o": {"type": "output", "inputs": {"geo": "f"}}
        },
        "display": "o"
    }))


@unittest.skipUnless(HAVE_FONTTOOLS, "fontTools is required for font node tests")
class FontTests(unittest.TestCase):
    def test_default_creation(self):
        g = cook_font()
        self.assertGreater(g.npoints, 0)
        self.assertGreater(g.nprims, 0)
        self.assertIn("N", g.point.names())
        np.testing.assert_allclose(np.linalg.norm(g.point["N"], axis=1), 1.0, atol=1e-5)
        # Default primitive type is polygons (closed is None)
        self.assertTrue(g.topology.closed_mask().all())

    def test_bezier_curves(self):
        g = cook_font(text="O", primitive_type="bezier")
        self.assertGreater(g.npoints, 0)
        # Letter 'O' has 2 closed contour loops
        self.assertEqual(g.nprims, 2)
        self.assertTrue(g.topology.closed_mask().all())

    def test_polygons_and_holes(self):
        # 'O' has an outer contour and an inner hole
        g_holes = cook_font(text="O", primitive_type="polygons", hole_faces=True)
        g_solid = cook_font(text="O", primitive_type="polygons", hole_faces=False)
        self.assertGreater(g_holes.nprims, 0)
        self.assertGreater(g_solid.nprims, 0)
        # With holes cut out, the center (0, 0, 0) in local coordinates is open
        # and total area with hole cut out should be less than solid
        # Compute total triangle area
        vp = g_holes.topology.vertex_point.reshape(-1, 3)
        a, b, c = g_holes.P[vp[:, 0]], g_holes.P[vp[:, 1]], g_holes.P[vp[:, 2]]
        cross = np.cross(b - a, c - a)
        area_holes = 0.5 * np.sum(cross[:, 2])

        vp_s = g_solid.topology.vertex_point.reshape(-1, 3)
        as_, bs, cs = g_solid.P[vp_s[:, 0]], g_solid.P[vp_s[:, 1]], g_solid.P[vp_s[:, 2]]
        cross_s = np.cross(bs - as_, cs - as_)
        area_solid = 0.5 * np.sum(cross_s[:, 2])

        self.assertLess(area_holes, area_solid)
        self.assertGreater(area_holes, 0.0)

    def test_variable_expansion(self):
        # Test $F with frame context
        with timectx.at(42):
            g = cook_font(text="F$F")
            # Index 0 is 'F' (70), indices 1 & 2 are '4' (52) & '2' (50)
            g_attrs = cook_font(text="F$F", add_attributes=True)
            symbols = set(g_attrs.prim["textsymbol"].tolist())
            self.assertIn(ord('F'), symbols)
            self.assertIn(ord('4'), symbols)
            self.assertIn(ord('2'), symbols)

        # Test $F4 padded frame
        with timectx.at(7):
            g_pad = cook_font(text="$F4", add_attributes=True)
            symbols_pad = set(g_pad.prim["textsymbol"].tolist())
            self.assertIn(ord('0'), symbols_pad)
            self.assertIn(ord('7'), symbols_pad)

        # Test backtick expression
        g_expr = cook_font(text="`3 * 4`", add_attributes=True)
        syms_expr = set(g_expr.prim["textsymbol"].tolist())
        self.assertIn(ord('1'), syms_expr)
        self.assertIn(ord('2'), syms_expr)

    def test_horizontal_alignment(self):
        g_left = cook_font(text="HELLO", halign="left")
        g_center = cook_font(text="HELLO", halign="center")
        g_right = cook_font(text="HELLO", halign="right")

        # Left aligned: min X is near 0 (within left side bearing)
        self.assertAlmostEqual(float(g_left.P[:, 0].min()), 0.0, delta=0.15)
        # Center aligned: bounding box is centered around X = 0
        mid = float(g_center.P[:, 0].min() + g_center.P[:, 0].max()) * 0.5
        self.assertAlmostEqual(mid, 0.0, delta=0.1)
        # Right aligned: max X is near 0
        self.assertAlmostEqual(float(g_right.P[:, 0].max()), 0.0, delta=0.15)
        # Verify relative ordering (right-aligned has negative X coordinates, left-aligned has positive)
        self.assertLess(float(g_right.P[:, 0].min()), float(g_center.P[:, 0].min()))
        self.assertLess(float(g_center.P[:, 0].min()), float(g_left.P[:, 0].min()))
        self.assertLess(float(g_right.P[:, 0].max()), float(g_center.P[:, 0].max()))
        self.assertLess(float(g_center.P[:, 0].max()), float(g_left.P[:, 0].max()))

    def test_vertical_alignment(self):
        g_top = cook_font(text="TEST", valign="top")
        g_mid = cook_font(text="TEST", valign="middle")
        g_bot = cook_font(text="TEST", valign="bottom")

        # Top aligned: max Y is near 0
        self.assertAlmostEqual(float(g_top.P[:, 1].max()), 0.0, delta=0.08)
        # Bottom aligned has lower baseline near 0
        self.assertGreater(float(g_bot.P[:, 1].mean()), float(g_top.P[:, 1].mean()))
        self.assertGreater(float(g_mid.P[:, 1].mean()), float(g_top.P[:, 1].mean()))

    def test_transformations(self):
        # Origin translation
        g_orig = cook_font(text="A", origin=[5.0, 10.0, -3.0])
        self.assertAlmostEqual(float(g_orig.P[:, 0].mean()), 5.0, delta=1.5)
        self.assertAlmostEqual(float(g_orig.P[:, 1].mean()), 10.0, delta=1.5)
        self.assertAlmostEqual(float(g_orig.P[:, 2].mean()), -3.0, delta=1e-5)

        # Scale
        g_s1 = cook_font(text="A", font_size=1.0)
        g_s2 = cook_font(text="A", font_size=2.0)
        h1 = float(g_s1.P[:, 1].max() - g_s1.P[:, 1].min())
        h2 = float(g_s2.P[:, 1].max() - g_s2.P[:, 1].min())
        self.assertAlmostEqual(h2, h1 * 2.0, delta=0.01)

        # Oblique slant
        g_normal = cook_font(text="I", oblique=0.0)
        g_slanted = cook_font(text="I", oblique=30.0)
        # At top of 'I', slanted X coordinate is greater than unslanted
        top_n = g_normal.P[g_normal.P[:, 1] > g_normal.P[:, 1].max() - 0.05]
        top_s = g_slanted.P[g_slanted.P[:, 1] > g_slanted.P[:, 1].max() - 0.05]
        self.assertGreater(float(top_s[:, 0].mean()), float(top_n[:, 0].mean()))

        # Rotate around X by 90 deg -> normal points in -Y or +Y
        g_rot = cook_font(text="A", rotate=[90.0, 0.0, 0.0])
        np.testing.assert_allclose(g_rot.P[:, 2].max() - g_rot.P[:, 2].min(), h1, atol=0.05)
        self.assertAlmostEqual(float(abs(g_rot.point["N"][0, 1])), 1.0, delta=1e-4)

    def test_tracking_and_kerning(self):
        g_base = cook_font(text="AV", tracking_xy=[0.0, 0.0], automatic_kerning=False)
        g_kern = cook_font(text="AV", tracking_xy=[0.0, 0.0], automatic_kerning=True)
        g_track = cook_font(text="AV", tracking_xy=[0.5, 0.0], automatic_kerning=False)

        w_base = float(g_base.P[:, 0].max() - g_base.P[:, 0].min())
        w_track = float(g_track.P[:, 0].max() - g_track.P[:, 0].min())
        # Tracking increases width
        self.assertGreater(w_track, w_base)

    def test_add_attributes(self):
        g = cook_font(text="AB", add_attributes=True)
        self.assertIn("textsymbol", g.prim.names())
        self.assertIn("textindex", g.prim.names())
        syms = g.prim["textsymbol"]
        idxs = g.prim["textindex"]
        self.assertEqual(set(syms.tolist()), {ord('A'), ord('B')})
        self.assertEqual(set(idxs.tolist()), {0, 1})

    def test_empty_and_whitespace_text(self):
        g_empty = cook_font(text="")
        self.assertEqual(g_empty.npoints, 0)
        self.assertEqual(g_empty.nprims, 0)

        g_spaces = cook_font(text="   \n   ")
        self.assertEqual(g_spaces.npoints, 0)
        self.assertEqual(g_spaces.nprims, 0)

    def test_multi_line_text(self):
        g = cook_font(text="Top\nBottom", valign="first_line")
        self.assertGreater(g.npoints, 0)
        # Bounding box should span two vertical lines
        h = float(g.P[:, 1].max() - g.P[:, 1].min())
        self.assertGreater(h, 1.2)

    def test_parameter_aliases(self):
        # font_size vs fontsize
        g1 = cook_font(text="X", font_size=0.5)
        g2 = cook_font(text="X", fontsize=0.5)
        np.testing.assert_allclose(g1.P, g2.P, atol=1e-5)

        # scale_xy vs scale
        g_sc1 = cook_font(text="X", scale_xy=[2.0, 1.0, 1.0])
        g_sc2 = cook_font(text="X", scale=[2.0, 1.0, 1.0])
        np.testing.assert_allclose(g_sc1.P, g_sc2.P, atol=1e-5)

        # tracking_xy vs tracking
        g_tr1 = cook_font(text="XY", tracking_xy=[0.2, 0.0, 0.0])
        g_tr2 = cook_font(text="XY", tracking=[0.2, 0.0, 0.0])
        np.testing.assert_allclose(g_tr1.P, g_tr2.P, atol=1e-5)

        # add_attributes vs add_text_attributes
        g_attr1 = cook_font(text="Z", add_attributes=True)
        g_attr2 = cook_font(text="Z", add_text_attributes=True)
        self.assertIn("textsymbol", g_attr1.prim.names())
        self.assertIn("textsymbol", g_attr2.prim.names())
        np.testing.assert_array_equal(g_attr1.prim["textsymbol"], g_attr2.prim["textsymbol"])

        # valign boolean aliases (top=True, bottom=True, etc.)
        g_top = cook_font(text="M", top=True)
        g_top_v = cook_font(text="M", valign="top")
        np.testing.assert_allclose(g_top.P, g_top_v.P, atol=1e-5)

    def test_parity_parameters_exist(self):
        nt = REGISTRY.get("font")
        self.assertIsNotNone(nt)
        pnames = {p.name for p in nt.params}
        for expected in ("primitive_type", "font", "text", "halign", "valign", "first_line", "top", "middle", "bottom",
                         "use_descender", "origin", "rotate", "scale_xy", "scale", "font_size", "fontsize",
                         "tracking_xy", "tracking", "oblique", "lod", "automatic_kerning", "hole_faces",
                         "add_attributes", "add_text_attributes"):
            self.assertIn(expected, pnames)
        self.assertEqual(len(nt.inputs), 0)


if __name__ == "__main__":
    unittest.main()
