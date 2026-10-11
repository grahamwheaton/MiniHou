"""Attribute toolkit nodes and the spatial helpers under them."""
import unittest

import numpy as np

from minihou import Evaluator, Graph, NodeError, spatial


def cook(nodes):
    nodes = dict(nodes)
    last = list(nodes)[-1]
    nodes["out"] = {"type": "output", "inputs": {"geo": last}}
    g = Evaluator().cook(Graph.from_dict({"nodes": nodes, "display": "out"}))
    g.validate()
    return g


def chain(*steps, src=None):
    nodes = dict(src or {"g": {"type": "grid", "params": {"columns": 5, "rows": 5, "size_x": 4, "size_y": 4}}})
    prev = list(nodes)[-1]
    for i, (typ, params) in enumerate(steps):
        nodes[f"n{i}"] = {"type": typ, "inputs": {"geo": prev}, "params": params}
        prev = f"n{i}"
    return cook(nodes)


def brute(Q, S):
    return np.concatenate([np.sqrt(((Q[k:k + 64][:, None] - S[None]) ** 2).sum(-1)).min(1) for k in range(0, len(Q), 64)])


class SpatialTests(unittest.TestCase):
    def test_grid_nearest_is_exact_for_many_layouts(self):
        rng = np.random.default_rng(5)
        layouts = {
            "volume": (rng.random((800, 3)) * 4, rng.random((900, 3)) * 6 - 1),
            "surface": (np.c_[rng.random((800, 2)) * 5, np.zeros(800)], rng.normal(2, 2, (900, 3))),
            "line": (np.c_[np.linspace(0, 9, 300), np.zeros((300, 2))], rng.normal(0, 5, (400, 3))),
            "far queries": (rng.random((300, 3)), rng.normal(0, 500, (300, 3))),
            "duplicates": (np.repeat(rng.random((50, 3)), 4, axis=0), rng.random((200, 3))),
            "one point": (np.array([[1.0, 2, 3]]), rng.random((20, 3))),
        }
        for name, (S, Q) in layouts.items():
            i, d = spatial._grid_nearest(S, Q)
            np.testing.assert_allclose(d, brute(Q, S), err_msg=name)
            np.testing.assert_allclose(np.linalg.norm(Q - S[i], axis=1), d, err_msg=name)
        i, d = spatial.nearest(np.zeros((0, 3)), np.ones((3, 3)))
        self.assertTrue(np.all(i == -1) and np.all(np.isinf(d)))

    @unittest.skipIf(spatial._KDTree is None, "scipy not installed")
    def test_scipy_and_grid_agree(self):
        rng = np.random.default_rng(6)
        S, Q = rng.random((2000, 3)), rng.random((2000, 3)) * 1.2
        np.testing.assert_allclose(spatial.nearest(S, Q)[1], spatial.nearest(S, Q, use_scipy=False)[1])

    def test_raycast_grid_is_exact(self):
        from minihou.raycast import TriGrid, _closest_on_tri, _moller_trumbore
        rng = np.random.default_rng(3)
        th = rng.random((300, 3)) * np.pi * 2
        soup = np.stack([np.c_[np.cos(th[:, i]) * 2, np.sin(th[:, i]) * 2, rng.normal(0, 1, 300)] for i in range(3)], 1)
        plane = np.array([[[-9, -9, -3], [9, -9, -3], [9, 9, -3]], [[-9, -9, -3], [9, 9, -3], [-9, 9, -3]]], float)
        tiny = rng.random((150, 3, 3)) * 0.1 + rng.random((150, 1, 3)) * 6 - 3
        for name, tris in (("soup", soup), ("plane", plane), ("mixed", np.concatenate([soup, plane, tiny]))):
            g = TriGrid(tris)
            o, d = rng.normal(0, 4, (200, 3)), rng.normal(0, 1, (200, 3))
            t = g.intersect(o, d)[0]
            dn = d / np.linalg.norm(d, axis=1, keepdims=True)
            T = len(tris)
            bt, ok = _moller_trumbore(np.repeat(o, T, 0), np.repeat(dn, T, 0), np.tile(tris, (200, 1, 1)))[0::3]
            bt = np.where(ok & (bt >= 0), bt, np.inf).reshape(200, T).min(1)
            np.testing.assert_allclose(np.where(np.isinf(t), -1, t), np.where(np.isinf(bt), -1, bt), atol=1e-7, err_msg=name)
            p = rng.normal(0, 5, (100, 3))
            pp = np.repeat(p, T, 0)
            bd = np.linalg.norm(_closest_on_tri(pp, np.tile(tris, (100, 1, 1))) - pp, axis=1).reshape(100, T).min(1)
            np.testing.assert_allclose(g.closest(p)[1], bd, atol=1e-9, err_msg=name)

    def test_components(self):
        lab = spatial.components(8, np.array([0, 1, 5, 6, 3]), np.array([1, 2, 6, 7, 3]))
        self.assertEqual(lab.tolist(), [0, 0, 0, 1, 2, 3, 3, 3])
        rng = np.random.default_rng(0)                                   # a long chain in random order
        perm = rng.permutation(5000)
        lab = spatial.components(5000, perm[:-1], perm[1:])
        self.assertTrue(np.all(lab == 0))


