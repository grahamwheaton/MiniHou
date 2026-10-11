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


def _system(nodes, display="out", exposed=(), doc=""):
    d = {"version": 1, "nodes": nodes, "display": display, "exposed": list(exposed)}
    if doc:
        d["doc"] = doc
    return json.dumps(d)


LIFTER = {"preview": {"type": "grid", "params": {"columns": 2, "rows": 2}},
          "inp": {"type": "input", "inputs": {"preview": "preview"}, "params": {"name": "src"}},
          "move": {"type": "transform", "inputs": {"geo": "inp"}, "params": {"translate": [0, 0, 1]}},
          "out": {"type": "output", "inputs": {"geo": "move"}}}


class SystemTests(unittest.TestCase):
    """Graph files in a node folder used as node types ("systems", like Houdini digital assets)."""

    def setUp(self):
        self.dir = Path(tempfile.mkdtemp(prefix="minihou_sys_"))
        self.addCleanup(shutil.rmtree, self.dir, True)
        self.lib = NodeLibrary([self.dir])
        self.addCleanup(self.lib.unload)
        self.tag = f"{os.getpid()}"

    def write(self, name, text):
        p = self.dir / f"{name}.json"
        old = p.stat().st_mtime_ns if p.exists() else 0
        p.write_text(text, encoding="utf-8")
        if p.stat().st_mtime_ns == old:
            os.utime(p, ns=(old + 1_000_000, old + 1_000_000))
        return p

    def error(self, name):
        return next(f["error"] for f in self.lib.status()["files"] if Path(f["file"]).stem == name)

    def use(self, system, **params):
        return Graph.from_dict({"nodes": {"g": {"type": "grid", "params": {"columns": 3, "rows": 3}},
                                          "s": {"type": system, "inputs": {"src": "g"}, "params": params}},
                                "display": "s"}, check=False)

    def test_inputs_params_and_doc_come_from_the_graph(self):
        name = f"lifter_{self.tag}"
        self.write(name, _system(LIFTER, exposed=[{"name": "Lift By", "node": "move", "param": "translate"}],
                                 doc="Moves things up."))
        self.lib.refresh()
        nt = REGISTRY.get(name)
        self.assertEqual([i.name for i in nt.inputs], ["src"])
        self.assertEqual([(p.name, p.kind, p.label, p.default) for p in nt.params],
                         [("lift_by", "vec3", "Lift By", (0.0, 0.0, 1.0))])
        self.assertEqual((nt.doc, nt.category), ("Moves things up.", "system"))
        geo = Evaluator().cook(self.use(name, lift_by=[0, 0, 5]))
        self.assertEqual(geo.npoints, 9)                                     # the outer grid, not the preview
        self.assertAlmostEqual(float(geo.P[:, 2].max()), 5.0)
        unwired = Graph.from_dict({"nodes": {"s": {"type": name}}, "display": "s"})
        self.assertEqual(Evaluator().cook(unwired).npoints, 4)              # unconnected input: the preview

    def test_editing_a_system_updates_graphs_that_use_it(self):
        name = f"lifter_{self.tag}"
        self.write(name, _system(LIFTER))
        self.lib.refresh()
        ev, g = Evaluator(), self.use(name)
        self.assertAlmostEqual(float(ev.cook(g).P[:, 2].max()), 1.0)
        changed = json.loads(json.dumps(LIFTER))
        changed["move"]["params"]["translate"] = [0, 0, 3]
        self.write(name, _system(changed))
        self.assertTrue(self.lib.refresh())
        self.assertAlmostEqual(float(ev.cook(g).P[:, 2].max()), 3.0)          # not the cached old result
        self.assertIn("g", ev.last_stats.cached)

    def test_nested_systems_and_dependency_hashes(self):
        inner, outer = f"zz_inner_{self.tag}", f"aa_outer_{self.tag}"           # outer sorts first: order must not matter
        self.write(inner, _system(LIFTER))
        self.write(outer, _system({
            "inp": {"type": "input", "params": {"name": "src"}},
            "twice_a": {"type": inner, "inputs": {"src": "inp"}},
            "twice_b": {"type": inner, "inputs": {"src": "twice_a"}},
            "out": {"type": "output", "inputs": {"geo": "twice_b"}}}))
        self.lib.refresh()
        self.assertIsNone(self.error(outer))
        ev, g = Evaluator(), self.use(outer)
        self.assertAlmostEqual(float(ev.cook(g).P[:, 2].max()), 2.0)
        changed = json.loads(json.dumps(LIFTER))
        changed["move"]["params"]["translate"] = [0, 0, 10]
        self.write(inner, _system(changed))                                  # only the inner file changes...
        self.lib.refresh()
        self.assertAlmostEqual(float(ev.cook(g).P[:, 2].max()), 20.0)         # ...and the outer system follows

    def test_a_system_that_uses_itself_is_an_error(self):
        a, b = f"loop_a_{self.tag}", f"loop_b_{self.tag}"
        self.write(a, _system({"x": {"type": b}, "out": {"type": "output", "inputs": {"geo": "x"}}}))
        self.write(b, _system({"x": {"type": a}, "out": {"type": "output", "inputs": {"geo": "x"}}}))
        self.lib.refresh()
        self.assertIn("uses itself", self.error(a))
        self.assertIn("uses itself", self.error(b))
        self.assertNotIn(a, REGISTRY)

    def test_blender_object_cannot_be_inside_a_system(self):
        name = f"live_{self.tag}"
        self.write(name, _system({"b": {"type": "blender_object", "params": {"object": "Cube"}},
                                  "out": {"type": "output", "inputs": {"geo": "b"}}}))
        self.lib.refresh()
        self.assertIn("cannot be inside a system", self.error(name))
        self.assertNotIn(name, REGISTRY)

    def test_bad_system_files_are_reported(self):
        self.write("bad-name", _system(LIFTER))
        self.write(f"broken_{self.tag}", "{not json")
        self.write("noise", _system(LIFTER))                                  # would shadow a built-in
        self.write(f"badexpose_{self.tag}", _system(LIFTER, exposed=[{"name": "X", "node": "move", "param": "nope"}]))
        self.lib.refresh()
        self.assertIn("valid node name", self.error("bad-name"))
        self.assertIn("cannot read", self.error(f"broken_{self.tag}"))
        self.assertIn("already exists (built in)", self.error("noise"))
        self.assertIn("nope", self.error(f"badexpose_{self.tag}"))
        self.assertEqual(REGISTRY.get("noise").source, "")

    def test_example_system_and_graph(self):
        ex = NodeLibrary([os.path.join(ROOT, "nodes")])
        self.addCleanup(ex.unload)
        ex.refresh()
        self.assertEqual({f["error"] for f in ex.status()["files"]}, {None})
        nt = REGISTRY.get("rock_scatter")
        self.assertEqual(nt.category, "system")
        g = Graph.load(os.path.join(ROOT, "examples", "graphs", "rocky_hills.json"))
        geo = Evaluator().cook(g)
        self.assertGreater(geo.nprims, 1000)


class SystemSessionTests(unittest.TestCase):
    def test_editing_an_open_system_file_reloads_its_node_type(self):
        d = Path(tempfile.mkdtemp(prefix="minihou_sys_"))
        self.addCleanup(shutil.rmtree, d, True)
        name = f"lifter_s_{os.getpid()}"
        path = d / f"{name}.json"
        path.write_text(_system(LIFTER), encoding="utf-8")
        lib = NodeLibrary([d])
        self.addCleanup(lib.unload)
        lib.refresh()
        s = Session(Graph.load(path, check=False))                          # the system graph itself is open
        s.start()
        self.addCleanup(s.stop)
        s.watch_file(path, interval=0.02)
        s.attach_library(lib, interval=0.02)
        h0 = REGISTRY.get(name).code_hash
        s.edit([{"op": "set", "id": "move", "params": {"translate": [0, 0, 7]}}])     # saved by the session...
        self.assertTrue(until(lambda: REGISTRY.get(name).code_hash != h0))           # ...reloaded as a node type
        g = Graph.from_dict({"nodes": {"x": {"type": name}}, "display": "x"})
        self.assertAlmostEqual(float(Evaluator().cook(g).P[:, 2].max()), 7.0)
