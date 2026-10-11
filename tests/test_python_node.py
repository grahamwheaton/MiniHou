"""The python node: real Python in the graph, switched off unless the engine is told otherwise."""
import os
import subprocess
import sys
import unittest

import numpy as np

from minihou import Evaluator, Geometry, Graph, NodeError
from minihou.nodes import python_node
from tests.helpers import graph_from
from tests.test_nodes import run

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


def cook(code="", inputs=None, **params):
    nodes = {"g": {"type": "grid", "params": {"columns": 4, "rows": 4}}, "s": {"type": "sphere"},
             "p": {"type": "python", "inputs": {"geo": "g", **(inputs or {})}, "params": dict(params, code=code)},
             "o": {"type": "output", "inputs": {"geo": "p"}}}
    return Evaluator().cook(graph_from(nodes, display="o"))


class Enabled(unittest.TestCase):
    def setUp(self):
        python_node.set_allowed(True)
        self.addCleanup(python_node.set_allowed, None)


class PythonNodeTests(Enabled):
    def test_default_code_moves_points_with_the_piped_in_variables(self):
        base = cook("out = geo", a=0.0)
        moved = cook(python_node.DEFAULT_CODE, a=2.0, b=1.5)
        np.testing.assert_allclose(moved.P[:, 2], 2.0 * np.sin(base.P[:, 0] * 1.5), atol=1e-5)
        np.testing.assert_array_equal(moved.P[:, :2], base.P[:, :2])

    def test_inputs_variables_and_helpers_are_in_scope(self):
        out = cook("""
out = geo.copy()
w = np.full(geo.npoints, a + b + c + d + i + j + v[2] + (10 if flag else 0) + len(s), np.float32)
out.point["w"] = w
out.detail["sphere_points"] = input1.npoints
out.detail["scale"] = factor * 2
out.detail["names"] = len(names)
""", inputs={"input1": "s"}, a=1, b=2, c=3, d=4, i=5, j=6, v=[0, 0, 7], s="abc", flag=True,
                   variables="factor = 2.5  # a comment\nnames = ['x', 'y']")
        self.assertTrue(np.all(out.point["w"] == 1 + 2 + 3 + 4 + 5 + 6 + 7 + 10 + 3))
        self.assertEqual(out.detail["sphere_points"], run("sphere").npoints)
        self.assertEqual((out.detail["scale"], out.detail["names"]), (5.0, 2))

    def test_out_can_be_points_or_left_unset(self):
        pts = cook("out = np.array([[0, 0, 0], [1, 2, 3]])")
        self.assertEqual((pts.npoints, pts.nprims), (2, 0))
        same = cook("x = 1")                                                  # no `out`: geo passes through
        self.assertEqual(same.npoints, 16)
        free = Evaluator().cook(graph_from({"p": {"type": "python", "params": {"code": "out = np.zeros((3, 3))"}}}, display="p"))
        self.assertEqual(free.npoints, 3)                                    # no input needed when `out` is built

    def test_errors_name_the_line_and_hint_at_read_only_inputs(self):
        for code, needle in (("x = 1\ny = undefined_name", "line 2"), ("def broken(:\n  pass", "SyntaxError"),
                             ("out = 5", "Geometry or an (n, 3) array"), ("raise KeyError('boom')", "KeyError")):
            with self.assertRaises(NodeError, msg=code) as cm:
                cook(code)
            self.assertIn(needle, str(cm.exception), code)
            self.assertIsInstance(cm.exception.__cause__, ValueError)
        with self.assertRaises(NodeError) as cm:
            cook("geo.P[:, 2] = 1")
        self.assertIn("read-only", str(cm.exception))
        self.assertIn("geo.copy()", str(cm.exception))
        for bad, msg in (("1 + 1", "name = value"), ("a = 1", "already a variable"), ("x = len('ab')", "must be a number")):
            with self.assertRaises(NodeError) as cm:
                cook("out = geo", variables=bad)
            self.assertIn(msg, str(cm.exception))

    def test_cached_until_something_changes_and_animated_follows_the_frame(self):
        g = graph_from({"g": {"type": "grid"}, "p": {"type": "python", "inputs": {"geo": "g"},
                                                     "params": {"code": "out = geo.copy()\nout.detail['f'] = Frame"}},
                        "o": {"type": "output", "inputs": {"geo": "p"}}}, display="o")
        ev = Evaluator()
        ev.cook(g, frame=1)
        ev.cook(g, frame=5)
        self.assertEqual(ev.last_stats.cooked, [])                             # not animated: the frame is not in its key
        g.set_param("p", "animated", True)
        self.assertEqual(ev.cook(g, frame=1).detail["f"], 1.0)
        self.assertEqual(ev.cook(g, frame=7).detail["f"], 7.0)
        self.assertEqual(ev.cook(g, frame=7).detail["f"], 7.0)
        self.assertEqual(ev.last_stats.cooked, [])


class SwitchTests(unittest.TestCase):
    def test_off_by_default_and_a_graph_cannot_turn_it_on(self):
        python_node.set_allowed(None)
        os.environ.pop("MINIHOU_PYTHON_NODES", None)
        self.assertFalse(python_node.allowed())
        with self.assertRaises(NodeError) as cm:
            cook("out = geo")
        self.assertIn("switched off", str(cm.exception))
        self.assertIn("--allow-python", str(cm.exception))
        with self.assertRaises(NodeError):                                      # code cannot reach the switch via the graph
            cook("out = geo", variables="allowed = True")
        os.environ["MINIHOU_PYTHON_NODES"] = "1"
        try:
            self.assertTrue(python_node.allowed())
            self.assertEqual(cook("out = geo").npoints, 16)
        finally:
            os.environ.pop("MINIHOU_PYTHON_NODES", None)
        python_node.set_allowed(True)
        self.assertTrue(python_node.allowed())
        python_node.set_allowed(None)

    def test_command_line_flag_enables_it_for_a_cook(self):
        import json
        import tempfile
        g = {"version": 1, "nodes": {"g": {"type": "grid", "params": {"columns": 3, "rows": 3}},
                                     "p": {"type": "python", "inputs": {"geo": "g"}, "params": {"code": "out = geo"}},
                                     "o": {"type": "output", "inputs": {"geo": "p"}}}, "display": "o"}
        with tempfile.TemporaryDirectory() as d:
            path = os.path.join(d, "g.json")
            with open(path, "w", encoding="utf-8") as f:
                json.dump(g, f)
            env = {k: v for k, v in os.environ.items() if k != "MINIHOU_PYTHON_NODES"}
            off = subprocess.run([sys.executable, "-m", "minihou", "cook", path], cwd=ROOT, env=env, capture_output=True, text=True, encoding="utf-8")
            on = subprocess.run([sys.executable, "-m", "minihou", "cook", path, "--allow-python"], cwd=ROOT, env=env,
                                capture_output=True, text=True, encoding="utf-8")
        self.assertNotEqual(off.returncode, 0)
        self.assertIn("switched off", off.stdout + off.stderr)
        self.assertEqual(on.returncode, 0, on.stderr)


if __name__ == "__main__":
    unittest.main()
