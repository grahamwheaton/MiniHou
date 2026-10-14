"""The Points family nodes: point_generate, point_replicate, point_velocity, cluster_points, point_cloud_*, points_from_volume,
scatter_in_texture_mask, particle_trail."""
import os
import unittest

import numpy as np

import minihou  # noqa: F401
from minihou import Geometry, spatial
from minihou.sdk import REGISTRY


def run(name, *inputs, **params):
    nt = REGISTRY.get(name)
    return nt.fn(*inputs, **nt.coerce_params(params))


def cloud(n=200, seed=0, flat=False):
    P = np.random.default_rng(seed).random((n, 3)).astype(np.float32)
    if flat:
        P[:, 2] = 0
    return Geometry.from_arrays(P)


def cube(size=1.0):
    h = size / 2
    P = np.array([[x, y, z] for z in (-h, h) for y in (-h, h) for x in (-h, h)], np.float32)
    quads = [[0, 1, 3, 2], [4, 6, 7, 5], [0, 4, 5, 1], [2, 3, 7, 6], [0, 2, 6, 4], [1, 5, 7, 3]]
    return Geometry.from_arrays(P, np.array(quads, np.int32).ravel(), np.arange(7, dtype=np.int32) * 4)


class GenerateTests(unittest.TestCase):
    def test_shapes(self):
        for shape in ("box", "sphere", "ball", "line", "circle", "origin"):
            g = run("point_generate", count=50, shape=shape, seed=1)
            self.assertEqual(g.npoints, 50)
            self.assertEqual(list(g.point["id"]), list(range(50)))
        s = run("point_generate", count=100, shape="sphere", size=(4, 4, 4))
        np.testing.assert_allclose(np.linalg.norm(s.P, axis=1), 2.0, atol=1e-4)
        self.assertTrue((run("point_generate", count=100, shape="box", size=(2, 2, 2)).P.__abs__() <= 1.0001).all())
        a, b = run("point_generate", count=30, seed=2), run("point_generate", count=30, seed=2)
        np.testing.assert_array_equal(a.P, b.P)


class ReplicateTests(unittest.TestCase):
    def test_copies_and_attributes(self):
        g = cloud(10)
        g.point["w"] = np.arange(10, dtype=np.float32)
        out = run("point_replicate", g, copies=3, offset=(0.1, 0.1, 0.1))
        self.assertEqual(out.npoints, 30)
        np.testing.assert_array_equal(out.point["source"], np.repeat(np.arange(10), 3))
        np.testing.assert_array_equal(out.point["w"], np.repeat(np.arange(10), 3))
        orig = out.point["copy"] == 0
        np.testing.assert_allclose(out.P[orig], g.P)
        self.assertGreater(np.abs(out.P[~orig] - g.P[out.point["source"][~orig]]).max(), 0)

    def test_per_point_counts(self):
        g = cloud(4)
        g.point["n"] = np.array([0, 1, 2, 3], np.int32)
        out = run("point_replicate", g, copies_attrib="n")
        self.assertEqual(list(out.point["source"]), [1, 2, 2, 3, 3, 3])


class VelocityTests(unittest.TestCase):
    def test_difference_by_number_and_by_id(self):
        now = cloud(5)
        prev = Geometry.from_arrays(now.P - np.array([1.0, 0, 0], np.float32))
        out = run("point_velocity", now, prev, method="difference", time_step=0.5)
        np.testing.assert_allclose(out.point["v"], np.tile([2.0, 0, 0], (5, 1)), atol=1e-5)
        now.point["id"] = np.arange(5, dtype=np.int32)[::-1].copy()
        prev.point["id"] = np.arange(5, dtype=np.int32)
        out = run("point_velocity", now, prev, method="difference", time_step=1.0)
        self.assertEqual(out.point["v"].shape, (5, 3))
        with self.assertRaises(ValueError):
            run("point_velocity", now, None, method="difference")

    def test_noise_is_bounded_and_direction_is_constant(self):
        g = cloud(300)
        v = run("point_velocity", g, method="noise", speed=2.0).point["v"]
        self.assertTrue(np.isfinite(v).all())
        self.assertGreater(np.linalg.norm(v, axis=1).max(), 0.5)
        d = run("point_velocity", g, method="direction", direction=(0, 0, 3), speed=2.0).point["v"]
        np.testing.assert_allclose(d, np.tile([0, 0, 2.0], (300, 1)), atol=1e-6)
        added = run("point_velocity", run("point_velocity", g, method="direction", direction=(1, 0, 0)), method="direction",
                    direction=(0, 1, 0), add=True).point["v"]
        np.testing.assert_allclose(added, np.tile([1.0, 1.0, 0], (300, 1)), atol=1e-6)


