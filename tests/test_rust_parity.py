"""The Rust core (``minihou_rs``, built from ``rust/``) against the Python engine, which stays the reference.

Skipped when the extension is not built. Topology and edge numbering must match exactly; floats within 1e-5 (float32 matrix
products and sums can round differently). Each comparison runs with the numpy path and, when numba is installed, the numba path
too, since the Python oracle itself takes either.
"""
import contextlib
import json
import unittest

import numpy as np

import minihou  # noqa: F401
from minihou import Evaluator, Geometry, Graph, accel, rust, topo
from minihou.sdk import REGISTRY

NODE_TYPES = ["grid", "box", "transform", "normal", "noise", "merge", "null", "output"]


def paths():
    """Context managers for the Python oracle's numpy path and, when available, its numba path."""
    out = [("numpy", accel.disabled)]
    if accel.enabled():
        out.append(("numba", contextlib.nullcontext))
    return out


def py_cook(graph: dict, node=None) -> Geometry:
    g = Graph.from_dict(graph)
    return Evaluator().cook(g, node or g.output_node())


def assert_same_geometry(tc, a: Geometry, b: Geometry, atol=1e-5):
    tc.assertEqual(a.npoints, b.npoints)
    np.testing.assert_array_equal(a.topology.vertex_point, b.topology.vertex_point)
    np.testing.assert_array_equal(a.topology.prim_offsets, b.topology.prim_offsets)
    np.testing.assert_array_equal(a.topology.closed_mask(), b.topology.closed_mask())
    for domain in ("point", "vertex", "prim"):
        ta, tb = a.table(domain), b.table(domain)
        tc.assertEqual(sorted(ta.names()), sorted(tb.names()), f"{domain} attributes")
        for name in ta.names():
            x, y = np.asarray(ta[name]), np.asarray(tb[name])
            tc.assertEqual(x.shape, y.shape, f"{domain} {name}")
            tc.assertEqual(x.dtype, y.dtype, f"{domain} {name}")
            np.testing.assert_allclose(x, y, rtol=atol, atol=atol, err_msg=f"{domain} {name}")
    tc.assertEqual(a.detail, b.detail)


def chain(*steps, source=None):
    """A graph: a source node and then each step reading the one before; the last is the display."""
    nodes = {"src": source or {"type": "grid", "params": {"columns": 7, "rows": 5}}}
    prev = "src"
    for i, (t, params) in enumerate(steps):
        nodes[f"n{i}"] = {"type": t, "inputs": {"geo": prev}, "params": params}
        prev = f"n{i}"
    return {"version": 1, "nodes": nodes, "display": prev}


