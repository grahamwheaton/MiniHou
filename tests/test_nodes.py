import unittest

import numpy as np

from minihou import Evaluator, Geometry, Graph, REGISTRY
from minihou.nodes._util import fan_triangulate, perlin3, rotation_from_z_to
from tests.helpers import graph_from, quad_geo


def run(type_, *inputs, **params):
    """Call a registered node function directly (inputs frozen, like the evaluator would)."""
    nt = REGISTRY.get(type_)
    for g in inputs:
        for x in (g if isinstance(g, list) else [g]):
            if not x.frozen:
                x.freeze()
    out = nt.fn(*inputs, **nt.coerce_params(params))
    out.validate()
    return out


class GridTests(unittest.TestCase):
    def test_counts_and_bounds(self):
        g = run("grid", size_x=4, size_y=2, columns=5, rows=3, center=(1, 2, 3))
        self.assertEqual((g.npoints, g.nprims, g.nvertices), (15, 8, 32))
        np.testing.assert_allclose(g.P.min(0), [-1, 1, 3], atol=1e-6)
        np.testing.assert_allclose(g.P.max(0), [3, 3, 3], atol=1e-6)
        self.assertEqual(g.point["uv"].shape, (15, 2))

    def test_quads_are_counter_clockwise_seen_from_plus_z(self):
        g = run("grid", columns=4, rows=4)
        P = g.P
        for poly in g.topology.polys():
            a, b, c = P[poly[0]], P[poly[1]], P[poly[2]]
            self.assertGreater(np.cross(b - a, c - a)[2], 0)

    def test_every_point_is_used_exactly_by_adjacent_quads(self):
        g = run("grid", columns=3, rows=3)
        counts = np.bincount(g.topology.vertex_point, minlength=g.npoints)
        self.assertEqual(counts.tolist(), [1, 2, 1, 2, 4, 2, 1, 2, 1])


class TransformTests(unittest.TestCase):
    def test_translate_scale_order(self):
        g = run("transform", quad_geo(), translate=(1, 0, 0), scale=(2, 2, 2))
        np.testing.assert_allclose(g.P[2], [3, 2, 0], atol=1e-6)       # (1,1,0)*2 + (1,0,0)

    def test_rotate_z_90_maps_x_to_y(self):
        src = Geometry.points_only(np.array([[1, 0, 0]], np.float32))
        g = run("transform", src, rotate=(0, 0, 90))
        np.testing.assert_allclose(g.P[0], [0, 1, 0], atol=1e-6)

    def test_euler_order_x_then_y_then_z(self):
        src = Geometry.points_only(np.array([[0, 1, 0]], np.float32))
        g = run("transform", src, rotate=(90, 0, 90))        # X: y->z, then Z: z stays
        np.testing.assert_allclose(g.P[0], [0, 0, 1], atol=1e-6)

    def test_pivot(self):
        src = Geometry.points_only(np.array([[2, 0, 0]], np.float32))
        g = run("transform", src, rotate=(0, 0, 180), pivot=(1, 0, 0))
        np.testing.assert_allclose(g.P[0], [0, 0, 0], atol=1e-6)

    def test_normals_rotate_and_stay_unit(self):
        src = Geometry.points_only(np.array([[0, 0, 0]], np.float32))
        src.point["N"] = np.array([[1, 0, 0]], np.float32)
        g = run("transform", src, rotate=(0, 0, 90), scale=(5, 1, 1))
        np.testing.assert_allclose(g.point["N"][0], [0, 1, 0], atol=1e-6)

    def test_untouched_attributes_are_shared_not_copied(self):
        src = run("grid", columns=3, rows=3)
        g = run("transform", src, translate=(1, 1, 1))
        self.assertIs(g.point["uv"], src.point["uv"])
        self.assertIs(g.topology, src.topology)