class ClusterTests(unittest.TestCase):
    def blobs(self):
        rng = np.random.default_rng(3)
        P = np.concatenate([rng.normal(c, 0.05, (60, 3)) for c in ((0, 0, 0), (5, 0, 0), (0, 5, 0))]).astype(np.float32)
        return Geometry.from_arrays(P)

    def test_kmeans_finds_the_blobs(self):
        g = self.blobs()
        lab = run("cluster_points", g, method="kmeans", count=3, seed=1).point["cluster"]
        for i in range(3):
            self.assertEqual(len(set(lab[i * 60:(i + 1) * 60])), 1)
        self.assertEqual(len(set(lab)), 3)

    def test_distance_and_grid_and_centroids(self):
        g = self.blobs()
        self.assertEqual(len(set(run("cluster_points", g, method="distance", size=1.0).point["cluster"])), 3)
        self.assertEqual(len(set(run("cluster_points", g, method="grid", size=2.0).point["cluster"])), 3)
        c = run("cluster_points", g, method="kmeans", count=3, output="centroids")
        self.assertEqual(c.npoints, 3)
        self.assertEqual(sorted(c.point["count"]), [60, 60, 60])


class CloudTests(unittest.TestCase):
    def test_normals_of_a_flat_cloud_are_z(self):
        g = cloud(300, flat=True)
        n = run("point_cloud_normal", g, orient="up").point["N"]
        np.testing.assert_allclose(np.abs(n[:, 2]), 1.0, atol=1e-4)
        self.assertTrue((n[:, 2] > 0).all())
        n2 = run("point_cloud_normal", g, orient="viewpoint", viewpoint=(0, 0, -5)).point["N"]
        self.assertTrue((n2[:, 2] < 0).all())

    def test_normals_of_a_sphere_point_outward(self):
        v = np.random.default_rng(4).normal(size=(600, 3))
        g = Geometry.from_arrays((v / np.linalg.norm(v, axis=1, keepdims=True)).astype(np.float32))
        n = run("point_cloud_normal", g, orient="outward", neighbors=10).point["N"]
        self.assertGreater((np.einsum("ij,ij->i", n, g.P) > 0.9).mean(), 0.95)

    def test_measure(self):
        g = cloud(300)
        out = run("point_cloud_measure", g, radius=0.2)
        for k in ("neighbors", "density", "nn_dist", "mean_dist"):
            self.assertEqual(len(out.point[k]), 300)
        self.assertTrue((out.point["mean_dist"] >= out.point["nn_dist"] - 1e-6).all())
        P = g.P.astype(np.float64)
        d = np.linalg.norm(P[:, None] - P[None], axis=2)
        np.testing.assert_array_equal(out.point["neighbors"], (d < 0.2).sum(1) - 1)

    def test_reduce_modes(self):
        g = cloud(800)
        out = run("point_cloud_reduce", g, mode="distance", distance=0.1)
        self.assertLess(out.npoints, 800)
        _, d = spatial.knn(out.P, out.P, 2)
        self.assertGreaterEqual(d[:, 1].min(), 0.1 - 1e-6)
        out = run("point_cloud_reduce", g, mode="count", count=100)
        self.assertLessEqual(out.npoints, 100)
        self.assertGreater(out.npoints, 40)
        self.assertEqual(run("point_cloud_reduce", g, mode="percent", percent=25).npoints, 200)

    def test_surface_flattens_noise(self):
        rng = np.random.default_rng(5)
        P = rng.random((600, 3)).astype(np.float32)
        P[:, 2] = (rng.random(600) - 0.5) * 0.02
        ref = Geometry.from_arrays(P)
        pts = Geometry.from_arrays(np.stack([rng.random(50), rng.random(50), rng.random(50) * 0.3], 1).astype(np.float32))
        out = run("point_cloud_surface", pts, ref, neighbors=20, iterations=2)
        self.assertLess(np.abs(out.P[:, 2]).max(), 0.05)
        self.assertEqual(out.point["N"].shape, (50, 3))

    def test_knn_matches_brute_force_with_and_without_scipy(self):
        P = np.random.default_rng(6).random((120, 3))
        d = np.linalg.norm(P[:, None] - P[None], axis=2)
        want = np.sort(d, axis=1)[:, :5]
        for use in (True, False):
            _, dist = spatial.knn(P, P, 5, use_scipy=use)
            np.testing.assert_allclose(dist, want, atol=1e-9)


