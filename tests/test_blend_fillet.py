"""fillet (rounded curve corners) and blend (morph between shapes with the same points)."""
import unittest

import numpy as np

from minihou import Evaluator, Graph, NodeError


def cook(nodes, out, **kw):
    nodes = dict(nodes)
    nodes["out"] = {"type": "output", "inputs": {"geo": out}}
    return Evaluator().cook(Graph.from_dict({"nodes": nodes, "display": "out"}), **kw)


def curve(points, closed=False):
    return {"c": {"type": "curve", "params": {"points": " ".join(",".join(map(str, p)) for p in points), "closed": closed}}}


def fillet(points, closed=False, **params):
    n = curve(points, closed)
    n["f"] = {"type": "fillet", "inputs": {"geo": "c"}, "params": params}
    return cook(n, "f")


def length(P, closed):
    d = np.diff(np.vstack([P, P[:1]]) if closed else P, axis=0)
    return float(np.linalg.norm(d, axis=1).sum())


SQUARE = [(0, 0, 0), (2, 0, 0), (2, 2, 0), (0, 2, 0)]


class FilletTests(unittest.TestCase):
    def test_a_square_gets_quarter_circle_corners_of_the_given_radius(self):
        g = fillet(SQUARE, True, radius=0.5, divisions=4)
        self.assertEqual(g.npoints, 4 * 5)
        self.assertTrue(bool(g.topology.closed_mask()[0]))                        # still a closed curve
        P = g.P.astype(float)
        for corner, centre in zip(range(4), [(0.5, 0.5), (1.5, 0.5), (1.5, 1.5), (0.5, 1.5)]):
            arc = P[corner * 5:(corner + 1) * 5, :2]
            np.testing.assert_allclose(np.linalg.norm(arc - np.array(centre), axis=1), 0.5, atol=1e-6)   # every arc point at r from its centre
        np.testing.assert_allclose(P[0], [0, 0.5, 0], atol=1e-6)                  # the arc starts where it leaves the edge
        np.testing.assert_allclose(P[4], [0.5, 0, 0], atol=1e-6)

    def test_the_perimeter_matches_the_exact_rounded_square(self):
        g = fillet(SQUARE, True, radius=0.4, divisions=64)
        exact = 4 * (2 - 2 * 0.4) + 2 * np.pi * 0.4
        self.assertAlmostEqual(length(g.P.astype(float), True), exact, delta=exact * 2e-3)

    def test_open_curves_keep_their_ends_and_round_the_inside_corners(self):
        pts = [(0, 0, 0), (2, 0, 0), (2, 2, 0), (4, 2, 0)]
        g = fillet(pts, False, radius=0.5, divisions=3)
        self.assertEqual(g.npoints, 2 + 2 * 4)
        self.assertFalse(bool(g.topology.closed_mask()[0]))
        np.testing.assert_allclose(g.P[0], [0, 0, 0])
        np.testing.assert_allclose(g.P[-1], [4, 2, 0])

    def test_the_curve_is_smooth_where_the_arcs_meet_the_edges(self):
        g = fillet(SQUARE, True, radius=0.5, divisions=8)
        P = g.P.astype(float)
        d = np.diff(np.vstack([P, P[:1]]), axis=0)
        d /= np.linalg.norm(d, axis=1)[:, None]
        turn = np.degrees(np.arccos(np.clip((d * np.roll(d, -1, axis=0)).sum(1), -1, 1)))
        self.assertLess(turn.max(), 90 / 8 * 1.2 + 1e-3)                          # no sharp corner left: each step turns by about 90/8 degrees

    def test_short_segments_limit_the_radius_so_arcs_never_overlap(self):
        g = fillet(SQUARE, True, radius=50.0, divisions=16)                      # far too big: each corner may use half of each side
        P = g.P.astype(float)
        np.testing.assert_allclose(np.linalg.norm(P[:, :2] - np.array([1.0, 1.0]), axis=1), 1.0, atol=1e-5)   # the inscribed circle
        self.assertTrue(np.isfinite(P).all())

    def test_straight_and_gentle_corners_are_left_alone(self):
        pts = [(0, 0, 0), (1, 0, 0), (2, 0, 0), (2, 1, 0)]                      # the middle point is collinear
        self.assertEqual(fillet(pts, False, radius=0.3, divisions=4).npoints, 4 + 4 - 1 + 1 - 1 + 0 + 1 - 1 + 1)
        gentle = [(0, 0, 0), (1, 0, 0), (2, 0.01, 0)]                           # turns by about half a degree
        self.assertEqual(fillet(gentle, False, radius=0.3, divisions=4, min_angle=2.0).npoints, 3)
        self.assertGreater(fillet(gentle, False, radius=0.1, divisions=4, min_angle=0.1).npoints, 3)

    def test_group_and_zero_radius(self):
        n = curve(SQUARE, True)
        n["f"] = {"type": "fillet", "inputs": {"geo": "c"}, "params": {"radius": 0.4, "divisions": 3, "group": "0 2"}}
        self.assertEqual(cook(n, "f").npoints, 2 * 4 + 2)                       # two corners rounded, two left
        n["f"]["params"] = {"radius": 0.0, "divisions": 3}
        self.assertEqual(cook(n, "f").npoints, 4)

    def test_a_three_dimensional_corner_stays_in_its_plane(self):
        pts = [(0, 0, 0), (2, 0, 0), (2, 1, 1.5)]
        g = fillet(pts, False, radius=0.4, divisions=6)
        P = g.P.astype(float)
        normal = np.cross(np.array(pts[1]) - pts[0], np.array(pts[2]) - pts[1])
        normal /= np.linalg.norm(normal)
        self.assertLess(np.abs((P - np.array(pts[0])) @ normal).max(), 1e-5)
        # on the inside of the corner: no point beyond the two edges
        self.assertTrue(np.isfinite(P).all())

    def test_attributes_are_blended_and_other_data_follows(self):
        n = curve([(0, 0, 0), (2, 0, 0), (2, 2, 0)])
        n["w"] = {"type": "wrangle", "inputs": {"geo": "c"}, "params": {"snippet": "f@pscale = @ptnum * 1.0; i@id = @ptnum + 10; @Cd = set(@ptnum, 0, 0);",
                                                                        "run_over": "points"}}
        n["pp"] = {"type": "wrangle", "inputs": {"geo": "w"}, "params": {"snippet": "i@tag = 7;", "run_over": "primitives"}}
        n["f"] = {"type": "fillet", "inputs": {"geo": "pp"}, "params": {"radius": 0.5, "divisions": 4}}
        g = cook(n, "f")
        ps = g.point["pscale"]
        self.assertAlmostEqual(float(ps[0]), 0.0, places=5)
        self.assertAlmostEqual(float(ps[-1]), 2.0, places=5)
        self.assertTrue((np.diff(ps) >= -1e-6).all())                            # runs smoothly along the curve (monotone: 0, ..., 2)
        self.assertTrue((ps >= 0).all() and (ps <= 2).all())
        self.assertTrue(((ps > 0.05) & (ps < 0.95)).any() or ((ps > 1.05) & (ps < 1.95)).any())   # arc points get in-between values, not a copy
        self.assertIn("id", g.point)                                            # integer attributes come along
        self.assertEqual(int(g.prim["tag"][0]), 7)                              # primitive attributes too

    def test_polygon_outlines_and_many_curves_at_once(self):
        n = {"b": {"type": "box", "params": {"size": [1, 1, 1]}}, "f": {"type": "fillet", "inputs": {"geo": "b"}, "params": {"radius": 0.1, "divisions": 2}}}
        g = cook(n, "f")
        self.assertEqual(g.nprims, 6)
        self.assertEqual(g.npoints, 6 * 4 * 3)                                  # each quad outline: four corners of three points
        self.assertTrue(g.topology.closed_mask().all())

    def test_hairpins_and_degenerate_input_do_not_break(self):
        g = fillet([(0, 0, 0), (1, 0, 0), (0, 0, 0)], False, radius=0.2, divisions=3)
        self.assertTrue(np.isfinite(g.P).all())
        g = fillet([(0, 0, 0), (0, 0, 0), (1, 0, 0), (1, 1, 0)], False, radius=0.2, divisions=3)
        self.assertTrue(np.isfinite(g.P).all())
        two = fillet([(0, 0, 0), (1, 0, 0)], False, radius=0.2, divisions=3)
        self.assertEqual(two.npoints, 2)

    def test_the_arc_count_is_guarded(self):
        n = curve(SQUARE, True)
        n["f"] = {"type": "fillet", "inputs": {"geo": "c"}, "params": {"radius": 0.4, "divisions": 256}}
        self.assertEqual(cook(n, "f").npoints, 4 * 257)
        from minihou.nodes import _util
        old = _util.MAX_GENERATED
        try:
            _util.MAX_GENERATED = 100
            with self.assertRaisesRegex(NodeError, "fillet would make"):
                cook(n, "f")
        finally:
            _util.MAX_GENERATED = old

    def test_random_polylines_give_finite_results_with_the_expected_counts(self):
        rng = np.random.default_rng(3)
        for _ in range(25):
            P = rng.uniform(-3, 3, (int(rng.integers(3, 9)), 3)).round(3)
            g = fillet([tuple(p) for p in P], bool(rng.integers(0, 2)), radius=float(rng.uniform(0.01, 2)), divisions=int(rng.integers(1, 9)))
            self.assertTrue(np.isfinite(g.P).all())
            self.assertGreaterEqual(g.npoints, len(P))
            lo, hi = P.min(0) - 1e-4, P.max(0) + 1e-4
            self.assertTrue(((g.P >= lo) & (g.P <= hi)).all())                  # rounding stays inside the control polygon's box


