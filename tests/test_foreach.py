"""for_each: pieces, bodies (built-in nodes and systems), iteration attributes, cache keys."""
import json
import os
import shutil
import tempfile
import unittest
from pathlib import Path

import numpy as np

from minihou import Evaluator, Graph, NodeError
from minihou.library import NodeLibrary
from minihou.sdk import REGISTRY

THREE = {f"b{i}": {"type": "box", "params": {"size": [1, 1, 1], "center": [3 * i, 0, 0]}} for i in range(3)}
THREE["m"] = {"type": "merge", "inputs": {"geos": ["b0", "b1", "b2"]}}


def cook(nodes, last, ev=None):
    nodes = dict(nodes)
    nodes["out"] = {"type": "output", "inputs": {"geo": last}}
    return (ev or Evaluator()).cook(Graph.from_dict({"nodes": nodes, "display": "out"}))


class ForEachTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, self.tmp, True)
        self.name = f"stack_{os.getpid()}"

    def write_system(self, snippet):
        (Path(self.tmp) / f"{self.name}.json").write_text(json.dumps({"nodes": {
            "in": {"type": "input", "params": {"name": "piece"}},
            "w": {"type": "wrangle", "inputs": {"geo": "in"}, "params": {"snippet": snippet}},
            "o": {"type": "output", "inputs": {"geo": "w"}}}, "display": "o"}), encoding="utf-8")

    def test_builtin_body_per_connected_piece(self):
        nodes = dict(THREE, f={"type": "for_each", "inputs": {"geo": "m"},
                               "params": {"body": "polyextrude", "params": json.dumps({"group": "0", "distance": 0.5})}})
        g = cook(nodes, "f")
        self.assertEqual(g.nprims, 3 * (6 + 4))                          # every box extruded its own face 0
        self.assertNotIn("iteration", g.detail)

    def test_system_body_reads_iteration_and_edits_recook(self):
        self.write_system("@P.z += @iteration * 2 + @value * 0")
        lib = NodeLibrary([self.tmp], REGISTRY)
        lib.refresh()
        self.addCleanup(lib.unload)
        nodes = dict(THREE, f={"type": "for_each", "inputs": {"geo": "m"}, "params": {"body": self.name}})
        ev = Evaluator()
        g = cook(nodes, "f", ev)
        zmin = [float(g.P[np.abs(g.P[:, 0] - 3 * i) < 1][:, 2].min()) for i in range(3)]
        np.testing.assert_allclose(zmin, [-0.5, 1.5, 3.5])                # piece i lifted by 2*i
        self.write_system("@P.z += @iteration * 10")                       # edit the body: the loop recooks
        lib.refresh()
        g = cook(nodes, "f", ev)
        self.assertAlmostEqual(float(g.P[:, 2].max()), 20.5, places=4)

    def test_piece_modes_and_errors(self):
        grid = {"g": {"type": "grid", "params": {"columns": 3, "rows": 3}}}
        g = cook(dict(grid, f={"type": "for_each", "inputs": {"geo": "g"},
                               "params": {"body": "transform", "piece": "primitive",
                                          "params": json.dumps({"translate": [0, 0, 1]})}}), "f")
        self.assertEqual((g.nprims, g.npoints), (4, 16))                  # pieces no longer share points
        g = cook(dict(grid, w={"type": "wrangle", "inputs": {"geo": "g"},
                               "params": {"snippet": "i@side = @P.x > 0", "run_over": "primitives"}},
                      f={"type": "for_each", "inputs": {"geo": "w"},
                         "params": {"body": "facet", "piece": "attribute", "attribute": "side"}}), "f")
        self.assertEqual(g.nprims, 4)
        for params in ({"body": ""}, {"body": "nope"}, {"body": "grid"}, {"body": "facet", "params": "[1]"},
                       {"body": "facet", "piece": "attribute", "attribute": "missing"},
                       {"body": "facet", "piece": "primitive", "max_pieces": 2}):
            with self.assertRaises(NodeError, msg=params):
                cook(dict(grid, f={"type": "for_each", "inputs": {"geo": "g"}, "params": params}), "f")


if __name__ == "__main__":
    unittest.main()
