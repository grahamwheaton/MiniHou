"""2-D Delaunay triangulation: the empty-circle property, full hull coverage, orientation, degenerate input."""
import unittest

import numpy as np

from minihou import Evaluator, Geometry, Graph, delaunay as dl
from tests.helpers import quad_geo
from tests.test_nodes import run


def hull_area(xy):
    """Area of the convex hull (monotone chain)."""
    pts = sorted(map(tuple, np.unique(xy, axis=0).tolist()))

    def half(seq):
        h = []
        for p in seq:
            while len(h) >= 2 and (h[-1][0] - h[-2][0]) * (p[1] - h[-2][1]) - (h[-1][1] - h[-2][1]) * (p[0] - h[-2][0]) <= 0:
                h.pop()
            h.append(p)
        return h[:-1]

    hull = np.array(half(pts) + half(pts[::-1]))
    x, y = hull[:, 0], hull[:, 1]
    return 0.5 * abs(np.dot(x, np.roll(y, -1)) - np.dot(y, np.roll(x, -1)))


def circle_violations(xy, tri):
    bad = 0
    for t in tri:
        (ax, ay), (bx, by), (cx, cy) = xy[t]
        d = 2 * (ax * (by - cy) + bx * (cy - ay) + cx * (ay - by))
        ux = ((ax * ax + ay * ay) * (by - cy) + (bx * bx + by * by) * (cy - ay) + (cx * cx + cy * cy) * (ay - by)) / d
        uy = ((ax * ax + ay * ay) * (cx - bx) + (bx * bx + by * by) * (ax - cx) + (cx * cx + cy * cy) * (bx - ax)) / d
        r = np.hypot(ax - ux, ay - uy)
        bad += int((np.hypot(xy[:, 0] - ux, xy[:, 1] - uy) < r * (1 - 1e-9)).sum())
    return bad


def exact_violations(xy, tri):
    """Points strictly inside some triangle's circumcircle, by exact rational arithmetic (for ill-conditioned sets)."""
    from fractions import Fraction
    pts = [(Fraction(float(x)), Fraction(float(y))) for x, y in xy]
    bad = 0
    for t in tri.tolist():
        a, b, c = (pts[i] for i in t)
        for k, d in enumerate(pts):
            if k in t:
                continue
            ax, ay, bx, by, cx, cy = a[0] - d[0], a[1] - d[1], b[0] - d[0], b[1] - d[1], c[0] - d[0], c[1] - d[1]
            det = ((ax * ax + ay * ay) * (bx * cy - cx * by) - (bx * bx + by * by) * (ax * cy - cx * ay)
                   + (cx * cx + cy * cy) * (ax * by - bx * ay))
            bad += det > 0                                       # counter-clockwise triangle: inside the circle
    return bad




BACKENDS = [True, False] if dl._Delaunay is not None else [False]


