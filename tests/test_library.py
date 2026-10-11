"""Node libraries (minihou.library): loading, hot reload, failures, cache safety, and the engine/MCP surfaces."""
import json
import os
import shutil
import subprocess
import sys
import tempfile
import textwrap
import time
import unittest
from pathlib import Path

import numpy as np

from minihou import REGISTRY, Evaluator, Graph, GraphError, NodeError
from minihou.library import TEMPLATE, NodeLibrary
from minihou.link.session import Session

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

LIFT = '''
import numpy as np
from minihou.sdk import Float, node

@node("{name}", inputs=["geo"], params=[Float("amount", 1.0)])
def {name}(geo, amount):
    """Lift points."""
    out = geo.copy()
    P = geo.P.copy()
    P[:, 2] += amount * {factor}
    out.point["P"] = P
    return out
'''


def until(pred, timeout=5.0):
    end = time.monotonic() + timeout
    while time.monotonic() < end:
        if pred():
            return True
        time.sleep(0.02)
    return False


class LibraryTests(unittest.TestCase):
    def setUp(self):
        self.dir = Path(tempfile.mkdtemp(prefix="minihou_lib_"))
        self.addCleanup(shutil.rmtree, self.dir, True)
        self.lib = NodeLibrary([self.dir])
        self.addCleanup(self.lib.unload)
        self.name = f"lift_{os.getpid()}_{id(self) % 100000}"

    def write(self, fname, text):
        p = self.dir / fname
        old = p.stat().st_mtime_ns if p.exists() else 0
        p.write_text(text, encoding="utf-8")
        if p.stat().st_mtime_ns == old:                       # coarse clocks: make the change visible
            os.utime(p, ns=(old + 1_000_000, old + 1_000_000))
        return p

    def graph(self):
        return Graph.from_dict({"nodes": {"g": {"type": "grid", "params": {"columns": 3, "rows": 3}},
                                          "l": {"type": self.name, "inputs": {"geo": "g"}},
                                          "o": {"type": "output", "inputs": {"geo": "l"}}}, "display": "o"}, check=False)

    def test_loads_nodes_with_source_and_hash(self):
        p = self.write("lift.py", LIFT.format(name=self.name, factor=1))
        self.assertTrue(self.lib.refresh())
        nt = REGISTRY.get(self.name)
        self.assertEqual(nt.source, str(p.resolve()))
        self.assertTrue(nt.code_hash)
        self.assertEqual(nt.category, "custom")
        self.assertEqual(nt.describe()["source"], str(p.resolve()))
        self.assertFalse(self.lib.refresh())                  # nothing changed
        self.assertEqual(self.lib.status()["files"][0]["nodes"], [self.name])

    def test_editing_the_code_never_serves_stale_cache(self):
        self.write("lift.py", LIFT.format(name=self.name, factor=1))
        self.lib.refresh()
        ev, g = Evaluator(), self.graph()
        self.assertAlmostEqual(float(ev.cook(g).P[:, 2].max()), 1.0)
        self.write("lift.py", LIFT.format(name=self.name, factor=5))
        self.assertTrue(self.lib.refresh())
        self.assertAlmostEqual(float(ev.cook(g).P[:, 2].max()), 5.0)          # same graph, same params: new result
        self.assertIn("l", ev.last_stats.cooked)
        self.assertIn("g", ev.last_stats.cached)                              # untouched upstream stays cached

    def test_broken_code_keeps_the_previous_version(self):
        self.write("lift.py", LIFT.format(name=self.name, factor=1))
        self.lib.refresh()
        self.write("lift.py", LIFT.format(name=self.name, factor=1).replace("out = geo.copy()", "out = geo.copy("))
        self.assertTrue(self.lib.refresh())
        err = self.lib.status()["files"][0]["error"]
        self.assertIn("SyntaxError", err)
        self.assertIn("line", err)
        self.assertAlmostEqual(float(Evaluator().cook(self.graph()).P[:, 2].max()), 1.0)    # old code still works
        self.write("lift.py", LIFT.format(name=self.name, factor=2) + "\nraise RuntimeError('boom')\n")
        self.lib.refresh()
        err = self.lib.status()["files"][0]["error"]
        self.assertIn("RuntimeError: boom", err)
        self.assertRegex(err, r"line \d+")
        self.assertAlmostEqual(float(Evaluator().cook(self.graph()).P[:, 2].max()), 1.0)
        self.write("lift.py", LIFT.format(name=self.name, factor=3))
        self.lib.refresh()
        self.assertIsNone(self.lib.status()["files"][0]["error"])
        self.assertAlmostEqual(float(Evaluator().cook(self.graph()).P[:, 2].max()), 3.0)

    def test_cannot_replace_a_built_in_node(self):
        before = REGISTRY.get("noise")
        self.write("evil.py", LIFT.format(name="noise", factor=1))
        self.lib.refresh()
        self.assertIn("already exists (built in)", self.lib.status()["files"][0]["error"])
        self.assertIs(REGISTRY.get("noise"), before)

    def test_deleted_or_renamed_nodes_disappear_and_graphs_still_open(self):
        p = self.write("lift.py", LIFT.format(name=self.name, factor=1))
        self.lib.refresh()
        saved = self.graph().to_json()
        p.unlink()
        self.assertTrue(self.lib.refresh())
        self.assertNotIn(self.name, REGISTRY)
        g = Graph.from_json(saved, check=False)                               # still opens...
        with self.assertRaises(GraphError) as cm:                             # ...and the cook names the problem
            Evaluator().cook(g)
        self.assertEqual(cm.exception.node_id, "l")
        self.assertIn("unknown node type", str(cm.exception))
        self.assertEqual(json.loads(g.to_json()), json.loads(saved))          # round-trips unchanged
        with self.assertRaises(GraphError):
            Graph.from_json(saved)                                            # a checked load still refuses it

    def test_runtime_errors_point_at_the_library_file(self):
        self.write("lift.py", LIFT.format(name=self.name, factor="np.array([1, 2])"))
        self.lib.refresh()
        with self.assertRaises(NodeError) as cm:
            Evaluator().cook(self.graph())
        self.assertIn("lift.py, line", str(cm.exception))

    def test_template_and_examples_load_and_cook(self):
        self.write("tmpl.py", TEMPLATE.format(name=self.name))
        self.lib.refresh()
        self.assertIsNone(self.lib.status()["files"][0]["error"])
        self.assertAlmostEqual(float(Evaluator().cook(self.graph()).P[:, 2].max()), 1.0)
        ex = NodeLibrary([os.path.join(ROOT, "nodes")])
        self.addCleanup(ex.unload)
        ex.refresh()
        self.assertEqual({e["error"] for e in ex.status()["files"]}, {None})
        g = Graph.from_dict({"nodes": {"s": {"type": "stairs", "params": {"steps": 5, "spiral": 25}},
                                       "t": {"type": "twist", "inputs": {"geo": "s"}},
                                       "n": {"type": "normal", "inputs": {"geo": "t"}, "params": {"domain": "prim"}}},
                             "display": "n"})
        geo = Evaluator().cook(g)
        self.assertEqual((geo.npoints, geo.nprims), (40, 30))
        # faces point outwards: each face normal points away from its step's box centre
        P, tp = geo.P, geo.topology
        corner_pts = P[tp.vertex_point].reshape(-1, 4, 3)
        face_c = corner_pts.mean(1)
        box_c = P.reshape(-1, 8, 3).mean(1)[geo.prim["step"]]
        self.assertTrue((np.sum(geo.prim["N"] * (face_c - box_c), axis=1) > 0).all())


