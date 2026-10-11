"""Every node on awkward inputs (empty, points only, curves, mixed, one face): it cooks, or fails with a clear
message, never with a numpy error leaking out (a bad reshape, a negative seed, an index out of bounds)."""
import unittest

from minihou import Evaluator, Graph, NodeError
from minihou.graph import GraphError
from minihou.sdk import REGISTRY

INPUTS = {
    "grid": {"type": "grid", "params": {"columns": 6, "rows": 5}},
    "box": {"type": "box", "params": {"divisions": 2}},
    "sphere": {"type": "sphere", "params": {"rows": 6, "columns": 8}},
    "curve": {"type": "helix", "params": {"points": 30}},
    "points": {"type": "scatter", "inputs": {"geo": "grid"}, "params": {"count": 25}},
    "empty": {"type": "blast", "inputs": {"geo": "grid"}, "params": {"group": "*", "domain": "points"}},
    "mixed": {"type": "merge", "inputs": {"geos": ["box", "curve"]}},
    "single": {"type": "blast", "inputs": {"geo": "grid"},
               "params": {"group": "0", "domain": "primitives", "delete_non_selected": True}},
}
SKIP = {"blender_object", "input", "output"}
NUMPY_WORDS = ("reshape", "zero-size", "non-negative integer", "broadcast", "out of bounds", "axis",
               "could not convert", "index")


def cook(node_type, src, params):
    nt = REGISTRY.get(node_type)
    inputs = {}
    for i, spec in enumerate(nt.inputs):
        inputs[spec.name] = [src, "box"] if spec.multi else (src if i == 0 else "sphere")
    nodes = dict(INPUTS)
    nodes["x"] = {"type": node_type, "inputs": inputs, "params": params}
    nodes["o"] = {"type": "output", "inputs": {"geo": "x"}}
    g = Evaluator().cook(Graph.from_dict({"nodes": nodes, "display": "o"}, check=False))
    g.validate()


class AwkwardInputTests(unittest.TestCase):
    def check(self, node_type, src, params):
        try:
            cook(node_type, src, params)
        except GraphError:
            pass
        except NodeError as e:
            cause = e.__cause__
            msg = f"{node_type} on {src} {params}: {type(cause).__name__}: {cause}"
            self.assertIsInstance(cause, (ValueError, ArithmeticError, type(None)), msg)
            self.assertFalse(any(w in str(cause) for w in NUMPY_WORDS), msg)

    def test_every_node_on_every_input(self):
        names = sorted(t for t in REGISTRY.names() if t not in SKIP)
        self.assertGreater(len(names), 50)
        for name in names:
            params = {p.name: -7 for p in REGISTRY.get(name).params if p.name == "seed"}
            for src in ("empty", "points", "curve", "mixed", "single"):
                with self.subTest(node=name, input=src):
                    self.check(name, src, {})
                    if params:
                        self.check(name, src, params)

    def test_group_expressions_on_points_only(self):
        for name in ("blast", "polyextrude", "subdivide", "facet"):
            for src in ("empty", "points"):
                with self.subTest(node=name, input=src):
                    self.check(name, src, {"group": "@P.z > 0"})


if __name__ == "__main__":
    unittest.main()
