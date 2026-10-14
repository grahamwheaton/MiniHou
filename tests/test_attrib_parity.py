"""Unit tests for the new attribute suite:
- attribute_remap (Houdini's Attribute Remap SOP)
- attribute_from_map (Houdini's Attribute from Map SOP)
- attribute_noise (Houdini's Attribute Noise 2.0 SOP)
- attribute_from_group & group_from_attribute
"""
import os
import tempfile
import unittest

import numpy as np
try:
    from PIL import Image
    HAVE_PIL = True
except ImportError:
    Image = None  # type: ignore
    HAVE_PIL = False

import minihou.nodes
from minihou import Evaluator, REGISTRY
from minihou.geometry import Geometry


def cook(node_type: str, *inputs, **params):
    nt = REGISTRY.get(node_type)
    for g in inputs:
        for x in (g if isinstance(g, list) else [g]):
            if hasattr(x, "frozen") and not x.frozen:
                x.freeze()
    out = nt.fn(*inputs, **nt.coerce_params(params))
    out.validate()
    return out


class AttributeRemapTests(unittest.TestCase):
    def setUp(self):
        # Create a grid with custom scalar attribute 'val' = [0, 10]
        self.grid = cook("grid", rows=5, columns=5, size_x=4.0, size_y=4.0)
        val = np.linspace(0.0, 10.0, self.grid.npoints, dtype=np.float32)
        self.geo = self.grid.copy()
        self.geo.point["val"] = val

    def test_basic_remap_and_compute_range(self):
        # Remap [0, 10] to [100, 200]
        res = cook("attribute_remap", self.geo, original_name="val", new_name="mapped",
                   compute_range=True, outputmin=100.0, outputmax=200.0, remap="linear")
        self.assertIn("mapped", res.point)
        mapped = res.point["mapped"]
        self.assertAlmostEqual(float(mapped.min()), 100.0, delta=1e-4)
        self.assertAlmostEqual(float(mapped.max()), 200.0, delta=1e-4)
        self.assertAlmostEqual(float(mapped[len(mapped) // 2]), 150.0, delta=1.0)

    def test_explicit_input_range_and_clamping(self):
        # Input range [2.0, 8.0], output [0, 1], clamptype='clamp'
        res = cook("attribute_remap", self.geo, original_name="val", compute_range=False,
                   inputmin=2.0, inputmax=8.0, outputmin=0.0, outputmax=1.0, clamptype="clamp")
        vals = res.point["val"]
        self.assertEqual(float(vals.min()), 0.0)
        self.assertEqual(float(vals.max()), 1.0)

        # clamptype='none' allows values outside [0, 1]
        res_noclamp = cook("attribute_remap", self.geo, original_name="val", compute_range=False,
                           inputmin=2.0, inputmax=8.0, outputmin=0.0, outputmax=1.0, clamptype="none")
        self.assertLess(float(res_noclamp.point["val"].min()), 0.0)
        self.assertGreater(float(res_noclamp.point["val"].max()), 1.0)

    def test_curve_shaping_profiles(self):
        shapes = ["linear", "smoothstep", "smootherstep", "ease_in", "ease_out", "s_curve", "invert", "bell"]
        for s in shapes:
            res = cook("attribute_remap", self.geo, original_name="val", new_name="out_" + s,
                       compute_range=True, outputmin=0.0, outputmax=1.0, useramp=True, remap=s)
            self.assertIn("out_" + s, res.point)
            arr = res.point["out_" + s]
            self.assertTrue(np.all(np.isfinite(arr)))

        # Invert shape should reverse order
        res_inv = cook("attribute_remap", self.geo, original_name="val", new_name="inv",
                       compute_range=True, outputmin=0.0, outputmax=1.0, useramp=True, remap="invert")
        self.assertAlmostEqual(float(res_inv.point["inv"][0]), 1.0, delta=1e-4)
        self.assertAlmostEqual(float(res_inv.point["inv"][-1]), 0.0, delta=1e-4)

    def test_wrap_repeat_and_ping_pong(self):
        # repeat wraps [0, 2] -> 0..1..0..1
        res_rep = cook("attribute_remap", self.geo, original_name="val", compute_range=False,
                       inputmin=0.0, inputmax=5.0, outputmin=0.0, outputmax=1.0, clamptype="repeat")
        self.assertTrue(np.all(res_rep.point["val"] >= 0.0))
        self.assertTrue(np.all(res_rep.point["val"] <= 1.0))

        # ping_pong wraps like triangle wave
        res_pp = cook("attribute_remap", self.geo, original_name="val", compute_range=False,
                      inputmin=0.0, inputmax=5.0, outputmin=0.0, outputmax=1.0, clamptype="ping_pong")
        self.assertTrue(np.all(res_pp.point["val"] >= 0.0))
        self.assertTrue(np.all(res_pp.point["val"] <= 1.0))


@unittest.skipUnless(HAVE_PIL, "Pillow is required for image map sampling tests")
class AttributeFromMapTests(unittest.TestCase):
    def setUp(self):
        self.temp_dir = tempfile.TemporaryDirectory()
        # Create a 64x64 test gradient image: red gradient horizontally, green gradient vertically
        img_arr = np.zeros((64, 64, 3), dtype=np.uint8)
        for y in range(64):
            for x in range(64):
                img_arr[y, x, 0] = int(x / 63.0 * 255)
                img_arr[y, x, 1] = int(y / 63.0 * 255)
                img_arr[y, x, 2] = 128
        img = Image.fromarray(img_arr)
        self.img_path = os.path.join(self.temp_dir.name, "test_grad.png")
        img.save(self.img_path)

        # Create a unit grid with UV coordinates
        self.grid = cook("grid", rows=10, columns=10, size_x=1.0, size_y=1.0)
        # Add planar UVs: [0, 1] x [0, 1]
        uv = np.zeros((self.grid.npoints, 3), dtype=np.float32)
        uv[:, 0] = self.grid.P[:, 0] + 0.5
        uv[:, 1] = self.grid.P[:, 1] + 0.5
        self.grid.point["uv"] = uv

    def tearDown(self):
        self.temp_dir.cleanup()

    def test_sample_rgb_from_file(self):
        res = cook("attribute_from_map", self.grid, texture_channel=self.img_path,
                   export_attribute="Cd", color_channel="rgb", srccolorspace="linear")
        self.assertIn("Cd", res.point)
        Cd = res.point["Cd"]
        self.assertEqual(Cd.shape, (self.grid.npoints, 3))
        # Red channel increases with X (UV.u)
        self.assertGreater(float(Cd[Cd[:, 0] > 0.8, 0].mean()), float(Cd[Cd[:, 0] < 0.2, 0].mean()))

    def test_sample_single_channels(self):
        # Sample luminance channel
        res_lum = cook("attribute_from_map", self.grid, texture_channel=self.img_path,
                       export_attribute="lum", color_channel="luminance")
        self.assertIn("lum", res_lum.point)
        self.assertEqual(res_lum.point["lum"].ndim, 1)

    def test_uv_transformations(self):
        # Invert U should flip red gradient
        res_inv = cook("attribute_from_map", self.grid, texture_channel=self.img_path,
                       export_attribute="Cd", color_channel="rgb", uv_invertu=True, srccolorspace="linear")
        Cd_inv = res_inv.point["Cd"]
        # With U inverted, left side has higher red
        self.assertGreater(float(Cd_inv[self.grid.P[:, 0] < -0.3, 0].mean()),
                           float(Cd_inv[self.grid.P[:, 0] > 0.3, 0].mean()))

    def test_wrap_border(self):
        # Translate UVs by (2, 0) and use border mode
        res_border = cook("attribute_from_map", self.grid, texture_channel=self.img_path,
                          export_attribute="Cd", uv_tx=(2.0, 0.0, 0.0), wrap="border",
                          borderrr=(0.25, 0.5, 0.75), srccolorspace="linear")
        Cd = res_border.point["Cd"]
        np.testing.assert_allclose(Cd[0], [0.25, 0.5, 0.75], atol=1e-3)


class AttributeNoiseTests(unittest.TestCase):
    def setUp(self):
        self.grid = cook("grid", rows=10, columns=10, size_x=4.0, size_y=4.0)

    def test_perlin_vector_noise(self):
        res = cook("attribute_noise", self.grid, attribs="Cd", attribtype="vector",
                   elementsize=1.0, amplitude=(1.0, 1.0, 1.0), noiserange="positive")
        self.assertIn("Cd", res.point)
        Cd = res.point["Cd"]
        self.assertEqual(Cd.shape, (self.grid.npoints, 3))
        self.assertTrue(np.all(Cd >= 0.0))
        self.assertTrue(np.all(Cd <= 1.0))

    def test_worley_cellular_noise(self):
        res = cook("attribute_noise", self.grid, attribs="cell", attribtype="float",
                   basis="worley", elementsize=1.0, noiserange="positive")
        self.assertIn("cell", res.point)
        cell = res.point["cell"]
        self.assertEqual(cell.ndim, 1)
        self.assertTrue(np.all(cell >= 0.0))
        self.assertTrue(np.all(cell <= 1.0))

    def test_operations_add_and_multiply(self):
        # Create base attribute
        g = self.grid.copy()
        g.point["Cd"] = np.full((g.npoints, 3), 0.5, dtype=np.float32)

        res_add = cook("attribute_noise", g, attribs="Cd", operation="add",
                       amplitude=(0.2, 0.2, 0.2), noiserange="zero_centered")
        # Added noise around 0.5 -> mean should be ~0.5, values differ from 0.5
        self.assertAlmostEqual(float(res_add.point["Cd"].mean()), 0.5, delta=0.1)
        self.assertFalse(np.allclose(res_add.point["Cd"], 0.5))

    def test_point_displacement(self):
        res = cook("attribute_noise", self.grid, attribs="P", displace=True,
                   amplitude=(0.0, 0.0, 1.0), elementsize=1.0, updatenmls=True)
        # Z positions should be displaced from 0
        self.assertGreater(float(np.abs(res.P[:, 2]).max()), 0.1)
        self.assertIn("N", res.point)


class AttributeGroupTests(unittest.TestCase):
    def setUp(self):
        self.grid = cook("grid", rows=6, columns=6, size_x=2.0, size_y=2.0)
        # Create a group selecting points with P.x > 0
        from minihou import groups
        mask = self.grid.P[:, 0] > 0.0
        self.geo = groups.write(self.grid, "point", "right_half", mask)

    def test_attribute_from_group(self):
        res = cook("attribute_from_group", self.geo, group="right_half", name="weight",
                   type="float", value=1.0, outside_value=0.0)
        self.assertIn("weight", res.point)
        w = res.point["weight"]
        self.assertTrue(np.all(w[self.geo.P[:, 0] > 0.0] == 1.0))
        self.assertTrue(np.all(w[self.geo.P[:, 0] <= 0.0] == 0.0))

    def test_group_from_attribute(self):
        from minihou import groups
        # Add scalar attribute
        g = self.grid.copy()
        g.point["temp"] = g.P[:, 1].copy()  # Y in [-1, 1]

        # Group where temp > 0.2
        res = cook("group_from_attribute", g, attribute="temp", group="hot_points",
                   operation="greater", threshold=0.2)
        self.assertIn("hot_points", groups.names(res, "point"))
        mask = groups.member(res, "point", "hot_points")
        self.assertTrue(np.all(g.P[mask, 1] > 0.2))
        self.assertTrue(np.all(g.P[~mask, 1] <= 0.2))


if __name__ == "__main__":
    unittest.main()
