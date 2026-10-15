"""Seeded graph fuzzing and small degenerate cases for the optional Rust core."""
import json
import os
import subprocess
import sys
import unittest

import numpy as np

from minihou import Evaluator, Graph, rust
from tests.test_rust_parity import assert_same_geometry, chain, paths, py_cook


def _graph(nodes, display):
    return {"version": 1, "nodes": nodes, "display": display}


def _v3(rng, lo, hi):
    return [float(x) for x in rng.uniform(lo, hi, 3)]


def _random_graph(seed):
    rng = np.random.default_rng(seed)
    nodes = {}
    leaves = []
    for i in range(int(rng.integers(2, 4))):
        name = f"s{i}"
        source_kind = int(rng.integers(8))
        if source_kind == 0:
            params = {"columns": int(rng.integers(2, 9)), "rows": int(rng.integers(2, 9)),
                      "size_x": float(rng.uniform(0.5, 5)), "size_y": float(rng.uniform(0.5, 5))}
            kind = "grid"
        elif source_kind == 1:
            params = {"divisions": int(rng.integers(1, 4)), "size": _v3(rng, 0.5, 3),
                      "connectivity": str(rng.choice(["triangles", "quadrilaterals"]))}
            kind = "box"
        elif source_kind == 2:
            params = {"points": "0,0,0 1,0,0 1,1,0", "closed": bool(rng.integers(2))}
            kind = "curve"
        elif source_kind == 3:
            params = {"points": int(rng.integers(2, 12)), "direction": _v3(rng, -1, 1)}
            kind = "line"
        elif source_kind == 4:
            params = {"divisions": int(rng.integers(3, 20)), "radius": float(rng.uniform(0.1, 3))}
            kind = "circle"
        elif source_kind == 5:
            params = {"rows": int(rng.integers(2, 8)), "columns": int(rng.integers(3, 10)),
                      "radius": float(rng.uniform(0.2, 3))}
            kind = "sphere"
        elif source_kind == 6:
            params = {"rows": int(rng.integers(1, 6)), "columns": int(rng.integers(3, 10)),
                      "radius_bottom": float(rng.uniform(0.0, 2.0)), "radius_top": float(rng.uniform(0.0, 2.0)),
                      "height": float(rng.uniform(0.5, 3.0)), "caps": bool(rng.integers(2))}
            kind = "tube"
        else:
            params = {"rows": int(rng.integers(3, 8)), "columns": int(rng.integers(3, 10)),
                      "radius": float(rng.uniform(1.0, 3.0)), "thickness": float(rng.uniform(0.1, 0.8))}
            kind = "torus"
        nodes[name] = {"type": kind, "params": params}
        prev = name
        for j in range(int(rng.integers(1, 4))):
            name = f"b{i}_{j}"
            kind = str(rng.choice(["transform", "noise", "normal", "bound", "null"]))
            if kind == "transform":
                params = {"translate": _v3(rng, -2, 2), "rotate": _v3(rng, -45, 45),
                          "scale": _v3(rng, 0.6, 1.5)}
                if rng.integers(3) == 0:
                    params.update({"group": "0", "proportional_enabled": True,
                                   "proportional_radius": float(rng.uniform(0.5, 3)),
                                   "proportional_falloff": str(rng.choice(["smooth", "sphere", "root", "sharp", "linear", "constant"])),
                                   "proportional_connected": bool(rng.integers(2))})
            elif kind == "noise":
                params = {"amplitude": _v3(rng, 0.05, 0.3), "frequency": float(rng.uniform(0.3, 2)),
                          "octaves": int(rng.integers(1, 5)), "seed": int(rng.integers(-100, 100)),
                          "mode": str(rng.choice(["scalar", "vector"]))}
            elif kind == "normal":
                params = {"domain": str(rng.choice(["point", "prim"]))}
            elif kind == "bound":
                params = {"padding": _v3(rng, 0, 0.5)}
            else:
                params = {}
            nodes[name] = {"type": kind, "params": params, "inputs": {"geo": prev}}
            prev = name
        leaves.append(prev)
    nodes["m"] = {"type": "merge", "inputs": {"geos": leaves}}
    nodes["out"] = {"type": "output", "inputs": {"geo": "m"}}
    return _graph(nodes, "out")


