"""SDF field kernels against the unmodified numpy oracle (no relative tolerance)."""
import unittest

import numpy as np

from minihou import Evaluator, Graph, rust, sdf


@unittest.skipUnless(rust.available(), "Rust extension is not installed")
class RustSdfFieldTests(unittest.TestCase):
    def points_case(self, points, radius, h, blend):
        with rust.disabled():
            expected = sdf.from_points(points, radius, h, blend)
        actual = rust.sdf_from_points(points, radius, h, blend)
        np.testing.assert_array_equal(actual.origin, expected.origin)
        self.assertEqual(actual.h, expected.h)
        self.assertEqual(actual.values.dtype, expected.values.dtype)
        # Preserve NumPy power, chunked add.at order, and pairwise radius mean.
        np.testing.assert_array_equal(actual.values, expected.values)

    def test_from_points_single_huge_voxels_zero_blend(self):
        for h in (0.13, 100.0):
            for blend in (-1.0, 0.0, 0.37, 1.0, 2.1):
                with self.subTest(h=h, blend=blend):
                    self.points_case([[0.17, -0.32, 0.09]], 0.4, h, blend)

    def test_from_points_random_clouds(self):
        rng = np.random.default_rng(81236)
        for count in (2, 9, 130):
            with self.subTest(count=count):
                self.points_case(rng.normal(0, 0.2, (count, 3)), rng.uniform(0.05, 0.2, count), 0.12, 0.73)

    def test_from_points_node_shapes(self):
        for kind, params in (("box", {}), ("sphere", {"rows": 6, "columns": 8}), ("torus", {})):
            with self.subTest(kind=kind), rust.disabled():
                graph = Graph.from_dict({"version": 1, "nodes": {"s": {"type": kind, "params": params}}, "display": "s"})
                points = Evaluator().cook(graph, "s").P
            self.points_case(points, 0.12, 0.2, 1.0)

    def test_from_points_empty_and_zero_radius(self):
        for function in (sdf.from_points, rust.sdf_from_points):
            with rust.disabled(), self.assertRaises(ValueError):
                function(np.empty((0, 3)), np.empty(0), 0.2)
        with np.errstate(divide="ignore", invalid="ignore"):
            self.points_case([[0.0, 0.0, 0.0]], 0.0, 0.2, 1.0)

    def test_blur_exact(self):
        rng = np.random.default_rng(785)
        for shape in ((1, 1, 1), (1, 3, 4), (9, 7, 5), (0, 2, 3)):
            for iterations in (0, 1, 3):
                if 0 in shape and iterations:
                    continue  # numpy edge padding of an empty axis raises
                with self.subTest(shape=shape, iterations=iterations):
                    vol = sdf.Volume(np.array([1., 2., 3.]), 0.25, rng.normal(size=shape).astype(np.float32))
                    with rust.disabled():
                        expected = sdf.blur(vol, iterations)
                    actual = rust.sdf_blur(vol, iterations)
                    np.testing.assert_array_equal(actual.values, expected.values)
                    self.assertEqual(actual.values.dtype, expected.values.dtype)
                    np.testing.assert_array_equal(actual.origin, expected.origin)
                    self.assertEqual(actual.h, expected.h)

    def test_resample_exact(self):
        vol = sdf.Volume(np.array([1., 2., 3.]), 0.25, np.arange(24, dtype=np.float32).reshape(2, 3, 4))
        for offset in ((0, 0, 0), (-1, 1, -2), (20, 0, 0), (0.5, 1.5, -0.5), (-1.5, -2.5, 0.5)):
            for dims in ((1, 1, 1), (3, 4, 5), (0, 3, 4)):
                with self.subTest(offset=offset, dims=dims):
                    origin = vol.origin + np.array(offset) * vol.h
                    with rust.disabled():
                        expected = sdf.resample_to(vol, origin, dims, vol.h, -7.25)
                    actual = rust.sdf_resample_to(vol, origin, dims, vol.h, -7.25)
                    np.testing.assert_array_equal(actual, expected)
                    self.assertEqual(actual.dtype, expected.dtype)

    def test_smooth_min_exact_dtype_and_broadcast(self):
        rng = np.random.default_rng(369)
        for dtype_a, dtype_b in ((np.float32, np.float32), (np.float64, np.float64), (np.float32, np.float64)):
            a = rng.normal(size=(5, 1, 7)).astype(dtype_a)
            b = rng.normal(size=(1, 4, 7)).astype(dtype_b)
            for k in (-1.0, 0.0, 0.01, 0.37, 3.0):
                with self.subTest(dtype_a=dtype_a, dtype_b=dtype_b, k=k):
                    with rust.disabled():
                        expected = sdf.smooth_min(a, b, k)
                    actual = rust.sdf_smooth_min(a, b, k)
                    self.assertEqual(actual.dtype, expected.dtype)
                    np.testing.assert_array_equal(actual, expected)

    def test_smooth_min_nan_signed_zero_empty_scalars(self):
        for dtype in (np.float32, np.float64):
            cases = [(np.array([np.nan, 0., -0., 2.], dtype), np.array([1., -0., 0., np.nan], dtype)),
                     (np.empty((0, 3), dtype), np.array(1., dtype)),
                     (np.array(0.2, dtype), np.array(0.3, dtype))]
            for a, b in cases:
                for k in (0., 0.3):
                    with self.subTest(dtype=dtype, shape=a.shape, k=k), np.errstate(invalid="ignore"):
                        with rust.disabled():
                            expected = sdf.smooth_min(a, b, k)
                        actual = rust.sdf_smooth_min(a, b, k)
                        self.assertEqual(np.asarray(actual).dtype, np.asarray(expected).dtype)
                        np.testing.assert_array_equal(actual, expected)
                        np.testing.assert_array_equal(np.signbit(actual), np.signbit(expected))


if __name__ == "__main__":
    unittest.main()
