"""PCG points: exact box overlaps, point booleans, self-pruning, samplers, density tools, projection, spawning."""
import unittest

import numpy as np

from minihou import Evaluator, Graph, pcg, spatial


def cook(nodes, display):
    nodes = dict(nodes)
    nodes["out"] = {"type": "output", "inputs": {"geo": display}}
    g = Evaluator().cook(Graph.from_dict({"nodes": nodes, "display": "out"}))
    g.validate()
    return g


GROUND = {"type": "grid", "params": {"size_x": 20, "size_y": 20, "columns": 21, "rows": 21}}


def brute_overlap(a, b, n=9):
    """Box overlap by sampling points of a's boxes inside b's (pairwise, row by row)."""
    Ca, Aa, Ha = pcg.boxes(a)
    Cb, Ab, Hb = pcg.boxes(b)
    g = np.linspace(-1, 1, n)
    G = np.stack(np.meshgrid(g, g, g, indexing="ij"), -1).reshape(-1, 3)
    pts = Ca[:, None] + np.einsum("mij,mkj->mki", Aa, G[None] * Ha[:, None])
    loc = np.einsum("mji,mkj->mki", Ab, pts - Cb[:, None])
    return np.any(np.all(np.abs(loc) < Hb[:, None] - 1e-9, 2), 1)