class NoiseTests(unittest.TestCase):
    def grid(self):
        return run("grid", size_x=10, size_y=10, columns=40, rows=40)

    def test_deterministic_and_seed_sensitive(self):
        a = run("noise", self.grid(), seed=1, frequency=0.7)
        b = run("noise", self.grid(), seed=1, frequency=0.7)
        c = run("noise", self.grid(), seed=2, frequency=0.7)
        np.testing.assert_array_equal(a.P, b.P)
        self.assertFalse(np.allclose(a.P, c.P))

    def test_displaces_only_along_amplitude_in_scalar_mode(self):
        g = self.grid()
        n = run("noise", g, amplitude=(0, 0, 2.0), frequency=0.5)
        d = n.P - g.P
        np.testing.assert_array_equal(d[:, :2], 0)
        self.assertGreater(np.abs(d[:, 2]).max(), 0.1)
        self.assertLess(np.abs(d[:, 2]).max(), 2.0 + 1e-3)           # |noise| <= 1

    def test_zero_amplitude_is_identity(self):
        g = self.grid()
        np.testing.assert_array_equal(run("noise", g, amplitude=(0, 0, 0)).P, g.P)

    def test_perlin_is_zero_at_lattice_points_and_continuous(self):
        lattice = np.array([[0, 0, 0], [1, 2, 3], [-4, 5, -6]], np.float32)
        np.testing.assert_allclose(perlin3(lattice, 3), 0, atol=1e-6)
        p = np.random.default_rng(0).uniform(-5, 5, (2000, 3)).astype(np.float32)
        step = np.array([1e-3, 0, 0], np.float32)
        self.assertLess(np.abs(perlin3(p + step, 0) - perlin3(p, 0)).max(), 0.02)

    def test_perlin_statistics(self):
        p = np.random.default_rng(1).uniform(-50, 50, (200_000, 3)).astype(np.float32)
        v = perlin3(p, 0)
        self.assertLess(abs(float(v.mean())), 0.02)
        self.assertLess(float(np.abs(v).max()), 1.2)
        self.assertGreater(float(v.std()), 0.15)

    def test_vector_mode_channels_are_decorrelated(self):
        # Needs many independent lattice cells, so use a big random point cloud rather than a small grid.
        pts = np.random.default_rng(0).uniform(-50, 50, (100_000, 3)).astype(np.float32)
        g = Geometry.points_only(pts)
        n = run("noise", g, mode="vector", amplitude=(1, 1, 1), frequency=1.0)
        d = n.P - g.P
        c = np.corrcoef(d.T)
        self.assertLess(abs(c[0, 1]), 0.05)
        self.assertLess(abs(c[0, 2]), 0.05)
        self.assertLess(abs(c[1, 2]), 0.05)

    def test_non_P_attribute_is_written(self):
        g = self.grid()
        n = run("noise", g, attrib="mask", amplitude=(2, 0, 0))
        self.assertEqual(n.point["mask"].shape, (g.npoints,))
        np.testing.assert_array_equal(n.P, g.P)
        v = run("noise", g, attrib="Cd", mode="vector", amplitude=(1, 1, 1))
        self.assertEqual(v.point["Cd"].shape, (g.npoints, 3))

    def test_octaves_add_detail_and_stay_normalised(self):
        g = self.grid()
        one = run("noise", g, octaves=1, frequency=0.5)
        many = run("noise", g, octaves=6, frequency=0.5)
        self.assertFalse(np.allclose(one.P, many.P))
        self.assertLess(np.abs(many.P[:, 2]).max(), 1.2)

    def test_topology_and_other_attrs_untouched(self):
        g = self.grid()
        n = run("noise", g)
        self.assertIs(n.topology, g.topology)
        self.assertIs(n.point["uv"], g.point["uv"])