class VolumeTests(unittest.TestCase):
    def test_grid_fills_the_cube(self):
        out = run("points_from_volume", cube(1.0), mode="grid", spacing=0.25)
        self.assertGreater(out.npoints, 10)
        self.assertTrue((np.abs(out.P) < 0.5).all())
        np.testing.assert_allclose(out.point["pscale"], 0.125)

    def test_random_poisson_and_shell(self):
        g = run("points_from_volume", cube(1.0), mode="random", count=500, seed=1)
        self.assertEqual(g.npoints, 500)
        p = run("points_from_volume", cube(1.0), mode="poisson", count=1000, spacing=0.2)
        _, d = spatial.knn(p.P, p.P, 2)
        self.assertGreaterEqual(d[:, 1].min(), 0.2 - 1e-6)
        shell = run("points_from_volume", cube(1.0), mode="random", count=1000, spacing=0.1, shell_only=True)
        self.assertLess(shell.npoints, 1000)
        self.assertGreaterEqual(np.abs(shell.P).max(axis=1).min(), 0.4 - 1e-4)


class MaskTests(unittest.TestCase):
    def mask(self):
        from minihou import images
        pix = np.zeros((16, 16, 4), np.float32)
        pix[:, 8:, :3] = 1.0                                  # right half white
        pix[..., 3] = 1.0
        return images.grid_from_pixels(pix, 1.0)

    def surface(self):
        from minihou.nodes.grid import grid as grid_node  # noqa: F401
        g = run("grid", columns=2, rows=2, size_x=1.0, size_y=1.0)
        P = g.P
        uv = np.stack([P[:, 0] + 0.5, P[:, 1] + 0.5], 1).astype(np.float32)
        out = g.copy()
        out.point["uv"] = uv
        return out

    def test_points_land_only_where_the_mask_is_white(self):
        out = run("scatter_in_texture_mask", self.surface(), self.mask(), count=400, seed=2)
        self.assertEqual(out.npoints, 400)
        self.assertGreater(out.P[:, 0].min(), -0.1)             # left half (x < 0) is black
        out = run("scatter_in_texture_mask", self.surface(), self.mask(), count=300, invert=True, seed=2)
        self.assertLess(out.P[:, 0].max(), 0.1)

    def test_errors_are_clear(self):
        bare = run("grid", columns=2, rows=2)
        with self.assertRaises(ValueError) as cm:
            run("scatter_in_texture_mask", bare, self.mask(), count=10, uv_attrib="nope")
        self.assertIn("UV", str(cm.exception))


class TrailTests(unittest.TestCase):
    def test_trails_go_back_along_v(self):
        g = Geometry.from_arrays(np.zeros((3, 3), np.float32))
        g.point["v"] = np.tile([2.0, 0, 0], (3, 1)).astype(np.float32)
        out = run("particle_trail", g, length=1.0, samples=5)
        self.assertEqual((out.npoints, out.nprims), (15, 3))
        np.testing.assert_allclose(out.P[:5, 0], np.linspace(0, -2, 5), atol=1e-6)
        self.assertEqual(out.point["trail"][0], 0.0)
        self.assertEqual(out.point["trail"][4], 1.0)
        self.assertFalse(out.topology.closed_mask().any())
        with self.assertRaises(ValueError):
            run("particle_trail", Geometry.from_arrays(np.zeros((2, 3), np.float32)))


