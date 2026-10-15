"""Incremental Rust Session parity, invalidation, and cache behavior."""
import copy
import json
import unittest
from concurrent.futures import ThreadPoolExecutor

import numpy as np

from minihou import Geometry, rust


def _graph(nodes, display):
    return {"version": 1, "nodes": nodes, "display": display}


def _chain():
    return _graph({
        "grid": {"type": "grid", "params": {"columns": 7, "rows": 5, "size_x": 3.0}},
        "noise": {"type": "noise", "params": {"seed": 7, "amplitude": [0.2, 0.3, 0.4]},
                  "inputs": {"geo": "grid"}},
        "transform": {"type": "transform", "params": {"translate": [1, 0, -1]},
                      "inputs": {"geo": "noise"}},
        "normal": {"type": "normal", "params": {"domain": "point"},
                   "inputs": {"geo": "transform"}},
    }, "normal")


def _same_bytes(tc, actual, expected):
    tc.assertIsInstance(actual, Geometry)
    tc.assertEqual(actual.npoints, expected.npoints)
    tc.assertEqual(actual.nvertices, expected.nvertices)
    tc.assertEqual(actual.nprims, expected.nprims)
    for name in ("vertex_point", "prim_offsets"):
        a, b = getattr(actual.topology, name), getattr(expected.topology, name)
        tc.assertEqual((a.dtype, a.shape, a.tobytes()), (b.dtype, b.shape, b.tobytes()), name)
    a, b = actual.topology.closed_mask(), expected.topology.closed_mask()
    tc.assertEqual((a.dtype, a.shape, a.tobytes()), (b.dtype, b.shape, b.tobytes()), "closed")
    for domain in ("point", "vertex", "prim"):
        left, right = actual.table(domain), expected.table(domain)
        tc.assertEqual(sorted(left.names()), sorted(right.names()), domain)
        for name in left.names():
            a, b = np.asarray(left[name]), np.asarray(right[name])
            tc.assertEqual((a.dtype, a.shape, a.tobytes()), (b.dtype, b.shape, b.tobytes()),
                           f"{domain}.{name}")
    tc.assertEqual(actual.detail, expected.detail)


def _v3(rng, lo, hi):
    return [float(x) for x in rng.uniform(lo, hi, 3)]


def _random_graph(rng):
    nodes = {}
    leaves = []
    for branch in range(2):
        source = f"source{branch}"
        if int(rng.integers(2)):
            nodes[source] = {"type": "grid", "params": {
                "columns": int(rng.integers(2, 8)), "rows": int(rng.integers(2, 8)),
                "size_x": float(rng.uniform(0.5, 4)), "size_y": float(rng.uniform(0.5, 4))}}
        else:
            nodes[source] = {"type": "box", "params": {
                "divisions": int(rng.integers(1, 4)), "size": _v3(rng, 0.5, 3),
                "connectivity": str(rng.choice(["triangles", "quadrilaterals"]))}}
        prev = source
        for step in range(int(rng.integers(1, 3))):
            name = f"branch{branch}_{step}"
            kind = str(rng.choice(["transform", "normal", "noise"]))
            if kind == "transform":
                params = {"translate": _v3(rng, -2, 2), "scale": _v3(rng, 0.5, 2)}
            elif kind == "normal":
                params = {"domain": str(rng.choice(["point", "prim"]))}
            else:
                params = {"amplitude": _v3(rng, 0.05, 0.3),
                          "frequency": float(rng.uniform(0.3, 2)),
                          "octaves": int(rng.integers(1, 4)),
                          "seed": int(rng.integers(-100, 100))}
            nodes[name] = {"type": kind, "params": params, "inputs": {"geo": prev}}
            prev = name
        leaves.append(prev)
    nodes["merge"] = {"type": "merge", "inputs": {"geos": leaves}}
    return _graph(nodes, "merge")


def _new_value(rng, kind, name, old):
    if name in ("columns", "rows", "divisions", "octaves"):
        return int(old) + 1
    if name == "seed":
        return int(old) + 1
    if name in ("size_x", "size_y", "frequency"):
        return float(old) + 0.25
    if name in ("size", "translate", "scale", "amplitude"):
        return [float(x) + 0.125 for x in old]
    if name == "domain":
        return "prim" if old == "point" else "point"
    if name == "connectivity":
        return "triangles" if old == "quadrilaterals" else "quadrilaterals"
    raise AssertionError((kind, name))


@unittest.skipUnless(rust.available() and hasattr(rust, "Session"),
                     "the Rust Session is not built")
