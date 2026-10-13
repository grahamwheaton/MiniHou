"""Images in the graph: pixel grids, colour spaces, baking, PNG files, the nodes, and the link messages (a client plays the add-on)."""
import os
import struct
import tempfile
import time
import unittest
import zlib

import numpy as np

import minihou  # noqa: F401
from minihou import Evaluator, Graph, images
from minihou.inputs import INPUTS
from minihou.nodes.image_nodes import build_image_manifest
from tests.test_link import Harness, wait_for


def ramp(h=4, w=6):
    """Pixels whose red goes left to right and green bottom to top, so orientation mistakes show."""
    pix = np.ones((h, w, 4), np.float32)
    pix[..., 0] = np.linspace(0, 1, w)[None, :]
    pix[..., 1] = np.linspace(0, 1, h)[:, None]
    pix[..., 2] = 0.25
    return pix


def cook(nodes, target):
    return Evaluator().cook(Graph.from_dict({"nodes": nodes, "display": target}, check=False), target)


class PixelGridTests(unittest.TestCase):
    def test_a_grid_has_a_point_per_pixel_row_by_row_from_the_bottom_left(self):
        pix = ramp(3, 5)
        g = images.grid_from_pixels(pix, size=10.0)
        self.assertEqual((g.npoints, g.nprims), (15, 2 * 4))
        self.assertEqual((g.detail["grid_columns"], g.detail["grid_rows"]), (5, 3))
        np.testing.assert_allclose(g.point["Cd"].reshape(3, 5, 3)[..., 0], pix[..., 0])             # pixel (row, col) is point row * 5 + col
        np.testing.assert_allclose(g.P[0], [-5.0, -3.0, 0.0])                                         # bottom left; height follows the aspect
        np.testing.assert_allclose(g.P[-1], [5.0, 3.0, 0.0])
        self.assertNotIn("Alpha", g.point)

    def test_alpha_is_kept_only_when_it_is_not_opaque(self):
        pix = ramp()
        pix[..., 3] = 0.5
        self.assertIn("Alpha", images.grid_from_pixels(pix).point)

    def test_height_lifts_the_points_and_is_stored(self):
        pix = ramp(4, 4)
        g = images.grid_from_pixels(pix, height="red", height_scale=2.0)
        np.testing.assert_allclose(g.P[:, 2], g.point["height"] * 2.0)
        np.testing.assert_allclose(g.point["height"].reshape(4, 4), pix[..., 0])
        lum = images.grid_from_pixels(pix, height="luminance").point["height"]
        self.assertTrue(np.all(lum >= 0) and lum.max() <= 1.0)

    def test_tiny_and_huge_images_are_refused(self):
        with self.assertRaises(ValueError):
            images.grid_from_pixels(np.ones((1, 5, 4), np.float32))
        with self.assertRaises(ValueError):
            images.grid_from_pixels(np.ones((5000, 5000, 4), np.float32))

    def test_downsample_averages_blocks_and_never_enlarges(self):
        pix = np.zeros((8, 8, 4), np.float32)
        pix[::2] = 1.0
        small = images.downsample(pix, 4)
        self.assertEqual(small.shape, (4, 4, 4))
        np.testing.assert_allclose(small, 0.5)
        self.assertIs(images.downsample(pix, 8), pix)
        self.assertEqual(images.downsample(np.zeros((9, 4, 4), np.float32), 4).shape[:2], (3, 1))

    def test_srgb_conversions_are_inverses_and_known_at_the_ends(self):
        a = np.linspace(0, 1, 11, dtype=np.float32)
        np.testing.assert_allclose(images.linear_to_srgb(images.srgb_to_linear(a)), a, atol=1e-5)
        self.assertAlmostEqual(float(images.srgb_to_linear(np.float32(0.5))), 0.2140, places=3)