@unittest.skipUnless(rust.available(), "the Rust core is not built (see rust/README.md)")
class RustParityTests(unittest.TestCase):
    def same(self, graph, node=None, atol=1e-5):
        b = rust.cook(graph, node)
        for name, ctx in paths():
            with self.subTest(oracle=name), ctx():
                assert_same_geometry(self, py_cook(graph, node), b, atol)

    # -- the contract ----------------------------------------------------------------------------------------------------
    def test_every_node_declares_the_python_parameters_and_defaults(self):
        self.assertEqual(sorted(rust.node_types()), sorted(NODE_TYPES))
        for t in NODE_TYPES:
            with self.subTest(node=t):
                py = json.loads(json.dumps(REGISTRY.get(t).coerce_params({})))
                rs = {k: v[0] for k, v in rust.node_params(t).items()}
                self.assertEqual(rs, py)

    def test_unsupported_settings_are_refused(self):
        for graph in (chain(("transform", {"group": "0-3"})),
                      chain(("transform", {"proportional_enabled": True})),
                      chain(("normal", {"domain": "vertex"})),
                      chain(("noise", {"group": "0-3"})),
                      {"version": 1, "nodes": {"b": {"type": "box", "params": {"add_vertex_normals": True}}}, "display": "b"},
                      {"version": 1, "nodes": {"w": {"type": "wrangle"}}, "display": "w"}):
            with self.subTest(graph=json.dumps(graph)[:90]), self.assertRaises(RuntimeError) as cm:
                rust.cook(graph)
            self.assertRegex(str(cm.exception), "not (supported by|in) the Rust backend")

    # -- nodes -----------------------------------------------------------------------------------------------------------
    def test_grid(self):
        self.same(chain())
        self.same(chain(source={"type": "grid", "params": {"size_x": 3.5, "size_y": 2, "columns": 31, "rows": 17,
                                                           "center": [1, -2, 0.5], "rotate": [10, 25, -40], "scale": [1, 2, 0.5],
                                                           "uniform_scale": 1.5}}))

    def test_box(self):
        for params in ({}, {"divisions": 3}, {"axis_divisions": [2, 3, 4], "size": [2, 1, 0.5], "center": [1, 2, 3]},
                       {"connectivity": "triangles", "divisions": 2}, {"consolidate": False, "rotate": [30, 0, 45]},
                       {"primitive_type": "points", "divisions": 2}, {"uniform_scale": 3.0}):
            with self.subTest(params=params):
                self.same({"version": 1, "nodes": {"b": {"type": "box", "params": params}}, "display": "b"})

    def test_transform(self):
        for params in ({"translate": [1, 2, 3]}, {"rotate": [30, -45, 60], "scale": [2, 0.5, 1], "pivot": [0.3, -1, 2]},
                       {"axis_rotation": [0, 0, 45], "scale": [2, 1, 1], "uniform_scale": 0.5, "translate": [1, 0, 0]},
                       {"scale": [0, 1, 1]}):
            with self.subTest(params=params):
                self.same(chain(("transform", params)))
                self.same(chain(("normal", {}), ("transform", params)))           # N is transformed too

    def test_normal(self):
        for domain in ("point", "prim"):
            with self.subTest(domain=domain):
                self.same(chain(("noise", {"amplitude": [0.3, 0.2, 1], "frequency": 2}), ("normal", {"domain": domain})))
                self.same({"version": 1, "nodes": {"b": {"type": "box", "params": {"divisions": 3}},
                                                   "n": {"type": "normal", "inputs": {"geo": "b"}, "params": {"domain": domain}}},
                           "display": "n"})

    def test_noise(self):
        for params in ({}, {"amplitude": [0.5, 0.2, 1.5], "frequency": 3.3, "octaves": 4, "seed": 7},
                       {"mode": "vector", "offset": [0.1, 2, -3], "lacunarity": 2.5, "gain": 0.7, "octaves": 3},
                       {"attrib": "h", "seed": -5}, {"attrib": "w", "mode": "vector", "seed": 123456789}):
            with self.subTest(params=params):
                self.same(chain(("noise", params), source={"type": "grid", "params": {"columns": 40, "rows": 33,
                                                                                         "size_x": 7, "size_y": 5}}))

    def test_merge_null_output_and_bypass(self):
        graph = {"version": 1, "nodes": {
            "g": {"type": "grid", "params": {"columns": 4, "rows": 3}},
            "b": {"type": "box", "params": {"divisions": 2}},
            "n": {"type": "normal", "inputs": {"geo": "b"}},
            "t": {"type": "transform", "inputs": {"geo": "g"}, "params": {"translate": [0, 0, 2]}, "flags": {"bypass": True}},
            "m": {"type": "merge", "inputs": {"geos": ["g", "n", "t"]}},
            "z": {"type": "null", "inputs": {"geo": "m"}},
            "out": {"type": "output", "inputs": {"geo": "z"}}}}
        self.same(graph)
        self.same(graph, "n")

    def test_seeded_random_parameters(self):
        """A few random settings per node, through both engines (not just the hand-picked ones)."""
        rng = np.random.default_rng(11)
        v3 = lambda lo, hi: [float(x) for x in rng.uniform(lo, hi, 3)]                     # noqa: E731
        for _ in range(6):
            src = {"type": "grid", "params": {"columns": int(rng.integers(2, 40)), "rows": int(rng.integers(2, 40)),
                                              "size_x": float(rng.uniform(0.1, 20)), "size_y": float(rng.uniform(0.1, 20)),
                                              "rotate": v3(-180, 180), "center": v3(-5, 5)}}
            steps = [("noise", {"amplitude": v3(-1, 1), "frequency": float(rng.uniform(0.1, 5)), "octaves": int(rng.integers(1, 6)),
                                "seed": int(rng.integers(-1000, 1000)), "mode": str(rng.choice(["scalar", "vector"]))}),
                     ("transform", {"translate": v3(-3, 3), "rotate": v3(-180, 180), "scale": v3(0.2, 3), "pivot": v3(-1, 1)}),
                     ("normal", {"domain": str(rng.choice(["point", "prim"]))})]
            with self.subTest(source=src["params"]):
                # positions agree to 1-2 float32 ulps after noise and transform; normals of small faces far from the origin
                # magnify that, hence the looser tolerance here
                self.same(chain(*steps, source=src), atol=1e-4)

    # -- kernels ---------------------------------------------------------------------------------------------------------
    def test_edges_match_both_python_paths_exactly(self):
        cases = [py_cook(chain(source={"type": "grid", "params": {"columns": 9, "rows": 6}})),
                 py_cook(chain(source={"type": "grid", "params": {"columns": 150, "rows": 150}})),       # > 20k vertices: numba path
                 py_cook({"version": 1, "nodes": {"b": {"type": "box", "params": {"connectivity": "triangles", "divisions": 5}}},
                          "display": "b"})]
        # open curves, a closed one-vertex primitive, an empty primitive and an unused point
        curves = Geometry.from_arrays(np.zeros((6, 3), np.float32), np.array([0, 1, 2, 3, 3, 4], np.int32),
                                      np.array([0, 3, 4, 4, 6], np.int32), np.array([False, True, True, False]))
        cases.append(curves)
        for g in cases:
            r = rust.edges(g)
            for name, ctx in paths():
                with self.subTest(n=g.nvertices, oracle=name), ctx():
                    e = topo.edges(g)
                    np.testing.assert_array_equal(r.points, e.points)
                    np.testing.assert_array_equal(r.of_vertex, e.of_vertex)
                    np.testing.assert_array_equal(r.count, e.count)
                    self.assertEqual(r.points.dtype, e.points.dtype)

    def test_normal_and_noise_kernels(self):
        from minihou.nodes.normal import prim_normals
        from minihou.nodes._util import fbm3_numpy
        g = py_cook(chain(("noise", {"amplitude": [0.2, 0.3, 1]}), source={"type": "grid", "params": {"columns": 60, "rows": 50}}))
        np.testing.assert_allclose(rust.prim_normals(g), prim_normals(g), rtol=1e-9, atol=1e-12)
        pos = np.random.default_rng(3).uniform(-50, 50, (5000, 3)).astype(np.float32)
        np.testing.assert_array_equal(rust.fbm3(pos, 5, 3, 2.0, 0.5), fbm3_numpy(pos, 5, 3, 2.0, 0.5))   # bit for bit

    def test_bad_input_is_an_error_not_a_crash(self):
        import minihou_rs
        with self.assertRaises(ValueError):
            minihou_rs.edges(np.array([0, 1, 9], np.int32), np.array([0, 3], np.int32), None, 3)
        with self.assertRaises(ValueError):
            minihou_rs.edges(np.array([0, 1, 2], np.int32), np.array([0, 2], np.int32), None, 3)
        with self.assertRaises(RuntimeError):
            rust.cook({"version": 1, "nodes": {"t": {"type": "transform"}}, "display": "t"})


class OptionalTests(unittest.TestCase):
    def test_minihou_imports_and_reports_without_the_extension(self):
        self.assertIsInstance(rust.available(), bool)
        if not rust.available():
            with self.assertRaises(RuntimeError):
                rust.node_types()


if __name__ == "__main__":
    unittest.main()