class RustSessionTests(unittest.TestCase):
    def assert_fresh_parity(self, graph, target=None):
        _same_bytes(self, rust.Session(graph).cook(target), rust.cook(graph, target))

    def test_fresh_cooks_match_standalone(self):
        grid = _graph({"grid": {"type": "grid", "params": {"columns": 4, "rows": 3}}}, "grid")
        merged = _graph({
            "box": {"type": "box", "params": {"divisions": 2}},
            "left": {"type": "grid", "params": {"columns": 3, "rows": 4}},
            "right": {"type": "grid", "params": {"columns": 5, "rows": 2}},
            "merge": {"type": "merge", "inputs": {"geos": ["box", "left", "right"]}},
        }, "merge")
        bypassed = _chain()
        bypassed["nodes"]["transform"]["flags"] = {"bypass": True}
        for label, graph in (("grid", grid), ("chain", _chain()),
                             ("merge", merged), ("bypass", bypassed)):
            with self.subTest(graph=label):
                self.assert_fresh_parity(graph)
        self.assert_fresh_parity(_chain(), "noise")
        self.assert_fresh_parity(json.dumps(_chain()))

    def test_set_param_matches_edited_graph(self):
        graph = _chain()
        session = rust.Session(graph)
        session.cook()
        session.set_param("transform", "translate", [0.25, 1.5, -2])
        graph["nodes"]["transform"]["params"]["translate"] = [0.25, 1.5, -2]
        _same_bytes(self, session.cook(), rust.cook(graph))

    def test_set_bypass_matches_edited_graph(self):
        graph = _chain()
        session = rust.Session(graph)
        session.cook()
        session.set_bypass("transform", True)
        graph["nodes"]["transform"]["flags"] = {"bypass": True}
        _same_bytes(self, session.cook(), rust.cook(graph))
        session.set_bypass("transform", False)
        graph["nodes"]["transform"]["flags"]["bypass"] = False
        _same_bytes(self, session.cook(), rust.cook(graph))

    def test_stats_follow_content_invalidation(self):
        graph = _chain()
        session = rust.Session(graph)
        session.cook()
        self.assertEqual(session.last_stats["cooked"], ["grid", "noise", "transform", "normal"])
        self.assertEqual(session.last_stats["cached"], [])
        self.assertEqual(set(session.last_stats["node_ms"]), set(graph["nodes"]))
        self.assertTrue(all(isinstance(ms, float) and ms >= 0
                            for ms in session.last_stats["node_ms"].values()))
        session.cook()
        self.assertEqual(session.last_stats["cooked"], [])
        self.assertIn("normal", session.last_stats["cached"])
        session.set_param("noise", "seed", 8)
        session.cook()
        self.assertEqual(session.last_stats["cooked"], ["noise", "transform", "normal"])
        self.assertEqual(session.last_stats["cached"], ["grid"])
        session.set_param("grid", "size_x", 4.0)
        session.cook()
        self.assertEqual(session.last_stats["cooked"], ["grid", "noise", "transform", "normal"])

    def test_seeded_edits_match_standalone(self):
        for seed in (11, 23, 37):
            rng = np.random.default_rng(seed)
            graph = _random_graph(rng)
            session = rust.Session(copy.deepcopy(graph))
            edits = []
            with self.subTest(seed=seed, edits=edits, graph=json.dumps(graph, sort_keys=True)):
                _same_bytes(self, session.cook(), rust.cook(graph))
            for _ in range(8):
                editable = [name for name, node in graph["nodes"].items() if node.get("params")]
                node_id = str(rng.choice(editable))
                node = graph["nodes"][node_id]
                if int(rng.integers(2)):
                    param = str(rng.choice(list(node["params"])))
                    value = _new_value(rng, node["type"], param, node["params"][param])
                    session.set_param(node_id, param, value)
                    node["params"][param] = value
                    edits.append(["set_param", node_id, param, value])
                else:
                    flags = node.setdefault("flags", {})
                    bypass = not flags.get("bypass", False)
                    session.set_bypass(node_id, bypass)
                    flags["bypass"] = bypass
                    edits.append(["set_bypass", node_id, bypass])
                with self.subTest(seed=seed, edits=json.dumps(edits), graph=json.dumps(graph, sort_keys=True)):
                    _same_bytes(self, session.cook(), rust.cook(graph))

    def test_errors_leave_session_usable(self):
        graph = _chain()
        session = rust.Session(graph)
        with self.assertRaises(ValueError):
            session.set_param("missing", "seed", 1)
        with self.assertRaises(ValueError):
            session.set_param("noise", "missing", 1)
        session.cook()
        session.set_param("noise", "group", "0-3")
        with self.assertRaisesRegex(RuntimeError, "Rust backend"):
            session.cook()
        session.set_param("noise", "group", "")
        graph["nodes"]["noise"]["params"]["group"] = ""
        _same_bytes(self, session.cook(), rust.cook(graph))

    def test_budget_and_clear(self):
        graph = _chain()
        session = rust.Session(graph)
        session.set_budget_bytes(1)
        sizes = []
        for seed in range(12):
            session.set_param("noise", "seed", seed)
            graph["nodes"]["noise"]["params"]["seed"] = seed
            _same_bytes(self, session.cook(), rust.cook(graph))
            sizes.append(session.cache_bytes())
        self.assertTrue(all(isinstance(size, int) and size >= 0 for size in sizes))
        self.assertLessEqual(max(sizes), 4 * rust.cook(graph).nbytes)
        session.set_budget_bytes(None)
        session.clear()
        self.assertEqual(session.cache_bytes(), 0)
        session.cook()
        self.assertGreater(session.cache_bytes(), 0)
        session.clear()
        self.assertEqual(session.cache_bytes(), 0)
        session.set_budget_bytes(0)
        session.cook()
        self.assertGreater(session.cache_bytes(), 0)

    def test_sessions_are_independent(self):
        graph = _chain()
        left, right = rust.Session(graph), rust.Session(graph)
        original = rust.cook(graph)
        _same_bytes(self, left.cook(), original)
        _same_bytes(self, right.cook(), original)
        left.set_param("noise", "seed", 19)
        edited = copy.deepcopy(graph)
        edited["nodes"]["noise"]["params"]["seed"] = 19
        _same_bytes(self, left.cook(), rust.cook(edited))
        _same_bytes(self, right.cook(), original)
        self.assertEqual(right.last_stats["cooked"], [])

    def test_separate_sessions_in_threads(self):
        graph = _chain()
        expected = rust.cook(graph)

        def work(_):
            return rust.Session(copy.deepcopy(graph)).cook()

        with ThreadPoolExecutor(max_workers=4) as pool:
            for result in pool.map(work, range(4)):
                _same_bytes(self, result, expected)


if __name__ == "__main__":
    unittest.main()
