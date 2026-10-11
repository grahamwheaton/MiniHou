"""uv_flatten: conformal islands (exact on developable shapes), disc-shaped auto cuts, no flips, no overlaps, 0-1."""
import os
import unittest
from unittest import mock

import numpy as np

from minihou import Evaluator, Graph, NodeError
from minihou import lscm
from minihou.nodes._util import fan_triangulate
from minihou.nodes.bevel import twins


def cook(nodes, display):
    nodes = dict(nodes)
    nodes["out"] = {"type": "output", "inputs": {"geo": display}}
    g = Evaluator().cook(Graph.from_dict({"nodes": nodes, "display": "out"}))
    g.validate()
    return g


def flatten(src, **params):
    return cook({"s": src, "u": {"type": "uv_flatten", "inputs": {"geo": "s"}, "params": params}}, "u")


def angles(X):
    """Corner angles (T, 3) of triangles X (T, 3, d)."""
    out = []
    for i in range(3):
        a, b = X[:, (i + 1) % 3] - X[:, i], X[:, (i + 2) % 3] - X[:, i]
        c = np.einsum("ij,ij->i", a, b) / np.maximum(np.linalg.norm(a, axis=1) * np.linalg.norm(b, axis=1), 1e-30)
        out.append(np.arccos(np.clip(c, -1, 1)))
    return np.stack(out, 1)


def check(test, g, max_mean_angle_err):
    uv = g.vertex["uv"].astype(np.float64)
    test.assertTrue(uv.min() >= 0.0 and uv.max() <= 1.0)
    tris, _ = fan_triangulate(g.topology)
    U, T = uv[tris], g.P[g.topology.vertex_point[tris]].astype(np.float64)
    d1, d2 = U[:, 1] - U[:, 0], U[:, 2] - U[:, 0]
    test.assertTrue(np.all(d1[:, 0] * d2[:, 1] - d1[:, 1] * d2[:, 0] > 0), "no flipped triangles")
    err = np.degrees(np.abs(angles(U) - angles(T)))
    test.assertLess(err.mean(), max_mean_angle_err)
    # islands are discs and their boxes do not overlap
    isl = g.prim["island"]
    k = int(isl.max()) + 1
    test.assertTrue(np.all(lscm.euler(g, isl, k) == 1))
    iv = isl[np.repeat(np.arange(g.nprims), g.topology.prim_sizes())]
    lo = np.array([uv[iv == i].min(0) for i in range(k)])
    hi = np.array([uv[iv == i].max(0) for i in range(k)])
    for i in range(k):
        for j in range(i + 1, k):
            test.assertFalse(np.all(lo[i] < hi[j] - 1e-9) and np.all(lo[j] < hi[i] - 1e-9), f"islands {i}, {j}")
    return k


class UVFlattenTests(unittest.TestCase):
    def test_flat_and_developable_shapes_are_exact(self):
        g = flatten({"type": "grid", "params": {"size_x": 4, "size_y": 2}})
        self.assertEqual(check(self, g, 1e-3), 1)
        uv = g.vertex["uv"]
        ext = np.ptp(uv, 0)
        self.assertAlmostEqual(max(ext) / min(ext), 2.0, places=3)                 # the 4 x 2 rectangle
        box = flatten({"type": "box", "params": {"divisions": 2}})
        self.assertEqual(check(self, box, 1e-3), 6)                                 # one island per side
        tube = flatten({"type": "tube", "params": {"rows": 3, "caps": False}})
        self.assertEqual(check(self, tube, 1e-3), 2)                                # cut into two half-pipes

    def test_closed_surfaces_are_cut_into_discs(self):
        self.assertEqual(check(self, flatten({"type": "sphere"}), 3.0), 2)
        self.assertEqual(check(self, flatten({"type": "torus"}), 3.0), 4)
        box = flatten({"type": "box", "params": {"divisions": 3}}, seam_angle=180)  # no angle seams: cut anyway
        check(self, box, 15.0)

    def test_islands_keep_relative_size(self):
        g = cook({"a": {"type": "grid", "params": {"size_x": 1, "size_y": 1}},
                  "b": {"type": "grid", "params": {"size_x": 2, "size_y": 2, "center": [5, 0, 0]}},
                  "m": {"type": "merge", "inputs": {"geos": ["a", "b"]}},
                  "u": {"type": "uv_flatten", "inputs": {"geo": "m"}}}, "u")
        iv = g.prim["island"][np.repeat(np.arange(g.nprims), g.topology.prim_sizes())]
        e0, e1 = np.ptp(g.vertex["uv"][iv == 0], 0), np.ptp(g.vertex["uv"][iv == 1], 0)
        self.assertAlmostEqual(e1[0] / e0[0], 2.0, places=3)

    def test_split_attribute_and_errors(self):
        g = cook({"g": {"type": "grid"},
                  "w": {"type": "wrangle", "inputs": {"geo": "g"},
                        "params": {"snippet": "i@side = @P.x > 0", "run_over": "primitives"}},
                  "u": {"type": "uv_flatten", "inputs": {"geo": "w"}, "params": {"split_attrib": "side"}}}, "u")
        self.assertEqual(check(self, g, 1e-3), 2)
        with self.assertRaises(NodeError):
            cook({"g": {"type": "grid"}, "u": {"type": "uv_flatten", "inputs": {"geo": "g"},
                                                "params": {"split_attrib": "nope"}}}, "u")
        with self.assertRaises(NodeError):
            cook({"c": {"type": "helix"}, "u": {"type": "uv_flatten", "inputs": {"geo": "c"}}}, "u")

    def test_numpy_solver_matches(self):
        src = {"type": "sphere", "params": {"rows": 12, "columns": 24}}
        ref = flatten(src)
        with mock.patch.dict(os.environ, {"MINIHOU_NO_SCIPY": "1"}):
            alt = flatten(src)
        np.testing.assert_allclose(alt.vertex["uv"], ref.vertex["uv"], atol=2e-3)
        self.assertTrue(np.all(twins(alt) == twins(ref)))


if __name__ == "__main__":
    unittest.main()