def box_pair():
    return {"a": {"type": "box", "params": {"size": [1, 1, 1]}}, "t": {"type": "box", "params": {"size": [3, 1, 1], "center": [0, 0, 2]}}}


class BlendTests(unittest.TestCase):
    def blend(self, params=None, targets=("t",), extra=None):
        n = dict(box_pair(), **(extra or {}))
        n["bl"] = {"type": "blend", "inputs": {"geo": "a", "targets": list(targets)}, "params": params or {}}
        return cook(n, "bl")

    def test_amount_runs_from_the_base_to_the_target(self):
        base, target = cook(box_pair(), "a"), cook(box_pair(), "t")
        for amount in (0.0, 0.25, 1.0, 1.5):
            g = self.blend({"amount": amount})
            np.testing.assert_allclose(g.P, base.P + amount * (target.P - base.P), atol=1e-5)
        np.testing.assert_allclose(self.blend({"amount": 1.0}).P, target.P, atol=1e-6)         # the target reached exactly
        self.assertEqual(self.blend().topology.vertex_point.tolist(), base.topology.vertex_point.tolist())   # topology from the base

    def test_several_targets_add_their_offsets(self):
        n = dict(box_pair(), u={"type": "box", "params": {"size": [1, 4, 1]}})
        n["bl"] = {"type": "blend", "inputs": {"geo": "a", "targets": ["t", "u"]}, "params": {"weights": "0.5; 1.0"}}
        g = cook(n, "bl")
        a, t, u = (cook(n, k).P.astype(float) for k in ("a", "t", "u"))
        np.testing.assert_allclose(g.P, a + 0.5 * (t - a) + 1.0 * (u - a), atol=1e-5)
        n["bl"]["params"] = {"weights": "0.25"}                                              # one weight: applied to every target
        np.testing.assert_allclose(cook(n, "bl").P, a + 0.25 * (t - a) + 0.25 * (u - a), atol=1e-5)

    def test_weights_can_be_expressions_of_time_and_recook_with_the_frame(self):
        n = dict(box_pair())
        n["bl"] = {"type": "blend", "inputs": {"geo": "a", "targets": ["t"]}, "params": {"weights": "clamp((@Frame - 1) / 10, 0, 1)"}}
        g = Graph.from_dict({"nodes": dict(n, out={"type": "output", "inputs": {"geo": "bl"}}), "display": "out"})
        ev = Evaluator()
        a, t = cook(n, "a").P.astype(float), cook(n, "t").P.astype(float)
        np.testing.assert_allclose(ev.cook(g, frame=1).P, a, atol=1e-6)
        np.testing.assert_allclose(ev.cook(g, frame=6).P, a + 0.5 * (t - a), atol=1e-5)
        np.testing.assert_allclose(ev.cook(g, frame=40).P, t, atol=1e-5)                    # clamped at 1
        again = ev.cook(g, frame=6)
        self.assertEqual(len(ev.last_stats.cooked), 0)                                           # the same frame is a cache hit

    def test_mask_scales_the_blend_per_point_and_group_restricts_it(self):
        extra = {"m": {"type": "wrangle", "inputs": {"geo": "a"}, "params": {"snippet": "f@m = (@P.z > 0) * 1.0;", "run_over": "points"}},
                 }
        n = dict(box_pair(), **extra)
        n["bl"] = {"type": "blend", "inputs": {"geo": "m", "targets": ["t"]}, "params": {"mask": "m"}}
        g = cook(n, "bl")
        base, target = cook(n, "m"), cook(n, "t")
        top = base.P[:, 2] > 0
        np.testing.assert_allclose(g.P[top], target.P[top], atol=1e-6)                      # masked in
        np.testing.assert_allclose(g.P[~top], base.P[~top], atol=1e-6)                      # masked out
        n["bl"]["params"] = {"group": "0-3"}
        g = cook(n, "bl")
        np.testing.assert_allclose(g.P[:4], target.P[:4], atol=1e-6)
        np.testing.assert_allclose(g.P[4:], base.P[4:], atol=1e-6)

    def test_other_attributes_can_be_blended_and_star_means_every_shared_float(self):
        n = {"a": {"type": "box", "params": {"size": [1, 1, 1]}}, "t": {"type": "box", "params": {"size": [1, 1, 1]}}}
        n["a2"] = {"type": "wrangle", "inputs": {"geo": "a"}, "params": {"snippet": "v@Cd = set(1, 0, 0); f@pscale = 1.0;", "run_over": "points"}}
        n["t2"] = {"type": "wrangle", "inputs": {"geo": "t"}, "params": {"snippet": "v@Cd = set(0, 0, 1); f@pscale = 3.0; f@extra = 1.0;", "run_over": "points"}}
        n["bl"] = {"type": "blend", "inputs": {"geo": "a2", "targets": ["t2"]}, "params": {"attributes": "*", "amount": 0.5}}
        g = cook(n, "bl")
        np.testing.assert_allclose(g.point["Cd"], np.tile([0.5, 0, 0.5], (8, 1)), atol=1e-6)
        np.testing.assert_allclose(g.point["pscale"], 2.0, atol=1e-6)
        self.assertNotIn("extra", g.point)                                                  # only what the base has

    def test_clear_errors(self):
        n = dict(box_pair(), s={"type": "sphere", "params": {"radius": 1}})
        n["bl"] = {"type": "blend", "inputs": {"geo": "a", "targets": ["s"]}}
        with self.assertRaisesRegex(NodeError, "target 1 has .* points but the base has 8"):
            cook(n, "bl")
        n["bl"] = {"type": "blend", "inputs": {"geo": "a", "targets": ["t"]}, "params": {"weights": "1; 2"}}
        with self.assertRaisesRegex(NodeError, "2 weights for 1 targets"):
            cook(n, "bl")
        for bad, msg in (("__import__('os')", "only numbers"), ("foo", "unknown name"), ("1 +", "weight"), ("sin()", "arguments"), ("1/0", "cannot compute")):
            n["bl"]["params"] = {"weights": bad}
            with self.assertRaisesRegex(NodeError, msg):
                cook(n, "bl")
        n["bl"]["params"] = {"attributes": "nope"}
        with self.assertRaisesRegex(NodeError, "no point attribute 'nope'"):
            cook(n, "bl")

    def test_no_targets_passes_the_base_through(self):
        n = dict(box_pair())
        n["bl"] = {"type": "blend", "inputs": {"geo": "a"}}
        np.testing.assert_allclose(cook(n, "bl").P, cook(n, "a").P)


if __name__ == "__main__":
    unittest.main()