@unittest.skipUnless(rust.available(), "the Rust core is not built (see rust/README.md)")
class RustFuzzTests(unittest.TestCase):
    def check_invariants(self, geo):
        t = geo.topology
        self.assertEqual(geo.P.dtype, np.dtype("float32"))
        self.assertTrue(np.isfinite(geo.P).all())
        self.assertTrue(((t.vertex_point >= 0) & (t.vertex_point < geo.npoints)).all())
        self.assertEqual(int(t.prim_offsets[0]), 0)
        self.assertTrue((np.diff(t.prim_offsets) >= 0).all())
        self.assertEqual(int(t.prim_offsets[-1]), len(t.vertex_point))
        for domain in ("point", "vertex", "prim"):
            table = geo.table(domain)
            if "N" in table:
                n = np.asarray(table["N"])
                lengths = np.linalg.norm(n, axis=1)
                self.assertTrue(np.isfinite(n).all())
                np.testing.assert_allclose(lengths[lengths > 1e-6], 1, atol=1e-4, rtol=0)

    def check_graph(self, graph, seed):
        case = json.dumps(graph, sort_keys=True, allow_nan=True)
        with self.subTest(seed=seed, graph=case):
            parsed = Graph.from_dict(graph)
            on = Evaluator().cook(parsed, parsed.output_node())
            standalone = rust.cook(graph)
            self.check_invariants(on)
            self.check_invariants(standalone)
            assert_same_geometry(self, on, standalone)
            for oracle, context in paths():
                with self.subTest(seed=seed, oracle=oracle, graph=case), context():
                    off = py_cook(graph)
                    self.check_invariants(off)
                    assert_same_geometry(self, off, on)
                    assert_same_geometry(self, off, standalone)

    def test_seeded_random_graphs(self):
        for seed in range(10):
            self.check_graph(_random_graph(seed), seed)

    def test_degenerate_sources_and_parameters(self):
        cases = [
            chain(source={"type": "grid", "params": {"columns": 2, "rows": 2, "size_x": 0, "size_y": 0}}),
            chain(source={"type": "grid", "params": {"columns": 2, "rows": 7, "size_x": 1e-8}}),
            chain(source={"type": "grid", "params": {"columns": 7, "rows": 2, "size_y": 1e-8}}),
            *[chain(source={"type": "box", "params": {"divisions": 1, "size": [0, 0, 0],
                   "connectivity": connectivity}}) for connectivity in ("triangles", "quadrilaterals")],
            chain(("normal", {}), ("transform", {"scale": [0, 0, 0]})),
            chain(("transform", {"group": "0", "proportional_enabled": True, "proportional_connected": True,
                                  "proportional_radius": 0.5, "translate": [1,0,0]}),
                  source={"type": "line", "params": {"points": 2, "length": 0}}),
            chain(("bound", {}), source={"type": "point", "params": {"center": [1.25, -2, 3]}}),
            chain(("bound", {"padding": [0.1, 0.2, 0.3]}), source={"type": "curve", "params": {"points": "0,0,0 0,0,0"}}),
            chain(source={"type": "curve", "params": {"points": "0,0,0 0,0,0"}}),
            chain(source={"type": "circle", "params": {"divisions": 3, "radius": 0}}),
            chain(source={"type": "sphere", "params": {"rows": 2, "columns": 3, "radius": 0}}),
            chain(source={"type": "tube", "params": {"rows": 1, "columns": 3, "radius_bottom": 0, "radius_top": 0}}),
            chain(source={"type": "torus", "params": {"rows": 3, "columns": 3, "thickness": 0}}),
            chain(("transform", {"translate": [1e6, -1e6, 1e6]}),
                  source={"type": "box", "params": {"size": [2, 2, 2]}}),
            chain(("delete", {"expr": "@P.x > 0"}), source={"type": "point", "params": {"center": [1, 0, 0]}}),
            chain(("delete", {"expr": "1"}), source={"type": "box", "params": {"divisions": 1}}),
            chain(("delete", {"expr": ""}), source={"type": "grid", "params": {"columns": 2, "rows": 2}}),
            chain(("delete", {"expr": "@P.z > 100", "invert": True}), source={"type": "curve", "params": {"points": "0,0,0 1,1,1"}}),
            *[chain(("noise", {"octaves": octaves, "frequency": frequency,
                      "amplitude": [amplitude] * 3}))
              for octaves, frequency, amplitude in ((1, 0, 1), (12, 2, 0))],
        ]
        for i, graph in enumerate(cases):
            self.check_graph(graph, f"degenerate-{i}")

    def test_grid_one_row_or_column_is_rejected(self):
        for columns, rows in ((1, 1), (1, 5), (5, 1)):
            graph = chain(source={"type": "grid", "params": {"columns": columns, "rows": rows}})
            case = json.dumps(graph, sort_keys=True)
            with self.subTest(seed=f"grid-{columns}-{rows}", graph=case):
                with self.assertRaises(ValueError):
                    py_cook(graph)
                with self.assertRaises(ValueError):
                    Evaluator().cook(Graph.from_dict(graph), graph["display"])
                with self.assertRaises(RuntimeError):
                    rust.cook(graph)

    def test_empty_geometry_merge_bypass_and_pass_through(self):
        nodes = {"empty": {"type": "grid", "flags": {"bypass": True}}}
        prev = "empty"
        for i, kind in enumerate(("transform", "normal", "noise")):
            name = f"n{i}"
            nodes[name] = {"type": kind, "inputs": {"geo": prev}}
            prev = name
        nodes["one"] = {"type": "merge", "inputs": {"geos": [prev]}}
        nodes["many"] = {"type": "merge", "inputs": {"geos": ["empty", "one", prev]}}
        nodes["pass"] = {"type": "null", "inputs": {"geo": "many"}}
        nodes["out"] = {"type": "output", "inputs": {"geo": "pass"}}
        graph = _graph(nodes, "out")
        self.check_graph(graph, "empty-bypass")
        self.assertEqual(py_cook(graph).npoints, 0, json.dumps(graph))
        self.assertEqual(py_cook(graph).nprims, 0, json.dumps(graph))

    def test_one_and_many_input_merge(self):
        for count in (1, 4):
            nodes = {f"g{i}": {"type": "grid", "params": {"columns": 2 + i, "rows": 2}}
                     for i in range(count)}
            nodes["m"] = {"type": "merge", "inputs": {"geos": list(nodes)}}
            self.check_graph(_graph(nodes, "m"), f"merge-{count}")

    def test_determinism_and_thread_count(self):
        graph = chain(("noise", {"octaves": 4, "seed": 77, "amplitude": [0.1, 0.2, 0.3]}),
                      ("normal", {}), ("transform", {"translate": [0.5, 0.2, -0.3]}),
                      source={"type": "grid", "params": {"columns": 40, "rows": 31}})
        case = json.dumps(graph, sort_keys=True)
        for cook in (lambda: rust.cook(graph), lambda: Evaluator().cook(Graph.from_dict(graph), graph["display"]),
                     lambda: py_cook(graph)):
            a, b = cook(), cook()
            with self.subTest(graph=case, cook=str(cook)):
                for domain in ("point", "vertex", "prim"):
                    for name in a.table(domain).names():
                        self.assertEqual(a.table(domain)[name].tobytes(), b.table(domain)[name].tobytes())
                self.assertEqual(a.topology.vertex_point.tobytes(), b.topology.vertex_point.tobytes())
                self.assertEqual(a.topology.prim_offsets.tobytes(), b.topology.prim_offsets.tobytes())
        script = ("import json,sys; from minihou import rust; "
                  "g=rust.cook(json.loads(sys.argv[1])); "
                  "print(json.dumps({k:g.point[k].tobytes().hex() for k in ('P','N')}))")
        env = os.environ.copy()
        env.pop("RAYON_NUM_THREADS", None)
        default = subprocess.run([sys.executable, "-c", script, case], env=env, check=True,
                                 capture_output=True, text=True, encoding="utf-8", timeout=15)
        env["RAYON_NUM_THREADS"] = "1"
        single = subprocess.run([sys.executable, "-c", script, case], env=env, check=True,
                                capture_output=True, text=True, encoding="utf-8", timeout=15)
        self.assertEqual(json.loads(default.stdout.splitlines()[-1]), json.loads(single.stdout.splitlines()[-1]), case)

    def test_refused_settings_fall_back(self):
        cases = [chain(("transform", {"proportional_enabled": True, "proportional_falloff": "random"})),
                 chain(("normal", {"domain": "vertex"})),
                 chain(("noise", {"group": "0-3"})),
                 chain(("bound", {"oriented": True})),
                 chain(source={"type": "box", "params": {"add_vertex_normals": True}}),
                 _graph({"g": {"type": "grid"}, "b": {"type": "box", "inputs": {"geo": "g"}}}, "b")]
        for i, graph in enumerate(cases):
            case = json.dumps(graph, sort_keys=True)
            with self.subTest(seed=f"refusal-{i}", graph=case):
                with self.assertRaisesRegex(RuntimeError, "Rust backend"):
                    rust.cook(graph)
                on = Evaluator().cook(Graph.from_dict(graph), graph["display"])
                off = py_cook(graph)
                self.check_invariants(on)
                self.check_invariants(off)
                assert_same_geometry(self, on, off)

    def test_nan_transform_is_error_or_consistent(self):
        graph = chain(("transform", {"translate": [float("nan"), 0, 0]}))
        case = json.dumps(graph, sort_keys=True)
        results = []
        for cook in (lambda: py_cook(graph), lambda: Evaluator().cook(Graph.from_dict(graph), graph["display"]),
                     lambda: rust.cook(graph)):
            try:
                results.append(cook())
            except (ValueError, RuntimeError) as exc:
                results.append(exc)
        with self.subTest(seed="nan-transform", graph=case):
            self.assertEqual(len(results), 3)
            self.assertTrue(all(isinstance(x, Exception) for x in results) or
                            all(not isinstance(x, Exception) for x in results), case)
            if not isinstance(results[0], Exception):
                for geo in results[1:]:
                    assert_same_geometry(self, results[0], geo, atol=0)


if __name__ == "__main__":
    unittest.main()
