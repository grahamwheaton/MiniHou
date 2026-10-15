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
from minihou import Evaluator, Geometry, Graph, accel, proxy, rust, topo
from minihou.geometry import Topology
from minihou.sdk import REGISTRY

NODE_TYPES = ["grid", "box", "point", "curve", "line", "circle", "bound", "transform", "normal", "noise", "merge", "null", "output"]


@contextlib.contextmanager
def _numpy_only():
    with rust.disabled(), accel.disabled():
        yield


def paths():
    """Context managers for the Python oracle's numpy path and, when available, its numba path (Rust off in both: the engine
    itself uses Rust when it is installed)."""
    out = [("numpy", _numpy_only)]
    if accel.enabled():
        out.append(("numba", rust.disabled))
    return out


def py_cook(graph: dict, node=None) -> Geometry:
    g = Graph.from_dict(graph)
    with rust.disabled():
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
        for graph in (chain(("transform", {"proportional_enabled": True, "proportional_falloff": "random"})),
                      chain(("normal", {"domain": "vertex"})),
                      chain(("noise", {"group": "0-3"})),
                      chain(("bound", {"oriented": True})),
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

    def test_bound(self):
        sources = ({"type": "grid", "params": {"columns": 7, "rows": 5}},
                   {"type": "grid", "params": {"columns": 4, "rows": 3, "size_x": 3, "size_y": 2, "center": [1, -2, 0.5],
                                               "rotate": [10, 25, -40]}},
                   {"type": "box", "params": {"divisions": 3, "size": [2, 1, 0.5], "center": [1, 2, 3]}},
                   {"type": "curve", "params": {"points": "0,0,0 1,2,3 2,1,0", "closed": True}},
                   {"type": "point", "params": {"center": [1.25, -2, 3]}})
        for source in sources:
            for padding in ([0, 0, 0], [0.1, 0.2, 0.3]):
                with self.subTest(source=source, padding=padding):
                    self.same(chain(("bound", {"padding": padding}), source=source))
        self.same(chain(("noise", {"amplitude": [0.3, 0.4, 0.5], "frequency": 2}),
                        ("bound", {"padding": [0.5, 0, 0.25]})))
        # empty input: a plain unit box of one welded point, with no detail attributes
        empty = {"version": 1, "nodes": {"g": {"type": "grid", "flags": {"bypass": True}},
                                         "b": {"type": "bound", "inputs": {"geo": "g"}}}, "display": "b"}
        self.same(empty)

    def test_curve_line_circle(self):
        for source in ({"type": "point"}, {"type": "point", "params": {"center": [1.25, -2, 3]}},
                       {"type": "curve"}, {"type": "curve", "params": {"points": "0,0,0 1,2,3 2,1,0", "closed": True}},
                       {"type": "line"}, {"type": "line", "params": {"origin": [1,2,3], "direction": [1,2,3], "points": 7}},
                       {"type": "circle"}, {"type": "circle", "params": {"divisions": 7, "rotate": [15,30,45], "center": [1,2,3]}}):
            with self.subTest(source=source):
                self.same(chain(source=source))
        self.same(chain(("curve", {"points": "0,0,0 1,0,0"})))

    def test_transform(self):
        for params in ({"translate": [1, 2, 3]}, {"rotate": [30, -45, 60], "scale": [2, 0.5, 1], "pivot": [0.3, -1, 2]},
                       {"axis_rotation": [0, 0, 45], "scale": [2, 1, 1], "uniform_scale": 0.5, "translate": [1, 0, 0]},
                       {"scale": [0, 1, 1]}):
            with self.subTest(params=params):
                self.same(chain(("transform", params)))
                self.same(chain(("normal", {}), ("transform", params)))           # N is transformed too

    def test_transform_group_and_proportional(self):
        for params in ({"group": "0-3", "translate": [0,0,2]},
                       {"group": "^0-3", "translate": [1,0,0]},
                       {"group": "@P.x > 0", "translate": [0,1,0]},
                       {"group": "3", "proportional_enabled": True, "proportional_radius": 3,
                        "proportional_falloff": "smooth", "translate": [0,0,2]},
                       {"group": "3", "proportional_enabled": True, "proportional_radius": 3,
                        "proportional_falloff": "root", "proportional_connected": True, "rotate": [0,0,45]},
                       {"group": "3", "proportional_enabled": True, "proportional_radius": 3,
                        "proportional_projected": True, "translate": [0,0,2]}):
            with self.subTest(params=params):
                self.same(chain(("transform", params), source={"type": "line", "params": {"points": 6, "length": 5,
                                                                                    "direction": [1,0,0]}}))
        for falloff in ("smooth", "sphere", "root", "sharp", "linear", "constant"):
            with self.subTest(falloff=falloff):
                params = {"group": "3", "proportional_enabled": True, "proportional_radius": 3,
                          "proportional_falloff": falloff, "translate": [0.2, 0.3, 1],
                          "rotate": [10, 25, 45], "scale": [1.2, 0.8, 1.5]}
                self.same(chain(("normal", {}), ("transform", params)))

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
            # the viewport's counts (fan triangles, open-curve vertices, loose points); a writeable copy is not cached
            t = g.topology
            with rust.disabled():
                expect = proxy.draw_stats(Topology(t.vertex_point.copy(), t.prim_offsets.copy(), t.closed), g.npoints)
            self.assertEqual(rust.draw_stats(t, g.npoints), expect)

    def test_normal_and_noise_kernels(self):
        from minihou.nodes.normal import prim_normals
        from minihou.nodes._util import fbm3_numpy
        g = py_cook(chain(("noise", {"amplitude": [0.2, 0.3, 1]}), source={"type": "grid", "params": {"columns": 60, "rows": 50}}))
        with rust.disabled():
            np.testing.assert_allclose(rust.prim_normals(g), prim_normals(g), rtol=1e-9, atol=1e-12)
        pos = np.random.default_rng(3).uniform(-50, 50, (5000, 3)).astype(np.float32)
        np.testing.assert_array_equal(rust.fbm3(pos, 5, 3, 2.0, 0.5), fbm3_numpy(pos, 5, 3, 2.0, 0.5))   # bit for bit

    def test_viewport_layout_is_identical(self):
        """The editor's arrays for big meshes (fan triangles in Morton order, run table, edges) are exactly the Python / numba ones."""
        from minihou import preview
        big = py_cook(chain(("noise", {"amplitude": [0.3, 0.3, 2], "frequency": 0.7}),
                            source={"type": "grid", "params": {"columns": 420, "rows": 300, "size_x": 30, "size_y": 20}}))
        with rust.disabled():                                             # the reduced view below really takes the Rust branch
            red = preview.proxy.decimate(big, 150000, 300000)
        self.assertTrue(red.reduced and red.geo.nprims > preview.EXACT_PRIMS, (red.reduced, red.geo.nprims))
        tris = py_cook({"version": 1, "nodes": {"b": {"type": "box", "params": {"connectivity": "triangles", "divisions": 140}}},
                        "display": "b"})
        # quads, triangles and an open curve together, past the exact-triangulation size
        mixed = py_cook({"version": 1, "nodes": {"a": {"type": "grid", "params": {"columns": 360, "rows": 300}}, "b": {"type": "box",
                         "params": {"connectivity": "triangles", "divisions": 30}},
                         "m": {"type": "merge", "inputs": {"geos": ["a", "b"]}}}, "display": "m"})
        curve = Geometry.from_arrays(np.random.default_rng(1).random((5, 3)).astype(np.float32), np.arange(5, dtype=np.int32),
                                     np.array([0, 5], np.int32), np.array([False]))
        from minihou.nodes.merge import merge
        mixed = merge([mixed, curve])
        for geo in (big, tris, mixed):
            # full resolution; a reduced (clustered) view that stays above preview.EXACT_PRIMS, so the Rust layout maps the
            # clustered primitives back (red.prim_src); and one small enough for the Python exact triangulation
            for full, budget in ((True, 0), (False, 150000 if geo is big else 0), (False, 20000)):
                if not full and not budget:
                    continue
                preview._LAYOUTS.clear()
                h_rs, a_rs = preview._arrays_for(geo, full=full, tris=budget)
                for name, ctx in paths():
                    with self.subTest(prims=geo.nprims, full=full, oracle=name), ctx():
                        preview._LAYOUTS.clear()
                        h_py, a_py = preview._arrays_for(geo, full=full, tris=budget)
                        self.assertEqual(h_rs, h_py)
                        self.assertEqual(sorted(a_rs), sorted(a_py))
                        for k in a_py:
                            self.assertEqual(a_rs[k].dtype, a_py[k].dtype, k)
                        self.same_layout(a_rs, a_py, exact_order=name == "numba")
            # a deformer downstream (same topology): only the run bounds are redone
            moved = geo.copy()
            moved.point["P"] = (geo.P * np.float32(1.5)).astype(np.float32)
            preview._LAYOUTS.clear()
            preview._arrays_for(geo, full=True)
            _, a_rs = preview._arrays_for(moved, full=True)
            for name, ctx in paths():
                with self.subTest(moved=True, oracle=name), ctx():
                    preview._LAYOUTS.clear()
                    preview._arrays_for(geo, full=True)
                    _, a_py = preview._arrays_for(moved, full=True)
                    self.same_layout(a_rs, a_py, exact_order=name == "numba")

    def same_layout(self, a, b, exact_order):
        """The numba path sorts triangles by the same Morton code as Rust, so the order and run table match exactly; the numpy
        fallback quantises triangle centres a little differently, so against it the triangles must be the same set."""
        if exact_order or "chunks" not in b:
            for k in b:
                np.testing.assert_array_equal(a[k], b[k], err_msg=k)
            return
        for k in b:
            if k not in ("tri", "tri_prim", "chunks"):
                np.testing.assert_array_equal(a[k], b[k], err_msg=k)
        rows = lambda d: np.concatenate([d["tri"].reshape(-1, 3), d["tri_prim"][:, None]], 1)                     # noqa: E731
        key = lambda r: r[np.lexsort(r.T[::-1])]                                                                   # noqa: E731
        np.testing.assert_array_equal(key(rows(a)), key(rows(b)))
        self.assertEqual(a["chunks"].shape, b["chunks"].shape)

    def test_the_engine_with_rust_on_matches_the_engine_with_it_off(self):
        """The Python engine uses the Rust kernels for grid, box, noise, transform, normal and edges when the extension is
        installed: whole graphs (with nodes Rust does not have in between) must come out the same as without it."""
        graphs = [chain(("noise", {"amplitude": [0.4, 0.2, 1.5], "octaves": 3, "seed": 9}), ("transform", {"rotate": [20, 40, 60],
                         "scale": [2, 1, 0.5], "pivot": [1, 0, 0]}), ("normal", {}), ("transform", {"translate": [0, 0, 1]}),
                        source={"type": "grid", "params": {"columns": 120, "rows": 90, "size_x": 8}}),
                  chain(("noise", {"mode": "vector", "attrib": "v", "frequency": 2}), ("normal", {"domain": "prim"}),
                        ("subdivide", {}), ("normal", {}), source={"type": "box", "params": {"divisions": 6}}),
                  chain(("polyextrude", {"distance": 0.3}), ("noise", {"attrib": "P", "amplitude": [0.1, 0.1, 0.1]}),
                        ("transform", {"axis_rotation": [0, 30, 0], "scale": [1, 2, 1]}), ("normal", {}),
                        source={"type": "grid", "params": {"columns": 30, "rows": 20}})]
        for graph in graphs:
            with self.subTest(graph=list(graph["nodes"])):
                g = Graph.from_dict(graph)
                with_rust = Evaluator().cook(g, g.output_node())
                with rust.disabled():
                    without = Evaluator().cook(g, g.output_node())
                assert_same_geometry(self, with_rust, without, atol=1e-5)

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