class ScatterTests(unittest.TestCase):
    def test_count_surface_and_attributes(self):
        g = run("grid", size_x=2, size_y=2, columns=3, rows=3)
        s = run("scatter", g, count=500, seed=3)
        self.assertEqual(s.npoints, 500)
        self.assertEqual(s.nprims, 0)
        self.assertLessEqual(np.abs(s.P[:, :2]).max(), 1.0 + 1e-6)
        np.testing.assert_allclose(s.P[:, 2], 0, atol=1e-6)
        np.testing.assert_allclose(s.point["N"], [[0, 0, 1]] * 500, atol=1e-6)
        self.assertTrue(((s.point["primnum"] >= 0) & (s.point["primnum"] < g.nprims)).all())

    def test_points_land_inside_the_prim_they_report(self):
        g = run("grid", size_x=3, size_y=3, columns=4, rows=4)
        s = run("scatter", g, count=2000, seed=0)
        centers = np.array([g.P[p].mean(0) for p in g.topology.polys()])
        far = np.abs(s.P[:, :2] - centers[s.point["primnum"]][:, :2]).max()
        self.assertLessEqual(far, 0.5 + 1e-5)                       # each quad is 1x1

    def test_deterministic_and_seeded(self):
        g = run("grid", columns=5, rows=5)
        np.testing.assert_array_equal(run("scatter", g, count=50, seed=1).P, run("scatter", g, count=50, seed=1).P)
        self.assertFalse(np.array_equal(run("scatter", g, count=50, seed=1).P, run("scatter", g, count=50, seed=2).P))

    def test_density_is_area_weighted(self):
        # two disjoint quads, areas 1 and 9
        P = np.array([[0, 0, 0], [1, 0, 0], [1, 1, 0], [0, 1, 0],
                      [5, 0, 0], [8, 0, 0], [8, 3, 0], [5, 3, 0]], np.float32)
        g = Geometry.from_arrays(P, np.arange(8), [0, 4, 8])
        s = run("scatter", g, count=20000, seed=5)
        frac_big = float((s.point["primnum"] == 1).mean())
        self.assertAlmostEqual(frac_big, 0.9, delta=0.02)

    def test_triangle_fan_and_degenerate_inputs(self):
        pent = Geometry.from_arrays(np.array([[0, 0, 0], [2, 0, 0], [3, 1, 0], [1, 2, 0], [-1, 1, 0]], np.float32),
                                    np.arange(5), [0, 5])
        tris, owner = fan_triangulate(pent.topology)
        self.assertEqual(tris.tolist(), [[0, 1, 2], [0, 2, 3], [0, 3, 4]])
        self.assertEqual(owner.tolist(), [0, 0, 0])
        self.assertEqual(run("scatter", Geometry.points_only(np.zeros((4, 3))), count=10).npoints, 0)
        flat = Geometry.from_arrays(np.zeros((3, 3), np.float32), [0, 1, 2], [0, 3])
        self.assertEqual(run("scatter", flat, count=10).npoints, 0)
        self.assertEqual(run("scatter", run("grid"), count=0).npoints, 0)

    def test_scatter_over_a_displaced_surface_follows_it(self):
        g = run("noise", run("grid", columns=30, rows=30), amplitude=(0, 0, 1), frequency=0.4)
        s = run("scatter", g, count=300, seed=1)
        self.assertGreater(float(np.abs(s.P[:, 2]).max()), 0.05)