class CreateRandomizePromoteTests(unittest.TestCase):
    def test_create(self):
        g = chain(("attribute_create", {"name": "w", "type": "vector", "value": [1, 2, 3], "group": "0-4"}),
                  ("attribute_create", {"name": "id", "domain": "primitives", "type": "int", "value": [7, 0, 0]}),
                  ("attribute_create", {"name": "tag", "domain": "detail", "type": "float", "value": [2.5, 0, 0]}))
        np.testing.assert_array_equal(g.point["w"][:5], np.tile([1, 2, 3], (5, 1)))
        self.assertTrue(np.all(g.point["w"][5:] == 0))
        self.assertEqual(g.prim["id"].dtype, np.int32)
        self.assertEqual(g.detail["tag"], 2.5)
        with self.assertRaises(NodeError):
            chain(("attribute_create", {"name": "bad name"}))

    def test_randomize_ranges_seeds_and_pieces(self):
        g = chain(("attribute_randomize", {"name": "s", "type": "float", "min": [2, 0, 0], "max": [3, 0, 0], "seed": 1}))
        self.assertTrue(np.all((g.point["s"] >= 2) & (g.point["s"] <= 3)))
        g2 = chain(("attribute_randomize", {"name": "s", "type": "float", "min": [2, 0, 0], "max": [3, 0, 0], "seed": 1}))
        np.testing.assert_array_equal(g.point["s"], g2.point["s"])
        g = chain(("attribute_randomize", {"name": "n", "type": "int", "min": [1, 0, 0], "max": [3, 0, 0]}))
        self.assertEqual(set(g.point["n"].tolist()), {1, 2, 3})
        two = {"a": {"type": "box", "params": {"size": [1, 1, 1]}},
               "b": {"type": "box", "params": {"size": [1, 1, 1], "center": [3, 0, 0]}},
               "m": {"type": "merge", "inputs": {"geos": ["a", "b"]}}}
        g = chain(("connectivity", {}), ("attribute_randomize", {"domain": "primitives", "by": "class"}), src=two)
        self.assertEqual(len(np.unique(g.prim["Cd"][:6], axis=0)), 1)        # one color per piece
        self.assertFalse(np.allclose(g.prim["Cd"][0], g.prim["Cd"][6]))

    def test_promote(self):
        g = chain(("measure", {"measure": "area"}), ("attribute_promote", {"name": "area", "to": "points", "method": "sum"}))
        self.assertNotIn("area", g.prim)
        np.testing.assert_allclose(sorted(set(np.round(g.point["area"], 5))), [1, 2, 4])   # corner, edge, interior
        g = chain(("attribute_promote", {"name": "uv", "to": "primitives"}))
        np.testing.assert_allclose(g.prim["uv"][0], [0.125, 0.125])
        g = chain(("attribute_promote", {"name": "P*", "to": "detail", "method": "max", "keep_original": True}))
        self.assertIn("P", g.point)
        g = chain(("measure", {"measure": "area"}), ("attribute_promote", {"name": "area", "to": "detail", "method": "sum"}))
        self.assertAlmostEqual(g.detail["area"], 16.0, places=4)
        g = chain(("attribute_promote", {"name": "uv", "to": "vertices"}))
        self.assertEqual(g.vertex["uv"].shape, (64, 2))