class RasterTests(unittest.TestCase):
    def test_channels_by_width_and_normalise(self):
        np.testing.assert_allclose(images.channels(np.array([0.0, 2.0, 4.0]), normalize=True)[:, 0], [0, 0.5, 1])
        self.assertEqual(images.channels(np.zeros((2, 2)))[0].tolist(), [0, 0, 0, 1])
        self.assertEqual(images.channels(np.ones((1, 3)))[0].tolist(), [1, 1, 1, 1])
        self.assertEqual(images.channels(np.array([[.1, .2, .3, .4]]))[0].tolist(), pytest_approx([.1, .2, .3, .4]))

    def test_a_pixel_grid_goes_back_to_the_same_pixels(self):
        pix = ramp(5, 7)
        g = images.grid_from_pixels(pix, size=3.0)
        back = images.pixels_from_lattice(g, g.point["Cd"])
        np.testing.assert_allclose(back[..., :3], pix[..., :3])

    def test_a_geometry_that_is_not_a_lattice_says_so(self):
        g = cook({"s": {"type": "scatter", "inputs": {"geo": "g"}, "params": {"count": 20}}, "g": {"type": "grid"}}, "s")
        with self.assertRaises(ValueError) as cm:
            images.lattice(g)
        self.assertIn("bake", str(cm.exception))

    def test_bake_bins_points_by_position_and_leaves_empty_pixels_transparent(self):
        g = images.grid_from_pixels(np.ones((2, 2, 4), np.float32), size=2.0)
        P = np.array([[0.0, 0.0, 0.0], [10.0, 10.0, 0.0], [0.1, 0.1, 0.0]], np.float32)
        from minihou.geometry import Geometry
        pts = Geometry.from_arrays(P)
        out = images.bake(pts, np.array([1.0, 3.0, 5.0]), 4, 4)
        self.assertEqual(out.shape, (4, 4, 4))
        self.assertAlmostEqual(float(out[0, 0, 0]), 3.0)                            # two points share the first pixel: mean
        self.assertAlmostEqual(float(out[3, 3, 0]), 3.0)
        self.assertEqual(float(out[1, 1, 3]), 0.0)                                  # nothing there
        self.assertEqual(float(out[0, 0, 3]), 1.0)


def pytest_approx(vals):
    return [float(np.float32(v)) for v in vals]


def filtered_png(pix8: np.ndarray, filter_type: int) -> bytes:
    """A PNG of an 8-bit RGBA array (rows from the top) whose every row uses the given filter."""
    h, w, c = pix8.shape
    rows = pix8.reshape(h, w * c).astype(np.int32)
    prev = np.zeros(w * c, np.int32)
    body = b""
    for y in range(h):
        cur = rows[y]
        out = np.zeros(w * c, np.int32)
        for x in range(w * c):
            a = cur[x - c] if x >= c else 0
            b = prev[x]
            cc = prev[x - c] if x >= c else 0
            if filter_type == 0:
                pred = 0
            elif filter_type == 1:
                pred = a
            elif filter_type == 2:
                pred = b
            elif filter_type == 3:
                pred = (a + b) >> 1
            else:
                p = a + b - cc
                pa, pb, pc = abs(p - a), abs(p - b), abs(p - cc)
                pred = a if (pa <= pb and pa <= pc) else (b if pb <= pc else cc)
            out[x] = (cur[x] - pred) & 255
        body += bytes([filter_type]) + out.astype(np.uint8).tobytes()
        prev = cur

    def chunk(kind, data):
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data) & 0xFFFFFFFF)
    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 6, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(body)) + chunk(b"IEND", b""))