class CopyToPointsTests(unittest.TestCase):
    def targets(self, pts, **attrs):
        t = Geometry.points_only(np.asarray(pts, np.float32))
        for k, v in attrs.items():
            t.point[k] = v
        return t

    def test_counts_translation_and_topology(self):
        q = quad_geo()
        t = self.targets([[0, 0, 0], [10, 0, 0], [0, 10, 5]])
        o = run("copy_to_points", q, t)
        self.assertEqual((o.npoints, o.nprims, o.nvertices), (12, 3, 12))
        np.testing.assert_allclose(o.P[4:8], q.P + [10, 0, 0])
        np.testing.assert_allclose(o.P[8:12], q.P + [0, 10, 5])
        self.assertEqual(o.topology.polys()[2], [8, 9, 10, 11])

    def test_orientation_follows_N(self):
        src = Geometry.points_only(np.array([[0, 0, 1]], np.float32))      # a point on +Z
        t = self.targets([[0, 0, 0]], N=np.array([[1, 0, 0]], np.float32))
        o = run("copy_to_points", src, t)
        np.testing.assert_allclose(o.P[0], [1, 0, 0], atol=1e-6)
        t2 = self.targets([[0, 0, 0]], N=np.array([[0, 0, -1]], np.float32))
        np.testing.assert_allclose(run("copy_to_points", src, t2).P[0], [0, 0, -1], atol=1e-6)
        t3 = self.targets([[0, 0, 0]], N=np.array([[0, 0, 1]], np.float32))
        np.testing.assert_allclose(run("copy_to_points", src, t3).P[0], [0, 0, 1], atol=1e-6)

    def test_orientation_off_ignores_N(self):
        src = Geometry.points_only(np.array([[0, 0, 1]], np.float32))
        t = self.targets([[5, 0, 0]], N=np.array([[1, 0, 0]], np.float32))
        o = run("copy_to_points", src, t, orient=False)
        np.testing.assert_allclose(o.P[0], [5, 0, 1], atol=1e-6)

    def test_rotation_matrices_map_z_to_n_for_random_normals(self):
        n = np.random.default_rng(2).normal(size=(500, 3))
        n /= np.linalg.norm(n, axis=1, keepdims=True)
        R = rotation_from_z_to(n)
        np.testing.assert_allclose(R @ np.array([0, 0, 1.0]), n, atol=1e-9)
        np.testing.assert_allclose(np.einsum("kij,kjl->kil", R, np.transpose(R, (0, 2, 1))), np.tile(np.eye(3), (500, 1, 1)), atol=1e-9)

    def test_pscale(self):
        src = Geometry.points_only(np.array([[1, 0, 0]], np.float32))
        t = self.targets([[0, 0, 0], [0, 0, 0]], pscale=np.array([2.0, 0.5], np.float32))
        np.testing.assert_allclose(run("copy_to_points", src, t).P[:, 0], [2.0, 0.5])
        np.testing.assert_allclose(run("copy_to_points", src, t, use_pscale=False).P[:, 0], [1.0, 1.0])

    def test_attribute_inheritance(self):
        q = quad_geo()
        q.point["w"] = np.array([1, 2, 3, 4], np.float32)
        q.prim["tag"] = np.array([9], np.int32)
        t = self.targets([[0, 0, 0], [1, 1, 1]], Cd=np.array([[1, 0, 0], [0, 1, 0]], np.float32))
        o = run("copy_to_points", q, t)
        np.testing.assert_array_equal(o.point["w"], [1, 2, 3, 4, 1, 2, 3, 4])
        np.testing.assert_array_equal(o.point["Cd"][:4], [[1, 0, 0]] * 4)
        np.testing.assert_array_equal(o.point["Cd"][4:], [[0, 1, 0]] * 4)
        np.testing.assert_array_equal(o.prim["tag"], [9, 9])

    def test_empty_inputs(self):
        q = quad_geo()
        self.assertEqual(run("copy_to_points", q, self.targets(np.zeros((0, 3)))).npoints, 0)
        self.assertEqual(run("copy_to_points", Geometry.points_only(np.zeros((0, 3))), self.targets([[0, 0, 0]])).npoints, 0)

    def test_scatter_then_copy_integration(self):
        g = run("grid", columns=6, rows=6)
        pts = run("scatter", g, count=100, seed=0)
        o = run("copy_to_points", quad_geo(size=0.1), pts)
        self.assertEqual((o.npoints, o.nprims), (400, 100))
        # N is +Z on a flat grid, so copies stay flat on the surface plane
        np.testing.assert_allclose(o.P[:, 2], 0, atol=1e-6)