class BoxTests(unittest.TestCase):
    def test_separating_axes_match_sampling(self):
        rng = np.random.default_rng(3)
        m = 600
        for z_only in (False, True):                         # z_only: parallel axes, the degenerate cross axes
            def boxes():
                rot = rng.uniform(-3, 3, (m, 3))
                if z_only:
                    rot[:, :2] = 0
                return {"P": rng.uniform(-1.5, 1.5, (m, 3)), "rot": rot, "scale": rng.uniform(0.3, 1.5, (m, 3)),
                        "bounds_min": -rng.uniform(0.1, 0.6, (m, 3)), "bounds_max": rng.uniform(0.1, 0.6, (m, 3))}
            a, b = boxes(), boxes()
            Ca, Aa, Ha = pcg.boxes(a)
            Cb, Ab, Hb = pcg.boxes(b)
            sat = pcg.obb_overlap(Ca, Aa, Ha, Cb, Ab, Hb)
            brute = brute_overlap(a, b) | brute_overlap(b, a)
            self.assertEqual(int((brute & ~sat).sum()), 0, "every sampled overlap is found")
            self.assertLessEqual(int((sat & ~brute).sum()), m // 50)   # only slivers thinner than the samples

    def test_greedy_matches_sequential(self):
        rng = np.random.default_rng(1)
        n = 300
        P = rng.uniform(0, 5, (n, 2))
        i, j = np.nonzero(np.triu(np.linalg.norm(P[:, None] - P[None], axis=2) < 0.5, 1))
        pri = rng.random(n)
        adj = [set() for _ in range(n)]
        for x, y in zip(i, j):
            adj[x].add(y)
            adj[y].add(x)
        seq = np.zeros(n, bool)
        for v in np.argsort(-pri):
            if not any(seq[u] for u in adj[v]):
                seq[v] = True
        np.testing.assert_array_equal(pcg.greedy_independent(n, i, j, pri), seq)


class PCGNodeTests(unittest.TestCase):
    def setUp(self):
        self.base = {
            "ground": GROUND,
            "pts": {"type": "pcg_surface_sampler", "inputs": {"surface": "ground"},
                    "params": {"points_per_m2": 1.0, "extents": [0.8, 0.8, 1.0], "looseness": 0.0}},
            "jit": {"type": "pcg_transform", "inputs": {"points": "pts"},
                    "params": {"scale_min": [0.7, 0.7, 0.7], "scale_max": [1.5, 1.5, 1.5]}},
            "house": {"type": "box", "params": {"size": [6, 4, 4]}},
        }

    def test_surface_sampler(self):
        g = cook(self.base, "pts")
        self.assertAlmostEqual(g.npoints, 400, delta=40)     # 20 x 20 m at 1 per square metre
        self.assertTrue(np.allclose(g.P[:, 2], 0) and np.all(np.abs(g.P[:, :2]) <= 10 + 1e-6))
        for name in ("rot", "scale", "bounds_min", "bounds_max", "density", "seed"):
            self.assertIn(name, g.point)
        i, j, dist = spatial.pairs_within(g.P.astype(float), 0.5)
        self.assertEqual(len(i), 0, "looseness 0 keeps points apart (blue noise)")

    def test_self_prune_leaves_no_overlaps_and_is_maximal(self):
        nodes = {**self.base, "prune": {"type": "pcg_self_prune", "inputs": {"points": "jit"}}}
        before, after = cook(nodes, "jit"), cook(nodes, "prune")
        self.assertLess(after.npoints, before.npoints)
        a = pcg.read(after)
        self.assertEqual(len(pcg.overlapping_pairs(a)[0]), 0)
        b = pcg.read(before)
        kept = {tuple(p) for p in np.round(after.P, 5)}
        removed = np.array([tuple(p) not in kept for p in np.round(before.P, 5)])
        sub = {k: v[removed] for k, v in b.items()}
        i, _ = pcg.overlapping_pairs(sub, a)
        self.assertEqual(len(np.unique(i)), int(removed.sum()), "every pruned point overlaps a kept one")
        big = cook({**nodes, "prune": {"type": "pcg_self_prune", "inputs": {"points": "jit"},
                                       "params": {"priority": "large_first"}}}, "prune")
        small = cook({**nodes, "prune": {"type": "pcg_self_prune", "inputs": {"points": "jit"},
                                         "params": {"priority": "small_first"}}}, "prune")
        self.assertGreater(big.point["scale"][:, 0].mean(), small.point["scale"][:, 0].mean())

    def test_difference_and_intersection(self):
        nodes = {**self.base,
                 "nohouse": {"type": "pcg_difference", "inputs": {"points": "jit", "subtract": "house"}},
                 "inhouse": {"type": "pcg_intersection", "inputs": {"points": "jit", "other": "house"}},
                 "road": {"type": "curve", "params": {"points": "-10,-3,0 10,3,0"}},
                 "road_pts": {"type": "pcg_spline_sampler", "inputs": {"curve": "road"},
                              "params": {"spacing": 1.0, "extents": [0.6, 1.5, 1.0]}},
                 "noroad": {"type": "pcg_difference", "inputs": {"points": "jit", "subtract": "road_pts"}},
                 "soft": {"type": "pcg_difference", "inputs": {"points": "jit", "subtract": "road_pts"},
                          "params": {"density_function": "subtract"}}}
        allp, out, inn = cook(nodes, "jit"), cook(nodes, "nohouse"), cook(nodes, "inhouse")
        self.assertEqual(out.npoints + inn.npoints, allp.npoints)       # each point is in exactly one
        self.assertFalse(np.any((np.abs(out.P[:, 0]) < 3) & (np.abs(out.P[:, 1]) < 2)), "no centre in the house")
        road, kept = cook(nodes, "road_pts"), cook(nodes, "noroad")
        self.assertEqual(len(pcg.overlapping_pairs(pcg.read(kept), pcg.read(road))[0]), 0)
        soft = cook(nodes, "soft")
        self.assertGreaterEqual(soft.npoints, kept.npoints)
        self.assertTrue(np.all(soft.point["density"] > 0) and soft.point["density"].min() < 1)

    def test_density_tools(self):
        nodes = {**self.base,
                 "noise": {"type": "pcg_density_noise", "inputs": {"points": "pts"},
                           "params": {"noise": "perlin", "frequency": 0.2}},
                 "filt": {"type": "pcg_density_filter", "inputs": {"points": "noise"}, "params": {"lower": 0.5}},
                 "prob": {"type": "pcg_density_filter", "inputs": {"points": "noise"},
                          "params": {"probabilistic": True}},
                 "road": {"type": "curve", "params": {"points": "-10,0,0 10,0,0"}},
                 "dist": {"type": "pcg_distance", "inputs": {"points": "pts", "target": "road"},
                          "params": {"max_distance": 5.0}}}
        noise, filt = cook(nodes, "noise"), cook(nodes, "filt")
        d = noise.point["density"]
        self.assertTrue(d.min() >= 0 and d.max() <= 1 and d.std() > 0.05)
        self.assertEqual(filt.npoints, int((d >= 0.5).sum()))
        self.assertAlmostEqual(cook(nodes, "prob").npoints / noise.npoints, float(d.mean()), delta=0.08)
        dist = cook(nodes, "dist")
        np.testing.assert_allclose(dist.point["density"], np.clip(np.abs(dist.P[:, 1]) / 5.0, 0, 1), atol=1e-5)

    def test_transform_follows_point_seeds(self):
        nodes = {**self.base, "sort": {"type": "sort", "inputs": {"geo": "pts"}, "params": {"key": "random"}},
                 "t2": {"type": "pcg_transform", "inputs": {"points": "sort"},
                        "params": {"scale_min": [0.7, 0.7, 0.7], "scale_max": [1.5, 1.5, 1.5]}}}
        a, b = cook(nodes, "jit"), cook(nodes, "t2")
        key = lambda g: {tuple(np.round(p, 4)): s for p, s in zip(g.P, g.point["scale"][:, 0])}  # noqa: E731
        self.assertEqual(key(a), key(b), "reordering points does not change their random values")

    def test_volume_spline_projection(self):
        vol = cook({"b": {"type": "box", "params": {"size": [4, 4, 4]}},
                    "v": {"type": "pcg_volume_sampler", "inputs": {"volume": "b"}, "params": {"voxel_size": [1, 1, 1]}}},
                   "v")
        self.assertEqual(vol.npoints, 64)
        self.assertTrue(np.all(np.abs(vol.P) < 2))
        lot = {"lot": {"type": "curve", "params": {"points": "0,0,0 10,0,0 10,6,0 0,6,0", "closed": True}},
               "in": {"type": "pcg_spline_sampler", "inputs": {"curve": "lot"},
                      "params": {"mode": "interior", "spacing": 1.0}},
               "terrain": {"type": "noise", "inputs": {"geo": "ground"},
                           "params": {"amplitude": [0, 0, 1], "frequency": 0.1}},
               "ground": GROUND,
               "proj": {"type": "pcg_projection", "inputs": {"points": "in", "surface": "terrain"}},
               "on": {"type": "pcg_spline_sampler", "inputs": {"curve": "lot"}, "params": {"spacing": 2.0}}}
        inside = cook(lot, "in")
        self.assertEqual(inside.npoints, 60)
        on = cook(lot, "on")
        self.assertEqual(on.npoints, 16)                       # 32 m of outline every 2 m
        proj = cook(lot, "proj")
        from minihou.raycast import TriGrid
        from minihou.nodes._util import fan_triangulate
        t = cook(lot, "terrain")
        tris, _ = fan_triangulate(t.topology)
        cp, _, _ = TriGrid(t.P[t.topology.vertex_point[tris]].astype(float)).closest(proj.P.astype(float))
        self.assertLess(np.abs(cp - proj.P).max(), 1e-4, "points sit on the terrain")

    def test_spawner_copy_debug_union(self):
        nodes = {**self.base,
                 "spawn": {"type": "pcg_spawner", "inputs": {"points": "pts"}, "params": {"objects": "Tree:3 Rock:1"}},
                 "ring": {"type": "pcg_volume_sampler", "inputs": {"volume": "house"}, "params": {"voxel_size": [2, 2, 2]}},
                 "cp": {"type": "pcg_copy_points", "inputs": {"source": "ring", "targets": "pts"}},
                 "dbg": {"type": "pcg_debug", "inputs": {"points": "jit"}},
                 "uni": {"type": "pcg_union", "inputs": {"inputs": ["pts", "ring"]},
                         "params": {"density_function": "maximum"}}}
        s = cook(nodes, "spawn")
        self.assertEqual(s.detail["instance_objects"], ["Tree", "Rock"])
        self.assertAlmostEqual(float((s.point["instance"] == 1).mean()), 0.75, delta=0.08)
        n, r = cook(nodes, "pts").npoints, cook(nodes, "ring").npoints
        self.assertEqual(cook(nodes, "cp").npoints, n * r)
        dbg = cook(nodes, "dbg")
        self.assertEqual((dbg.npoints, dbg.nprims), (8 * n, 6 * n))
        self.assertEqual(cook(nodes, "uni").npoints, n + r)


if __name__ == "__main__":
    unittest.main()
