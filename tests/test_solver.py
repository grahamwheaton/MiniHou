"""solver: deterministic frame-by-frame feedback, incremental playback, resets, substeps."""
import json
import unittest

import numpy as np

from minihou import Evaluator, Graph, NodeError

FALL = json.dumps({"snippet": "v@v += set(0, 0, -9.8) * @TimeInc; @P += @v * @TimeInc"})


def graph(params=None, start=1, substeps=1):
    return Graph.from_dict({"nodes": {
        "p": {"type": "grid", "params": {"columns": 2, "rows": 2}},
        "init": {"type": "wrangle", "inputs": {"geo": "p"}, "params": {"snippet": "v@v = set(0, 0, 5)"}},
        "s": {"type": "solver", "inputs": {"geo": "init"},
              "params": {"body": "wrangle", "params": params or FALL, "start_frame": start, "substeps": substeps}},
        "o": {"type": "output", "inputs": {"geo": "s"}}}, "display": "o"})


class SolverTests(unittest.TestCase):
    def test_same_frame_same_result_however_you_get_there(self):
        direct = Evaluator().cook(graph(), frame=30).P
        ev = Evaluator()
        for f in range(1, 31):
            stepped = ev.cook(graph(), frame=f).P
        np.testing.assert_array_equal(stepped, direct)
        back = ev.cook(graph(), frame=10).P                              # scrub back: from memory
        np.testing.assert_array_equal(back, Evaluator().cook(graph(), frame=10).P)

    def test_physics_start_frame_and_substeps(self):
        ev = Evaluator()
        np.testing.assert_allclose(ev.cook(graph(), frame=1).P[:, 2], 0)     # start frame: the input
        z1 = ev.cook(graph(), frame=25).P[0, 2]
        z8 = ev.cook(graph(substeps=8), frame=25).P[0, 2]
        exact = 5 * 1.0 - 4.9 * 1.0 ** 2
        self.assertLess(abs(z8 - exact), abs(z1 - exact))                 # substeps: more accurate
        late = ev.cook(graph(start=10), frame=12).P[0, 2]
        self.assertAlmostEqual(float(late), float(ev.cook(graph(), frame=3).P[0, 2]), places=5)

    def test_growth_and_errors(self):
        grow = json.dumps({"snippet": "@P.z += 1"})
        self.assertAlmostEqual(float(Evaluator().cook(graph(grow), frame=7).P[0, 2]), 6.0)
        for bad in ({"body": "nope"}, {"body": "grid"}, {"body": "wrangle", "params": "{bad"}):
            g = graph()
            g.nodes["s"].params.update(bad)
            with self.assertRaises(NodeError):
                Evaluator().cook(g, frame=3)


if __name__ == "__main__":
    unittest.main()