class TriangulateTests(unittest.TestCase):
    def test_random_points_are_delaunay_and_fill_the_hull(self):
        rng = np.random.default_rng(0)
        for n in (3, 4, 25, 400):
            xy = rng.random((n, 2)) * [3.0, 1.0]
            for us in BACKENDS:
                tri = dl.triangulate(xy, use_scipy=us)
                self.assertEqual(circle_violations(xy, tri), 0, (n, us))
                self.assertTrue(np.all(dl._area2(xy, tri) > 0))                       # counter-clockwise
                self.assertAlmostEqual(dl._area2(xy, tri).sum() / 2, hull_area(xy), places=9)
                self.assertEqual(len(tri), 2 * n - 2 - len(set(map(tuple, _hull_vertices(xy)))))   # Euler's formula

    def test_backends_agree(self):
        if len(BACKENDS) < 2:
            self.skipTest("scipy not installed")
        xy = np.random.default_rng(1).random((1500, 2))
        sets = [{tuple(sorted(t)) for t in dl.triangulate(xy, use_scipy=us).tolist()} for us in BACKENDS]
        self.assertEqual(sets[0], sets[1])

    def test_cocircular_grid_and_duplicates(self):
        g = np.stack(np.meshgrid(np.arange(12.0), np.arange(9.0)), -1).reshape(-1, 2)
        dup = np.concatenate([g, g[:20]])
        for us in BACKENDS:
            for pts in (g, dup):
                tri = dl.triangulate(pts, use_scipy=us)
                self.assertAlmostEqual(dl._area2(pts, tri).sum() / 2, 11 * 8)
                self.assertEqual(len(tri), 2 * 11 * 8)
                self.assertTrue(np.all(tri < len(g)))                                # duplicates map to the first copy

    def test_degenerate_inputs(self):
        for us in BACKENDS:
            for pts in (np.zeros((0, 2)), np.zeros((2, 2)), np.ones((5, 2)), np.c_[np.arange(8.0), np.arange(8.0) * 2]):
                self.assertEqual(dl.triangulate(pts, use_scipy=us).shape, (0, 3))

    def test_far_apart_scales_and_large_offsets(self):
        rng = np.random.default_rng(2)
        for scale, offset in ((1e-3, 0.0), (1.0, 1e3), (1e3, -5e3)):       # float32-representable, well conditioned
            xy = np.concatenate([rng.random((40, 2)) * scale, rng.random((40, 2)) * scale + [offset + 20 * scale] * 2])
            xy = xy.astype(np.float32).astype(np.float64)
            for us in BACKENDS:
                tri = dl.triangulate(xy, use_scipy=us)
                self.assertEqual(exact_violations(xy, tri), 0, (scale, us))
                self.assertAlmostEqual(dl._area2(xy, tri).sum() / 2, hull_area(xy), delta=1e-5 * hull_area(xy))


def _hull_vertices(xy):
    pts = sorted(map(tuple, xy.tolist()))

    def half(seq):
        h = []
        for p in seq:
            while len(h) >= 2 and (h[-1][0] - h[-2][0]) * (p[1] - h[-2][1]) - (h[-1][1] - h[-2][1]) * (p[0] - h[-2][0]) <= 0:
                h.pop()
            h.append(p)
        return h[:-1]
    return half(pts) + half(pts[::-1])


class DelaunayNodeTests(unittest.TestCase):
    def points(self, n=300, seed=3):
        return run("scatter", run("grid", columns=3, rows=3, size_x=10.0, size_y=6.0), count=n, seed=seed)

    def test_triangles_face_the_axis(self):
        pts = self.points()
        for axis, nrm in (("z", 2), ("y", 1), ("x", 0)):
            cols = {"z": [0, 1, 2], "y": [0, 2, 1], "x": [2, 0, 1]}[axis]       # sheet in the plane normal to `axis`
            geo = Geometry.points_only(np.ascontiguousarray(pts.P[:, cols]))
            out = run("delaunay", geo, axis=axis)
            self.assertGreater(out.nprims, 500)
            vp = out.topology.vertex_point.reshape(-1, 3)
            a, b, c = (out.P[vp[:, i]].astype(float) for i in range(3))
            n = np.cross(b - a, c - a)
            self.assertTrue(np.all(n[:, nrm] > 0), axis)

    def test_keeps_points_and_attributes_and_cuts_long_edges(self):
        pts = self.points()
        pts = pts.copy()
        pts.point["w"] = np.arange(pts.npoints, dtype=np.float32)
        out = run("delaunay", pts)
        self.assertEqual(out.npoints, pts.npoints)
        np.testing.assert_array_equal(out.point["w"], pts.point["w"])
        cut = run("delaunay", pts, max_edge=0.6)
        self.assertLess(cut.nprims, out.nprims)
        vp = cut.topology.vertex_point.reshape(-1, 3)
        for i in range(3):
            self.assertLessEqual(np.linalg.norm(cut.P[vp[:, i]] - cut.P[vp[:, (i + 1) % 3]], axis=1).max(), 0.6 + 1e-6)

    def test_nothing_to_triangulate(self):
        out = run("delaunay", Geometry.points_only(np.zeros((2, 3), np.float32)))
        self.assertEqual((out.npoints, out.nprims), (2, 0))
        self.assertEqual(run("delaunay", Geometry.points_only(np.zeros((0, 3), np.float32))).nprims, 0)

    def test_primitives_are_replaced(self):
        out = run("delaunay", quad_geo())
        self.assertEqual(out.nprims, 2)                                       # a quad's four corners: two triangles


if __name__ == "__main__":
    unittest.main()
