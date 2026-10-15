"""Mesh-to-SDF parity, including surface signs and six-neighbour flood topology."""
import unittest
import numpy as np
from minihou import Evaluator, Graph, accel, rust, sdf
from minihou.nodes.volume import _tris


@unittest.skipUnless(rust.available(), "Rust extension is not installed")
class RustSdfMeshTests(unittest.TestCase):
    def check_mesh(self, tris, h=0.19, **kwargs):
        with rust.disabled(), accel.disabled():
            expected = sdf.from_mesh(tris, h, **kwargs)
        actual = rust.sdf_from_mesh(tris, h, **kwargs)
        np.testing.assert_array_equal(actual.origin, expected.origin)
        self.assertEqual(actual.h, expected.h)
        self.assertEqual(actual.values.dtype, np.dtype('float32'))
        np.testing.assert_array_equal(actual.values, expected.values)
        np.testing.assert_array_equal(actual.values.view(np.uint32), expected.values.view(np.uint32))
        np.testing.assert_array_equal(actual.values < 0, expected.values < 0)
        if accel.enabled():
            with rust.disabled():
                compiled = sdf.from_mesh(tris, h, **kwargs)
            np.testing.assert_allclose(actual.values, compiled.values, atol=1e-6, rtol=0)
        return actual, expected

    def primitive(self, kind, params):
        g = Graph.from_dict({'version': 1, 'nodes': {'p': {'type': kind, 'params': params}}, 'display': 'p'})
        with rust.disabled(), accel.disabled():
            return _tris(Evaluator().cook(g, 'p'), 'test')

    def test_box(self):
        self.check_mesh(self.primitive('box', {}))
        self.check_mesh(self.primitive('box', {}), h=0.17, pad_voxels=4)

    def test_sphere(self):
        self.check_mesh(self.primitive('sphere', {'rows': 9, 'columns': 12}))

    def test_torus(self):
        self.check_mesh(self.primitive('torus', {'rows': 8, 'columns': 12}))

    def test_transformed_box(self):
        self.check_mesh(self.primitive('box', {'size': [1.1, 0.7, 1.5], 'rotate': [17, 31, -9], 'center': [0.23, -0.41, 0.7]}))

    def test_empty(self):
        self.check_mesh(np.zeros((0, 3, 3)))
        self.check_mesh(np.zeros((0, 3, 3)), grid=([1., 2., 3.], [1, 1, 1]))

    def test_one_triangle(self):
        self.check_mesh(np.array([[[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]]]))

    def test_degenerate_triangles(self):
        for t in (np.zeros((1, 3, 3)), np.array([[[0., 0., 0.], [1., 0., 0.], [2., 0., 0.]]])):
            with self.subTest(tris=t.tolist()):
                self.check_mesh(t, grid=([0., 0., 0.], [1, 1, 1]))

    def test_single_voxel_and_huge_voxel(self):
        tris = self.primitive('box', {})
        self.check_mesh(tris, grid=([0., 0., 0.], [1, 1, 1]))
        self.check_mesh(tris, h=1e5)

    def test_random_cloud_triangles(self):
        for seed in range(4):
            tris = np.random.default_rng(seed).normal(size=(7, 3, 3))
            with self.subTest(seed=seed):
                self.check_mesh(tris, h=0.37)

    def test_opposed_coincident_triangles(self):
        t = np.array([[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]])
        self.check_mesh(np.array([t, t[::-1]]), h=0.25)

    def test_equal_distance_faces(self):
        tris = self.primitive('box', {})
        self.check_mesh(tris, h=0.25, grid=([-0.75, -0.75, -0.75], [7, 7, 7]))

    def test_flood_random(self):
        for shape in ((1, 1, 1), (1, 3, 7), (7, 9, 5), (11, 11, 11)):
            for seed in range(3):
                free = np.random.default_rng(seed).random(shape) > 0.4
                with rust.disabled():
                    expected = sdf._flood(free)
                np.testing.assert_array_equal(rust.sdf_flood(free), expected)

    def test_flood_enclosed_and_tunnel(self):
        free = np.ones((7, 7, 7), bool)
        free[1:6, 1:6, 1:6] = False
        free[2:5, 2:5, 2:5] = True
        for tunnel in (False, True):
            if tunnel:
                free[:4, 3, 3] = True
            with rust.disabled():
                expected = sdf._flood(free)
            np.testing.assert_array_equal(rust.sdf_flood(free), expected)


if __name__ == '__main__':
    unittest.main()