class SessionLibraryTests(unittest.TestCase):
    def test_hot_reload_in_a_running_session(self):
        d = Path(tempfile.mkdtemp(prefix="minihou_lib_"))
        self.addCleanup(shutil.rmtree, d, True)
        name = f"bump_{os.getpid()}"
        (d / "bump.py").write_text(LIFT.format(name=name, factor=1), encoding="utf-8")
        lib = NodeLibrary([d])
        self.addCleanup(lib.unload)
        lib.refresh()
        g = Graph.from_dict({"nodes": {"g": {"type": "grid"}, "b": {"type": name, "inputs": {"geo": "g"}}},
                             "display": "b"})
        s = Session(g)
        s.start()
        self.addCleanup(s.stop)
        events = []

        class L:
            def on_result(self, r): pass
            def on_interface(self, i): pass
            def on_types(self): events.append("types")

        s.add_listener(L())
        s.attach_library(lib, interval=0.02)
        self.assertTrue(s.wait_idle())
        self.assertAlmostEqual(s.inspect("b")["bounds"]["max"][2], 1.0)
        time.sleep(0.05)
        p = d / "bump.py"
        p.write_text(LIFT.format(name=name, factor=4), encoding="utf-8")
        os.utime(p, ns=(time.time_ns(), time.time_ns()))
        self.assertTrue(until(lambda: "types" in events))
        self.assertTrue(s.wait_idle())
        self.assertAlmostEqual(s.inspect("b")["bounds"]["max"][2], 4.0)
        self.assertEqual(s.library_status()["files"][0]["nodes"], [name])


class CliTests(unittest.TestCase):
    def test_new_node_writes_a_loadable_template(self):
        d = tempfile.mkdtemp(prefix="minihou_cli_")
        self.addCleanup(shutil.rmtree, d, True)
        env = dict(os.environ, PYTHONPATH=ROOT)
        r = subprocess.run([sys.executable, "-m", "minihou", "new-node", "my_lift"], cwd=d, env=env,
                           capture_output=True, text=True, encoding="utf-8")
        self.assertEqual(r.returncode, 0, r.stderr)
        r = subprocess.run([sys.executable, "-m", "minihou", "nodes"], cwd=d, env=env,
                           capture_output=True, text=True, encoding="utf-8")
        self.assertIn("my_lift", r.stdout)
        self.assertNotIn("ERROR", r.stdout)
        r = subprocess.run([sys.executable, "-m", "minihou", "new-node", "my_lift"], cwd=d, env=env,
                           capture_output=True, text=True, encoding="utf-8")
        self.assertEqual(r.returncode, 2)                                         # never overwrites


if __name__ == "__main__":
    unittest.main()