if __name__ == "__main__":
    unittest.main()


class ExtraOptionTests(unittest.TestCase):
    def test_jitter_by_id_is_stable_when_points_are_removed(self):
        g = cloud(20)
        g.point["id"] = np.arange(20, dtype=np.int32)
        a = run("point_jitter", g, scale=(1, 1, 1), id_attrib="id", seed=3)
        keep = g.copy()
        sub = Geometry.from_arrays(g.P[5:15])
        sub.point["id"] = g.point["id"][5:15]
        b = run("point_jitter", sub, scale=(1, 1, 1), id_attrib="id", seed=3)
        np.testing.assert_allclose(b.P - sub.P, a.P[5:15] - g.P[5:15], atol=1e-6)
        m = g.copy()
        m.point["m"] = np.zeros(20, np.float32)
        np.testing.assert_allclose(run("point_jitter", m, scale=(1, 1, 1), mask="m").P, g.P)

    def test_scatter_density_relax_and_id(self):
        grid = run("grid", columns=3, rows=2, size_x=2.0, size_y=1.0)         # 2 quads side by side
        grid = grid.copy()
        grid.prim["d"] = np.array([0.0, 1.0], np.float32)
        out = run("scatter", grid, count=300, seed=1, density_attrib="d", output_id=True)
        self.assertGreater(out.P[:, 0].min(), -0.001)                           # only the right quad gets points
        self.assertEqual(list(out.point["id"][:3]), [0, 1, 2])
        plain = run("scatter", grid, count=200, seed=1)
        relaxed = run("scatter", grid, count=200, seed=1, relax_iterations=5)
        _, d0 = spatial.knn(plain.P, plain.P, 2)
        _, d1 = spatial.knn(relaxed.P, relaxed.P, 2)
        self.assertGreater(d1[:, 1].min(), d0[:, 1].min())

    def test_reduce_voxel(self):
        g = cloud(1000)
        g.point["w"] = g.P[:, 0].copy()
        for how in ("average", "nearest_center", "first"):
            out = run("point_cloud_reduce", g, mode="voxel", voxel_size=0.25, voxel_point=how)
            self.assertLessEqual(out.npoints, 64)
            self.assertGreater(out.npoints, 30)
        avg = run("point_cloud_reduce", g, mode="voxel", voxel_size=0.5, voxel_point="average")
        np.testing.assert_allclose(avg.point["w"], avg.P[:, 0], atol=1e-5)

    def test_measure_second_input_and_shape(self):
        line = Geometry.from_arrays(np.stack([np.linspace(0, 1, 200), np.zeros(200), np.zeros(200)], 1).astype(np.float32))
        out = run("point_cloud_measure", line, shape=True, neighbors_k=8)
        self.assertGreater(out.point["linear"][100], 0.9)
        self.assertLess(out.point["curvature"][100], 0.01)
        q = Geometry.from_arrays(np.array([[0.5, 0.0, 0.0], [5.0, 0.0, 0.0]], np.float32))
        out = run("point_cloud_measure", q, line, radius=0.1)
        self.assertGreater(out.point["neighbors"][0], 10)
        self.assertEqual(out.point["neighbors"][1], 0)

    def test_cluster_by_rest_and_normal_radius_reverse(self):
        g = cloud(60)
        rest = Geometry.from_arrays(np.concatenate([np.zeros((30, 3)), np.ones((30, 3)) * 9]).astype(np.float32))
        lab = run("cluster_points", g, rest, method="grid", size=3.0).point["cluster"]
        self.assertEqual(len(set(lab[:30])), 1)
        self.assertNotEqual(lab[0], lab[59])
        n1 = run("point_cloud_normal", cloud(200, flat=True), orient="up").point["N"]
        n2 = run("point_cloud_normal", cloud(200, flat=True), orient="up", reverse=True).point["N"]
        np.testing.assert_allclose(n1, -n2, atol=1e-5)
        far = Geometry.from_arrays(np.array([[0, 0, 0], [50, 0, 0], [0, 50, 0], [50, 50, 0]], np.float32))
        n3 = run("point_cloud_normal", far, radius=1.0, neighbors=3).point["N"]
        np.testing.assert_allclose(n3, 0.0)
