"""Surface nets: exact vertex order/topology and strict float64 arithmetic parity."""
import unittest

import numpy as np

from minihou import rust, sdf
from minihou.nodes.primitives import box, sphere, torus
from minihou.nodes.volume import _tris, shape_distance


@unittest.skipUnless(rust.available(), "the Rust core is not built")
class RustSdfNetsTests(unittest.TestCase):
    def same(self, values, h=0.173, origin=(0.31, -1.7, 2.9)):
        vol = sdf.Volume(np.asarray(origin, np.float64), h, np.asarray(values))
        before = vol.values.copy()
        with rust.disabled():
            expected_p, expected_q = sdf.surface_nets(vol)
        actual_p, actual_q = rust.sdf_surface_nets(vol)
        self.assertEqual(actual_p.dtype, expected_p.dtype)
        self.assertEqual(actual_q.dtype, expected_q.dtype)
        # The implementation preserves each add.at contribution, so even float64 is exact.
        np.testing.assert_array_equal(actual_p, expected_p)
        np.testing.assert_array_equal(actual_q, expected_q)
        np.testing.assert_array_equal(actual_q.reshape(-1), expected_q.reshape(-1))
        np.testing.assert_array_equal(np.arange(0, actual_q.size + 1, 4),
                                      np.arange(0, expected_q.size + 1, 4))
        np.testing.assert_array_equal(vol.values, before)

    def test_analytic_node_fields(self):
        p = np.stack(np.meshgrid(*[np.linspace(-1.4, 1.4, 19)] * 3, indexing="ij"), -1)
        for shape in ("box", "sphere", "torus"):
            with self.subTest(shape=shape):
                values = shape_distance(shape, p, .8, (1.1, 1.3, 1.7), 1., .3, .2, 6, .05)
                self.same(values.astype(np.float32))

    def test_mesh_node_fields(self):
        with rust.disabled():
            geos = [box(), sphere(.8, (0, 0, 0), 6, 10), torus(.7, .2, (0, 0, 0), 8, 12)]
            fields = [sdf.from_mesh(_tris(g, "nets parity"), .24) for g in geos]
        for i, vol in enumerate(fields):
            with self.subTest(shape=i):
                self.same(vol.values, vol.h, vol.origin)

    def test_random_point_cloud(self):
        rng = np.random.default_rng(42)
        with rust.disabled():
            vol = sdf.from_points(rng.normal(0, .3, (11, 3)), rng.uniform(.15, .3, 11), .17)
        self.same(vol.values, vol.h, vol.origin)

    def test_degenerate(self):
        fields = [np.empty((0, 2, 2)), np.ones((1, 1, 1)), -np.ones((1, 4, 3)),
                  np.zeros((3, 4, 5)), np.ones((3, 4, 5)), -np.ones((3, 4, 5)),
                  np.array([-1, 1, 1, 1, 1, 1, 1, 1]).reshape(2, 2, 2)]
        for i, field in enumerate(fields):
            with self.subTest(case=i):
                self.same(field.astype(np.float32))
        with rust.disabled():
            vol = sdf.from_mesh(np.array([[[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]]]), .4)
        self.same(vol.values, vol.h, vol.origin)
        self.same(np.array([-1, 1, 1, 1, 1, 1, 1, 1]).reshape(2, 2, 2), 1e12)

    def test_random_grids(self):
        rng = np.random.default_rng(843)
        for i in range(35):
            dims = tuple(rng.integers(2, 10, 3))
            for dtype in (np.float32, np.float64):
                with self.subTest(seed=i, dtype=dtype):
                    self.same(rng.normal(size=dims).astype(dtype))

    def test_saddle_batches_dtype_ties_rounds_and_input_unchanged(self):
        rng = np.random.default_rng(122)
        checkerboard = np.where(np.indices((5, 6, 7)).sum(0) % 2, -1., 1.)
        fields = [checkerboard, checkerboard * 1e12,
                  np.array([[[-1., 1.], [1., -1.]]]),
                  rng.normal(size=(6, 8, 5)), rng.choice([-1., 0., 1.], size=(5, 7, 3))]
        for dtype in (np.float32, np.float64):
            for i, values in enumerate(fields):
                for rounds in (0, 1, 2, 16):
                    with self.subTest(dtype=dtype, case=i, rounds=rounds):
                        values = values.astype(dtype)
                        before = values.copy()
                        with rust.disabled():
                            expected = sdf._resolve_face_saddles(values, rounds)
                        actual = rust.sdf_resolve_face_saddles(values, rounds)
                        self.assertEqual(actual.dtype, expected.dtype)
                        np.testing.assert_array_equal(actual, expected)
                        np.testing.assert_array_equal(values, before)


if __name__ == "__main__":
    unittest.main()