class TransferBlurTests(unittest.TestCase):
    def test_transfer_with_distance_and_blend(self):
        nodes = {"g": {"type": "grid", "params": {"columns": 9, "rows": 9, "size_x": 8, "size_y": 8}},
                 "dot": {"type": "grid", "params": {"columns": 2, "rows": 2, "size_x": 0.01, "size_y": 0.01}},
                 "red": {"type": "color", "inputs": {"geo": "dot"}, "params": {"color": [1, 0, 0]}},
                 "t": {"type": "attribute_transfer", "inputs": {"geo": "g", "source": "red"},
                       "params": {"names": "Cd", "max_distance": 2.5, "blend_width": 1.0}}}
        g = cook(nodes)
        d = np.linalg.norm(g.P, axis=1)
        np.testing.assert_allclose(g.point["Cd"][d < 1.4, 0], 1, atol=1e-4)
        np.testing.assert_allclose(g.point["Cd"][d > 2.6, 0], 0)
        mid = (d > 1.6) & (d < 2.4)
        self.assertTrue(np.all((g.point["Cd"][mid, 0] > 0) & (g.point["Cd"][mid, 0] < 1)))
        nodes["t"]["params"]["names"] = "nothing"
        with self.assertRaises(NodeError):
            cook(nodes)

    def test_blur_smooths_noise_and_pins_boundary(self):
        g0 = chain(("noise", {"amplitude": [0, 0, 1], "frequency": 1.3}))
        g = chain(("noise", {"amplitude": [0, 0, 1], "frequency": 1.3}), ("attribute_blur", {"iterations": 20}))
        inner = np.abs(g.P[:, 0]) < 1.9
        inner &= np.abs(g.P[:, 1]) < 1.9
        self.assertLess(np.std(g.P[inner, 2]), np.std(g0.P[inner, 2]))
        np.testing.assert_allclose(g.P[~inner], g0.P[~inner])                # boundary pinned
        with self.assertRaises(NodeError):
            chain(("attribute_create", {"name": "i", "type": "int"}), ("attribute_blur", {"names": "i"}))


class ConnectivityMeasureColorSortTests(unittest.TestCase):
    def test_connectivity_and_pieces(self):
        three = {f"b{i}": {"type": "box", "params": {"size": [1, 1, 1], "center": [3 * i, 0, 0]}} for i in range(3)}
        three["m"] = {"type": "merge", "inputs": {"geos": ["b0", "b1", "b2"]}}
        g = chain(("connectivity", {}), ("connectivity", {"domain": "points", "name": "piece"}), ("measure", {"measure": "piece_volume"}),
                  src=three)
        self.assertEqual(g.prim["class"].tolist(), [0] * 6 + [1] * 6 + [2] * 6)
        self.assertEqual(g.point["piece"].tolist(), [0] * 8 + [1] * 8 + [2] * 8)
        np.testing.assert_allclose(g.prim["piece_volume"], 1.0, atol=1e-5)

    def test_measure_area_perimeter_curvature(self):
        g = chain(("measure", {"measure": "area"}), ("measure", {"measure": "perimeter"}))
        np.testing.assert_allclose(g.prim["area"], 1.0, atol=1e-6)
        np.testing.assert_allclose(g.prim["perimeter"], 4.0, atol=1e-6)
        s = chain(("measure", {"measure": "curvature", "name": "k"}),
                  src={"s": {"type": "sphere", "params": {"radius": 2, "rows": 30, "columns": 40}}})
        self.assertAlmostEqual(float(np.median(s.point["k"])), 0.5, places=2)

    def test_color_modes(self):
        g = chain(("color", {"color": [0.2, 0.4, 0.6]}))
        np.testing.assert_allclose(g.point["Cd"], np.tile([0.2, 0.4, 0.6], (25, 1)), atol=1e-6)
        g = chain(("color", {"mode": "ramp", "ramp_attrib": "P.x", "color": [0, 0, 0], "color2": [1, 1, 1]}))
        np.testing.assert_allclose(g.point["Cd"][:, 0], (g.P[:, 0] + 2) / 4, atol=1e-6)
        g = chain(("color", {"mode": "random", "domain": "primitives", "group": "0-3"}))
        self.assertTrue(np.all(g.prim["Cd"][4:] == 0))
        with self.assertRaises(NodeError):
            chain(("color", {"mode": "ramp", "ramp_attrib": "nope"}))

    def test_sort_keeps_geometry_consistent(self):
        base = chain(("measure", {"measure": "area"}))
        for params in ({"key": "random", "seed": 3}, {"key": "reverse"}, {"key": "x"},
                       {"domain": "primitives", "key": "random"}, {"domain": "primitives", "key": "y"}):
            g = chain(("measure", {"measure": "area"}), ("sort", params))
            corners = np.sort(g.P[g.topology.vertex_point].reshape(-1, 4, 3).round(4), axis=1)
            want = np.sort(base.P[base.topology.vertex_point].reshape(-1, 4, 3).round(4), axis=1)
            self.assertEqual(sorted(map(bytes, corners)), sorted(map(bytes, want)), params)   # same faces
        g = chain(("sort", {"key": "x"}))
        self.assertTrue(np.all(np.diff(g.P[:, 0]) >= 0))


