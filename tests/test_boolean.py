"""boolean: exact volumes on boxes (offset, flush, nested, apart), identities on curved shapes, always closed and
manifold; attributes carried across cuts."""
import unittest

import numpy as np

from minihou import Evaluator, Graph, NodeError
from minihou.nodes._util import fan_triangulate
from minihou.nodes.bevel import twins


def cook(nodes, display):
    nodes = dict(nodes)
    nodes["out"] = {"type": "output", "inputs": {"geo": display}}
    g = Evaluator().cook(Graph.from_dict({"nodes": nodes, "display": "out"}))
    g.validate()
    return g


def volume(g):
    tris, _ = fan_triangulate(g.topology)
    T = g.P[g.topology.vertex_point[tris]].astype(np.float64)
    return float(np.einsum("ij,ij->i", T[:, 0], np.cross(T[:, 1], T[:, 2])).sum() / 6.0)


def crossings(g):
    """Number of polygons whose outline crosses itself (in the polygon's own plane)."""
    t, P, bad = g.topology, g.P.astype(np.float64), 0
    offs = t.prim_offsets
    for q in range(t.nprims):
        Q = P[t.vertex_point[offs[q]:offs[q + 1]]]
        if len(Q) < 4:
            continue
        n = np.cross(Q - Q.mean(0), np.roll(Q, -1, 0) - Q.mean(0)).sum(0)
        n /= max(np.linalg.norm(n), 1e-300)
        u = np.cross(n, [1.0, 0, 0] if abs(n[0]) < 0.9 else [0, 1.0, 0])
        u /= np.linalg.norm(u)
        xy = np.stack([Q @ u, Q @ np.cross(n, u)], 1)
        a, b = xy, np.roll(xy, -1, 0)
        for i in range(len(xy)):
            for j in range(i + 2, len(xy) - (1 if i == 0 else 0)):
                cr = lambda p_, q_, r_: (q_[0] - p_[0]) * (r_[1] - p_[1]) - (q_[1] - p_[1]) * (r_[0] - p_[0])  # noqa: E731
                ei, ej = b[i] - a[i], b[j] - a[j]
                sin = abs(ei[0] * ej[1] - ei[1] * ej[0]) / max(np.linalg.norm(ei) * np.linalg.norm(ej), 1e-300)
                if sin > 1e-3 and cr(a[i], b[i], a[j]) * cr(a[i], b[i], b[j]) < 0 and \
                        cr(a[j], b[j], a[i]) * cr(a[j], b[j], b[i]) < 0:
                    bad += 1                                # a real crossing (not a zero-width fold)
    return bad


def boolean(a, b, op, extra=None):
    nodes = {"a": a, "b": b, "x": {"type": "boolean", "inputs": {"a": "a", "b": "b"}, "params": {"operation": op}}}
    nodes.update(extra or {})
    return cook(nodes, "x")


def box(size, center=(0, 0, 0), divisions=1):
    return {"type": "box", "params": {"size": list(size), "center": list(center), "divisions": divisions}}