class PngTests(unittest.TestCase):
    def test_eight_and_sixteen_bit_round_trips_with_the_built_in_reader(self):
        pix = np.random.default_rng(1).random((7, 5, 4)).astype(np.float32)
        for bits, tol in ((8, 0.005), (16, 1e-4)):
            with self.subTest(bits=bits):
                back = images.png_decode(images.png_encode(pix, bits, srgb=True), srgb=True, use_pillow=False)
                self.assertEqual(back.shape, pix.shape)
                self.assertLess(float(np.abs(back - pix).max()), tol)

    def test_rows_run_bottom_up_like_blender_so_the_file_is_not_upside_down(self):
        pix = np.zeros((3, 2, 4), np.float32)
        pix[0, :, 0] = 1.0                                                    # the bottom row is red
        pix[..., 3] = 1.0
        data = images.png_encode(pix, 8, srgb=False)
        top_first = images.png_decode(data, srgb=False, use_pillow=False)
        np.testing.assert_allclose(top_first, pix, atol=1 / 255)
        try:
            from PIL import Image
            import io
            im = np.asarray(Image.open(io.BytesIO(data)).convert("RGBA"))
            self.assertEqual(int(im[-1, 0, 0]), 255)                          # the last row of the file is the bottom of the image
        except ImportError:
            pass

    def test_the_built_in_reader_handles_every_png_filter(self):
        rng = np.random.default_rng(3)
        pix8 = rng.integers(0, 256, (6, 5, 4), dtype=np.uint8)
        want = pix8[::-1].astype(np.float32) / 255.0
        for f in range(5):
            with self.subTest(filter=f):
                got = images.png_decode(filtered_png(pix8, f), srgb=False, use_pillow=False)
                np.testing.assert_allclose(got, want, atol=1e-6)

    def test_pillow_and_the_built_in_reader_agree(self):
        try:
            import PIL  # noqa: F401
        except ImportError:
            self.skipTest("Pillow not installed")
        data = images.png_encode(ramp(5, 6), 8, srgb=True)
        np.testing.assert_allclose(images.png_decode(data, True, use_pillow=True), images.png_decode(data, True, use_pillow=False), atol=1e-6)

    def test_damaged_files_are_a_clear_error(self):
        with self.assertRaises(ValueError):
            images.png_decode(b"not a png", use_pillow=False)
        with self.assertRaises(ValueError):
            images.png_decode(images.png_encode(ramp())[:30], use_pillow=False)