class RayTests(unittest.TestCase):
    TERRAIN = {"g": {"type": "grid", "params": {"columns": 40, "rows": 40, "size_x": 10, "size_y": 10}},
               "n": {"type": "noise", "inputs": {"geo": "g"}, "params": {"amplitude": [0, 0, 1], "frequency": 0.3}},
               "c": {"type": "color", "inputs": {"geo": "n"}, "params": {"mode": "ramp", "ramp_attrib": "P.z"}}}

    def project(self, params, pts=None):
        nodes = dict(self.TERRAIN)
        nodes["pts"] = pts or {"type": "grid", "params": {"columns": 12, "rows": 12, "size_x": 8, "size_y": 8,
                                                          "center": [0, 0, 5]}}
        nodes["r"] = {"type": "ray", "inputs": {"geo": "pts", "target": "c"}, "params": params}
        return cook(nodes), Evaluator().cook(Graph.from_dict({"nodes": {**self.TERRAIN, "o": {
            "type": "output", "inputs": {"geo": "c"}}}, "display": "o"}))

    def height_at(self, terrain, xy):
        """Brute-force reference: cast straight down onto every triangle."""
        from minihou.nodes.model import triangulate
        from minihou.raycast import _moller_trumbore
        tv, _, _ = triangulate(terrain, np.ones(terrain.nprims, bool))
        tris = terrain.P.astype(np.float64)[terrain.topology.vertex_point[tv]]
        out = []
        for x, y in xy:
            o = np.tile([x, y, 50.0], (len(tris), 1))
            t, u, v, ok = _moller_trumbore(o, np.tile([0, 0, -1.0], (len(tris), 1)), tris)
            out.append(50.0 - t[ok].min())
        return np.array(out)

    def test_direction_projects_onto_the_surface_and_imports_attributes(self):
        g, terrain = self.project({"direction": [0, 0, -1], "import_attribs": "Cd"})
        np.testing.assert_allclose(g.P[:, 2], self.height_at(terrain, g.P[:, :2]), atol=1e-4)
        self.assertTrue(np.all(g.point["group_rayHit"] == 1))
        np.testing.assert_allclose(g.point["hitdist"], 5 - g.P[:, 2], atol=1e-4)
        z = terrain.P[:, 2]
        np.testing.assert_allclose(g.point["Cd"][:, 0], 1 + (0.1 - 1) * (g.P[:, 2] - z.min()) / np.ptp(z), atol=2e-2)
        self.assertTrue(np.all(g.point["hitN"][:, 2] > 0.5))

    def test_misses_scale_lift_and_both_ways(self):
        g, _ = self.project({"direction": [0, 0, 1]})                         # pointing away: nothing hit
        self.assertTrue(np.all(g.point["hitdist"] == -1) and np.allclose(g.P[:, 2], 5))
        g, _ = self.project({"direction": [0, 0, 1], "both_ways": True, "scale": 0.5})
        self.assertTrue(np.all(g.point["hitdist"] > 0))
        self.assertTrue(np.all(g.P[:, 2] > 1.5) and np.all(g.P[:, 2] < 3.5))  # half way down
        g, _ = self.project({"direction": [0, 0, 1], "delete_misses": True})
        self.assertEqual(g.npoints, 0)
        lifted, _ = self.project({"lift": 0.25})
        base, _ = self.project({})
        self.assertGreater(float((lifted.P[:, 2] - base.P[:, 2]).min()), 0.2)

    def test_minimum_distance_and_normals(self):
        sphere = {"type": "sphere", "params": {"radius": 3, "rows": 24, "columns": 32}}
        nodes = {"s": sphere, "pts": {"type": "sphere", "params": {"radius": 1, "rows": 6, "columns": 8}},
                 "nn": {"type": "normal", "inputs": {"geo": "pts"}},
                 "r": {"type": "ray", "inputs": {"geo": "nn", "target": "s"}, "params": {"method": "normal"}}}
        g = cook(nodes)
        r = np.linalg.norm(g.P, axis=1)
        self.assertTrue(np.all((r > 2.9) & (r <= 3.0 + 1e-5)))
        nodes["r"]["params"] = {"method": "minimum_distance"}
        g = cook(nodes)
        r = np.linalg.norm(g.P, axis=1)
        self.assertTrue(np.all((r > 2.9) & (r <= 3.0 + 1e-5)))
        nodes["r"]["params"] = {"method": "normal"}
        nodes["r"]["inputs"]["geo"] = "pts"
        with self.assertRaises(NodeError):
            cook(nodes)


if __name__ == "__main__":
    unittest.main()