class BooleanTests(unittest.TestCase):
    def closed(self, g):
        self.assertTrue(np.all(twins(g) >= 0), "closed, manifold, consistently wound")
        self.assertEqual(crossings(g), 0, "every polygon is simple")

    def test_boxes(self):
        cases = [  # A, B, expected volumes, a triangle count that shows no slivers were left
            (box([2, 2, 2]), box([2, 2, 2], [1, 1, 1]), {"union": 15, "subtract": 7, "intersect": 1}),
            (box([2, 2, 2]), box([1, 1, 2], [0.5, 0.5, 0]), {"union": 8, "subtract": 6, "intersect": 2}),   # flush
            (box([2, 2, 2]), box([1, 1, 1]), {"union": 8, "subtract": 7, "intersect": 1}),                 # nested
            (box([1, 1, 1]), box([1, 1, 1], [3, 0, 0]), {"union": 2, "subtract": 1, "intersect": 0}),       # apart
        ]
        for a, b, expect in cases:
            for op, v in expect.items():
                with self.subTest(b=b["params"], op=op):
                    g = boolean(a, b, op)
                    self.assertAlmostEqual(volume(g), v, delta=1e-4)
                    self.closed(g)
        cube = boolean(box([2, 2, 2]), box([2, 2, 2], [1, 1, 1]), "intersect")
        self.assertEqual(cube.nprims, 6)                                    # exactly the unit cube: six quads

    def test_curved_identities(self):
        a = box([2, 2, 2], divisions=2)
        for b in ({"type": "sphere", "params": {"radius": 1.2, "rows": 16, "columns": 32}},
                  {"type": "tube", "params": {"radius_bottom": 0.1, "radius_top": 0.1, "height": 3, "columns": 12,
                                              "center": [0.6, -0.4, 0]}},          # a hole inside single triangles
                  {"type": "torus", "params": {"radius": 1.0, "thickness": 0.3, "center": [0.3, 0.2, 0.9]}}):
            with self.subTest(b=b["type"]):
                va, vb = volume(cook({"a": a}, "a")), volume(cook({"b": b}, "b"))
                u, i, s = (boolean(a, b, op) for op in ("union", "intersect", "subtract"))
                for g in (u, i, s):
                    self.closed(g)
                self.assertAlmostEqual(volume(u) + volume(i), va + vb, delta=1e-4)
                self.assertAlmostEqual(volume(s), va - volume(i), delta=1e-4)
                self.assertTrue(volume(i) > 0)

    def test_random_shapes(self):
        rng = np.random.default_rng(5)
        for trial in range(6):
            nodes = {"s": {"type": "sphere", "params": {"rows": 10, "columns": 20}},
                     "n": {"type": "noise", "inputs": {"geo": "s"}, "params": {"amplitude": [0.15] * 3, "seed": trial}},
                     "t": {"type": "torus", "params": {"rows": 8, "columns": 20}},
                     "m": {"type": "transform", "inputs": {"geo": "t"},
                           "params": {"rotate": rng.uniform(0, 360, 3).tolist(), "translate": rng.uniform(-.8, .8, 3).tolist()}}}
            va, vb = volume(cook(nodes, "n")), volume(cook(nodes, "m"))
            res = {}
            for op in ("union", "intersect", "subtract"):
                nodes["x"] = {"type": "boolean", "inputs": {"a": "n", "b": "m"}, "params": {"operation": op}}
                g = cook(nodes, "x")
                self.closed(g)
                res[op] = volume(g)
            self.assertAlmostEqual(res["union"] + res["intersect"], va + vb, delta=1e-4)
            self.assertAlmostEqual(res["subtract"], va - res["intersect"], delta=1e-4)

    def test_attributes_follow(self):
        a = {"type": "box", "params": {"size": [2, 2, 2]}}
        extra = {"u": {"type": "uv_project", "inputs": {"geo": "a"}, "params": {"method": "box"}},
                 "w": {"type": "wrangle", "inputs": {"geo": "u"},
                       "params": {"snippet": "i@side = @primnum", "run_over": "primitives"}}}
        nodes = {"a": a, **extra, "b": {"type": "sphere", "params": {"radius": 1.2}},
                 "x": {"type": "boolean", "inputs": {"a": "w", "b": "b"}, "params": {"operation": "subtract"}},
                 "again": {"type": "uv_project", "inputs": {"geo": "x"}, "params": {"method": "box"}}}
        g, ref = cook(nodes, "x"), cook(nodes, "again")
        fa = g.prim["group_from_a"].astype(bool)[np.repeat(np.arange(g.nprims), g.topology.prim_sizes())]
        np.testing.assert_allclose(g.vertex["uv"][fa], ref.vertex["uv"][fa], atol=1e-5)   # interpolated exactly
        self.assertTrue(np.all(g.prim["side"][g.prim["group_from_b"] == 1] == 0))           # zero-filled for B
        self.assertEqual(set(np.unique(g.prim["side"][g.prim["group_from_a"] == 1])), set(range(6)))

    def test_holes_and_chains_in_one_face(self):
        """A plate with three bores and a slot, then a dome: faces with several holes and cuts."""
        import json
        import pathlib
        path = pathlib.Path(__file__).resolve().parent.parent / "examples" / "graphs" / "boolean_part.json"
        graph = json.loads(path.read_text(encoding="utf-8"))
        for node in ("cut", "joined"):
            g = Evaluator().cook(Graph.from_dict(graph), node)
            self.closed(g)

    def test_coplanar_ring(self):
        """A sphere whose equator ring lies exactly in a box face: still closed, manifold and exact in volume."""
        a = box([2, 2, 2])
        b = {"type": "sphere", "params": {"radius": 0.6, "rows": 16, "columns": 32, "center": [0.2, 0.1, 1.0]}}
        va, vb = volume(cook({"a": a}, "a")), volume(cook({"b": b}, "b"))
        u, i = boolean(a, b, "union"), boolean(a, b, "intersect")
        for g in (u, i):
            self.assertTrue(np.all(twins(g) >= 0))
        self.assertAlmostEqual(volume(u) + volume(i), va + vb, delta=1e-4)
        self.assertAlmostEqual(volume(i), vb / 2, delta=1e-3)

    def test_errors(self):
        with self.assertRaises(NodeError):
            boolean({"type": "helix"}, box([1, 1, 1]), "union")


if __name__ == "__main__":
    unittest.main()