class MergeTests(unittest.TestCase):
    def test_topology_is_offset(self):
        a, b = quad_geo(), quad_geo(5)
        m = run("merge", [a, b])
        self.assertEqual((m.npoints, m.nvertices, m.nprims), (8, 8, 2))
        self.assertEqual(m.topology.polys(), [[0, 1, 2, 3], [4, 5, 6, 7]])
        np.testing.assert_array_equal(m.P[4:], b.P)

    def test_missing_attributes_zero_filled(self):
        a, b = quad_geo(), quad_geo(5)
        a.point["Cd"] = np.ones((4, 3), np.float32)
        b.point["mask"] = np.ones(4, np.float32)
        a.prim["id"] = np.array([5], np.int32)
        m = run("merge", [a, b])
        np.testing.assert_array_equal(m.point["Cd"][4:], 0)
        np.testing.assert_array_equal(m.point["mask"][:4], 0)
        np.testing.assert_array_equal(m.prim["id"], [5, 0])

    def test_dtype_promotion_and_shape_conflict(self):
        a, b = quad_geo(), quad_geo(5)
        a.point["x"] = np.ones(4, np.int32)
        b.point["x"] = np.full(4, 0.5, np.float32)
        self.assertEqual(run("merge", [a, b]).point["x"].dtype, np.float32)
        c = quad_geo(9)
        c.point["x"] = np.ones((4, 3), np.float32)
        with self.assertRaises(ValueError):
            run("merge", [a, c])

    def test_single_input_passthrough_and_points_only_mix(self):
        a = quad_geo()
        self.assertIs(run("merge", [a]), a)
        m = run("merge", [a, Geometry.points_only(np.ones((3, 3)))])
        self.assertEqual((m.npoints, m.nprims), (7, 1))

    def test_detail_first_wins(self):
        a, b = quad_geo(), quad_geo(2)
        a.detail["name"] = "a"
        b.detail.update(name="b", extra=1)
        m = run("merge", [a, b])
        self.assertEqual(m.detail, {"name": "a", "extra": 1})

    def test_many_inputs(self):
        m = run("merge", [quad_geo(i) for i in range(50)])
        self.assertEqual((m.npoints, m.nprims), (200, 50))
        m.validate()


class OutputTests(unittest.TestCase):
    def test_output_passes_through(self):
        g = run("grid")
        self.assertIs(run("output", g), g)

    def test_evaluator_rejects_invalid_output(self):
        from minihou import NodeError
        from minihou.sdk import Registry, node
        reg = Registry()
        node("broken", registry=reg)(lambda: Geometry.from_arrays(np.zeros((2, 3)), [0, 7], [0, 2]))
        with self.assertRaises(NodeError):
            Evaluator(reg).cook(graph_from({"b": {"type": "broken"}}, registry=reg), "b")


class WrangleReadinessTests(unittest.TestCase):
    """The node API is enough to host a future wrangle node (snippet string -> numpy)."""

    def test_toy_wrangle_node(self):
        from minihou.sdk import Registry, Str, node
        reg = Registry()
        compiled = {}

        @node("toy_wrangle", inputs=["geo"], params=[Str("snippet", "", kind="code")], registry=reg)
        def toy_wrangle(geo, snippet):
            code = compiled.get(snippet) or compiled.setdefault(snippet, compile(snippet, "<wrangle>", "exec"))
            P = geo.P.copy()
            env = {"P": P, "np": np, "x": P[:, 0], "y": P[:, 1], "z": P[:, 2]}
            exec(code, {"__builtins__": {}}, env)               # test-only stand-in for a real compiler
            out = geo.copy()
            out.point["P"] = P
            return out

        src = graph_from({"g": {"type": "grid", "params": {"columns": 5, "rows": 5}},
                          "w": {"type": "toy_wrangle", "params": {"snippet": "z += np.sin(x)"}, "inputs": {"geo": "g"}}},
                         display="w", registry=REGISTRY_WITH(reg))
        ev = Evaluator(REGISTRY_WITH(reg))
        out = ev.cook(src)
        np.testing.assert_allclose(out.P[:, 2], np.sin(out.P[:, 0]), atol=1e-6)
        # the snippet is part of the cache key: edit it and only the wrangle recooks
        src.set_param("w", "snippet", "z += 2")
        ev.cook(src)
        self.assertEqual(ev.last_stats.cooked, ["w"])


def REGISTRY_WITH(extra):
    """Registry containing the built-ins plus the nodes in ``extra``."""
    from minihou.sdk import Registry
    r = Registry()
    for nt in REGISTRY:
        r.register(nt)
    for nt in extra:
        r.register(nt)
    return r


if __name__ == "__main__":
    unittest.main()
