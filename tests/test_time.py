"""Time: @Frame / @Time in wrangles and expressions, frame-aware cache keys, sessions and systems."""
import json
import os
import shutil
import tempfile
import unittest
from pathlib import Path

import numpy as np

from minihou import Evaluator, Graph, timectx


def graph(snippet="@P.z = @Frame", extra=None):
    nodes = {"g": {"type": "grid", "params": {"columns": 3, "rows": 3}},
             "still": {"type": "noise", "inputs": {"geo": "g"}, "params": {"frequency": 0.7}},
             "w": {"type": "wrangle", "inputs": {"geo": "still"}, "params": {"snippet": snippet}},
             "o": {"type": "output", "inputs": {"geo": "w"}}}
    nodes.update(extra or {})
    return Graph.from_dict({"nodes": nodes, "display": "o"})


class TimeTests(unittest.TestCase):
    def test_frame_and_time_values(self):
        ev = Evaluator()
        g = ev.cook(graph("@P.z = @Frame; f@t = @Time"), frame=25, fps=24)
        np.testing.assert_allclose(g.P[:, 2], 25)
        np.testing.assert_allclose(g.point["t"], 1.0)                     # frame 1 is time 0, like Houdini
        self.assertEqual(timectx.frame(), 1.0)                            # restored after the cook

    def test_only_time_dependent_nodes_recook(self):
        ev, g = Evaluator(), graph()
        ev.cook(g, frame=1)
        ev.cook(g, frame=2)
        self.assertEqual(sorted(ev.last_stats.cooked), ["o", "w"])         # grid and noise come from the cache
        ev.cook(g, frame=2)
        self.assertEqual(ev.last_stats.cooked, [])
        still = graph("@P.z = 3")
        ev.cook(still, frame=1)
        ev.cook(still, frame=9)
        self.assertEqual(ev.last_stats.cooked, [])                         # no @Frame: frames do not matter

    def test_expressions_and_groups_use_time(self):
        g = graph("f@k = 1", {"d": {"type": "delete", "inputs": {"geo": "w"}, "params": {"expr": "@ptnum < @Frame"}},
                              "o": {"type": "output", "inputs": {"geo": "d"}}})
        ev = Evaluator()
        self.assertEqual(ev.cook(g, frame=2).npoints, 7)
        self.assertEqual(ev.cook(g, frame=5).npoints, 4)

    def test_session_frames(self):
        from minihou.link.session import Session
        s = Session(graph())
        s.start()
        self.addCleanup(s.stop)
        s.wait_idle(5)
        rev = s.rev
        s.set_frame(12, 30)
        s.wait_idle(5)
        self.assertGreater(s.rev, rev)
        self.assertEqual(s.status()["frame"], 12)
        self.assertAlmostEqual(s.inspect("w")["bounds"]["max"][2], 12, places=4)
        rev = s.rev
        s.set_frame(12, 30)                                                # unchanged: nothing happens
        self.assertEqual(s.rev, rev)

    def test_systems_with_time_follow_the_frame(self):
        from minihou.library import NodeLibrary
        from minihou.sdk import REGISTRY
        tmp = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, tmp, True)
        name = f"bob_{os.getpid()}"
        (Path(tmp) / f"{name}.json").write_text(json.dumps({"nodes": {
            "g": {"type": "grid", "params": {"columns": 2, "rows": 2}},
            "w": {"type": "wrangle", "inputs": {"geo": "g"}, "params": {"snippet": "@P.z = sin(@Time * 6.2831853)"}},
            "o": {"type": "output", "inputs": {"geo": "w"}}}, "display": "o"}), encoding="utf-8")
        outer = f"bob_outer_{os.getpid()}"
        (Path(tmp) / f"{outer}.json").write_text(json.dumps({"nodes": {
            "b": {"type": name}, "o": {"type": "output", "inputs": {"geo": "b"}}}, "display": "o"}), encoding="utf-8")
        lib = NodeLibrary([tmp], REGISTRY)
        lib.refresh()
        self.addCleanup(lib.unload)
        g = Graph.from_dict({"nodes": {"s": {"type": outer}, "o": {"type": "output", "inputs": {"geo": "s"}}},
                             "display": "o"})
        ev = Evaluator()
        z = [float(ev.cook(g, frame=f, fps=4).P[0, 2]) for f in (1, 2, 3)]
        np.testing.assert_allclose(z, [0, 1, 0], atol=1e-5)                # nested system, still frame-accurate


if __name__ == "__main__":
    unittest.main()