class NodeTests(unittest.TestCase):
    def setUp(self):
        INPUTS.clear()
        self.addCleanup(INPUTS.clear)
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)

    def test_blender_image_makes_a_grid_and_waits_for_the_pixels(self):
        node = {"i": {"type": "blender_image", "params": {"image": "Height", "size": 4.0}}}
        with self.assertRaises(Exception) as cm:
            cook(node, "i")
        self.assertIn("waiting for Blender image 'Height'", str(cm.exception))
        INPUTS.put_image("Height", ramp(4, 6), {"is_float": True, "colorspace": "Linear Rec.709"})
        g = cook(node, "i")
        self.assertEqual((g.detail["grid_columns"], g.detail["grid_rows"], g.detail["image"]), (6, 4, "Height"))
        INPUTS.put_image("Height", None, error="the image has no pixels")
        with self.assertRaises(Exception) as cm:
            cook(node, "i")
        self.assertIn("the image has no pixels", str(cm.exception))

    def test_eight_bit_srgb_images_become_linear_unless_stored_is_asked_for(self):
        pix = np.full((4, 4, 4), 0.5, np.float32)
        INPUTS.put_image("Tex", pix, {"is_float": False, "colorspace": "sRGB"})
        auto = cook({"i": {"type": "blender_image", "params": {"image": "Tex"}}}, "i").point["Cd"]
        stored = cook({"i": {"type": "blender_image", "params": {"image": "Tex", "colorspace": "stored"}}}, "i").point["Cd"]
        np.testing.assert_allclose(stored, 0.5)
        np.testing.assert_allclose(auto, images.srgb_to_linear(np.float32(0.5)), atol=1e-6)
        INPUTS.put_image("Tex", pix, {"is_float": True, "colorspace": "sRGB"})                  # float buffers are already linear
        np.testing.assert_allclose(cook({"i": {"type": "blender_image", "params": {"image": "Tex"}}}, "i").point["Cd"], 0.5)

    def test_new_pixels_recook(self):
        node = {"i": {"type": "blender_image", "params": {"image": "T"}}}
        INPUTS.put_image("T", np.zeros((3, 3, 4), np.float32), {"is_float": True})
        ev = Evaluator()
        g = Graph.from_dict({"nodes": node, "display": "i"}, check=False)
        a = ev.cook(g)
        INPUTS.put_image("T", np.ones((3, 3, 4), np.float32), {"is_float": True})
        b = ev.cook(g)
        self.assertNotEqual(float(a.point["Cd"].max()), float(b.point["Cd"].max()))

    def test_image_file_reads_a_png_and_recooks_when_it_changes(self):
        path = os.path.join(self.tmp.name, "h.png")
        images.write_png(path, ramp(5, 8), bits=16, srgb=False)
        node = {"f": {"type": "image_file", "params": {"file": path, "colorspace": "stored", "height": "red", "height_scale": 2.0}}}
        g = cook(node, "f")
        self.assertEqual((g.detail["grid_columns"], g.detail["grid_rows"]), (8, 5))
        np.testing.assert_allclose(g.P[:8, 2], np.linspace(0, 1, 8) * 2.0, atol=1e-4)
        with self.assertRaises(Exception) as cm:
            cook({"f": {"type": "image_file", "params": {"file": os.path.join(self.tmp.name, "nope.png")}}}, "f")
        self.assertIn("no such file", str(cm.exception))

    def test_a_height_image_goes_through_the_heightfield_nodes(self):
        rng = np.random.default_rng(5)
        pix = np.ones((24, 24, 4), np.float32)
        pix[..., :3] = rng.random((24, 24, 1))
        INPUTS.put_image("H", pix, {"is_float": True})
        g = cook({"i": {"type": "blender_image", "params": {"image": "H", "height": "red", "height_scale": 3.0}},
                  "e": {"type": "heightfield_erode", "inputs": {"geo": "i"}, "params": {"iterations": 2, "droplets": 200}}}, "e")
        self.assertEqual(g.npoints, 24 * 24)

    def test_image_out_builds_a_manifest_with_the_pixels(self):
        INPUTS.put_image("In", ramp(4, 5), {"is_float": True})
        geo = cook({"i": {"type": "blender_image", "params": {"image": "In", "colorspace": "stored"}}}, "i")
        nt = minihou.sdk.REGISTRY.get("image_out")
        params = nt.coerce_params({"image": "Out", "encoding": "linear"})
        header, arrays = build_image_manifest("io", params, geo, "k")
        self.assertEqual((header["type"], header["mode"], header["image"], header["width"], header["height"]), ("scene_objects", "image", "Out", 5, 4))
        np.testing.assert_allclose(arrays["pixels"][..., :3], ramp(4, 5)[..., :3], atol=1e-6)
        srgb = build_image_manifest("io", nt.coerce_params({"image": "Out", "encoding": "srgb"}), geo, "k")[1]["pixels"]
        np.testing.assert_allclose(srgb[..., :3], images.linear_to_srgb(ramp(4, 5)[..., :3]), atol=1e-6)

    def test_image_out_bakes_points_writes_a_png_and_says_what_is_missing(self):
        geo = cook({"g": {"type": "grid", "params": {"columns": 20, "rows": 20}}, "n": {"type": "noise", "inputs": {"geo": "g"}, "params": {}}}, "n")
        nt = minihou.sdk.REGISTRY.get("image_out")
        path = os.path.join(self.tmp.name, "out", "bake.png")
        params = nt.coerce_params({"mode": "bake", "source": "P", "width": 16, "height": 8, "file": path, "encoding": "data"})
        header, arrays = build_image_manifest("io", params, geo, "k")
        self.assertEqual(arrays["pixels"].shape, (8, 16, 4))
        self.assertTrue(os.path.isfile(path))
        self.assertEqual(images.read_png(path, srgb=False).shape, (8, 16, 4))
        with self.assertRaises(ValueError) as cm:
            build_image_manifest("io", nt.coerce_params({"source": "nope"}), geo, "k")
        self.assertIn("no point attribute 'nope'", str(cm.exception))

    def test_image_out_is_a_scene_sink_and_passes_geometry_through(self):
        g = Graph.from_dict({"nodes": {"g": {"type": "grid"}, "o": {"type": "image_out", "inputs": {"geo": "g"}, "params": {"image": "X", "source": "uv"}},
                                       "out": {"type": "output", "inputs": {"geo": "g"}}}, "display": "out"})
        self.assertEqual(g.scene_sinks(), {"o": "o"})
        ev = Evaluator()
        self.assertIs(ev.cook(g, "o"), ev.cook(g, "g"))


