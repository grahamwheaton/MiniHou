"""Which backend each node cooked with (CookStats.backend), which the editor shows as a tag on the node."""
import unittest

import minihou  # noqa: F401
from minihou import Evaluator, Graph, rust


def cook_backends(nodes: dict, display: str) -> dict:
    g = Graph.from_dict({"version": 1, "nodes": nodes, "display": display})
    ev = Evaluator()
    ev.cook(g, display)
    return ev.last_stats.backend


class BackendMarkerTests(unittest.TestCase):
    def test_nothing_is_recorded_when_rust_is_not_in_use(self):
        with rust.disabled():
            self.assertEqual(cook_backends({"g": {"type": "grid"}}, "g"), {})

    @unittest.skipUnless(rust.available(), "the Rust core is not built")
    def test_ported_python_and_fallback(self):
        nodes = {
            "g": {"type": "grid"},                                                       # runs in Rust
            "b": {"type": "box", "params": {"add_vertex_normals": True}},                # has a Rust path, refuses this
            "s": {"type": "helix"},                                                      # no Rust path
            "d": {"type": "bound", "inputs": {"geo": "g"}},                              # runs in Rust (its box is a kernel)
        }
        got = cook_backends(dict(nodes, m={"type": "merge", "inputs": {"geos": ["g", "b", "s", "d"]}}), "m")
        self.assertEqual(got["g"], "rust")
        self.assertEqual(got["b"], "fallback")
        self.assertEqual(got["s"], "python")
        self.assertEqual(got["d"], "rust")

    def test_the_stats_dict_carries_it(self):
        g = Graph.from_dict({"version": 1, "nodes": {"g": {"type": "grid"}}, "display": "g"})
        ev = Evaluator()
        ev.cook(g, "g")
        self.assertIn("backend", ev.last_stats.to_dict())


if __name__ == "__main__":
    unittest.main()