class LinkTests(unittest.TestCase):
    def setUp(self):
        INPUTS.clear()
        self.addCleanup(INPUTS.clear)

    def graph(self):
        return Graph.from_dict({"nodes": {
            "i": {"type": "blender_image", "params": {"image": "Mask", "colorspace": "stored", "height": "red", "height_scale": 2}},
            "o": {"type": "output", "inputs": {"geo": "i"}}}, "display": "o"}, check=False)

    def test_the_engine_asks_for_images_and_cooks_what_arrives(self):
        h = Harness(self.graph())
        self.addCleanup(h.close)
        c, q = h.client()
        hdr, _ = wait_for(q, "watch_objects")
        self.assertEqual(hdr["images"], ["Mask"])
        self.assertEqual(hdr["objects"], [])
        pix = ramp(4, 6)
        self.assertTrue(c.send({"type": "image_data", "image": "Mask", "width": 6, "height": 4, "is_float": True, "colorspace": "Non-Color"},
                               {"pixels": pix}))
        end = time.monotonic() + 5
        while time.monotonic() < end:
            h.session.wait_idle(1.0)
            if h.session.status().get("ok"):
                break
            time.sleep(0.02)
        geo = Evaluator().cook(h.session.graph)
        self.assertEqual(geo.npoints, 24)
        np.testing.assert_allclose(geo.point["height"].reshape(4, 6), pix[..., 0], atol=1e-6)
        c.send({"type": "image_missing", "image": "Mask", "reason": "no image named 'Mask' in the Blender file"})
        end = time.monotonic() + 5
        st = {}
        while time.monotonic() < end:
            h.session.wait_idle(1.0)
            st = h.session.status()
            if st.get("ok") is False and "no image named" in str(st.get("error", {}).get("message", "")):
                break
            time.sleep(0.02)
        self.assertIn("no image named", st["error"]["message"])

    def test_image_out_reaches_the_client_as_a_scene_message_with_pixels(self):
        INPUTS.put_image("In", ramp(3, 4), {"is_float": True})
        g = Graph.from_dict({"nodes": {
            "i": {"type": "blender_image", "params": {"image": "In", "colorspace": "stored"}},
            "o": {"type": "image_out", "inputs": {"geo": "i"}, "params": {"image": "Result", "encoding": "linear"}},
            "out": {"type": "output", "inputs": {"geo": "i"}}}, "display": "out"}, check=False)
        h = Harness(g)
        self.addCleanup(h.close)
        c, q = h.client()
        hdr, arrays = wait_for(q, "scene_objects", timeout=8, pred=lambda m: m.get("mode") == "image")
        self.assertEqual((hdr["image"], hdr["width"], hdr["height"]), ("Result", 4, 3))
        np.testing.assert_allclose(arrays["pixels"].reshape(3, 4, 4)[..., 0], ramp(3, 4)[..., 0], atol=1e-6)


if __name__ == "__main__":
    unittest.main()
