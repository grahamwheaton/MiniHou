"""Golden values: the engine must produce the same numbers on every platform, numpy version and backend.

Parity tests compare backends *within* one environment; these pin results *across* environments (they caught noise
differing between Windows and Linux in CI). Values were generated on Linux with numpy 1.26 and 2.5 (identical).
Regenerate only for an intentional algorithm change (and bump the node's ``version``).
"""
import json
import unittest
from pathlib import Path

import numpy as np

from minihou import REGISTRY, accel
from minihou.nodes._util import _hash3, fbm3, fbm3_numpy, perlin3

GOLD = json.loads((Path(__file__).with_name("golden_values.json")).read_text(encoding="utf-8"))
IX = np.array([0, -1, 5, -7, 123456, -98765], np.int32)
PTS = np.array([[0.3, 0.7, 0.1], [-2.6, 3.1, 0.0], [10.5, -7.25, 3.3], [100.25, -0.5, 7.75]], np.float32)


def call(n, *i, **p):
    nt = REGISTRY.get(n)
    return nt.fn(*i, **nt.coerce_params(p))


class GoldenTests(unittest.TestCase):
    def test_hash_is_exact(self):
        self.assertEqual(_hash3(IX, IX + 1, IX * 3, 3).tolist(), GOLD["hash"])
        self.assertEqual(_hash3(IX, IX, IX, 2 ** 40 + 17).tolist(), GOLD["hash_seed_big"])

    def test_perlin_numpy(self):
        np.testing.assert_allclose(perlin3(PTS, 3), GOLD["perlin"], atol=1e-6)

    def test_fbm_numpy(self):
        np.testing.assert_allclose(fbm3_numpy(PTS, 7, 4, 2.3, 0.45), GOLD["fbm"], atol=1e-6)

    def test_fbm_active_backend(self):
        np.testing.assert_allclose(fbm3(PTS, 7, 4, 2.3, 0.45), GOLD["fbm"], atol=2e-6, err_msg=accel.status())

    def _nodes(self):
        g = call("grid", columns=7, rows=5, size_x=6, size_y=4).freeze()
        n = call("noise", g, amplitude=(0, 0, 1), frequency=0.4, seed=3, octaves=3).freeze()
        s = call("scatter", n, count=8, seed=2).freeze()
        c = call("copy_to_points", call("grid", columns=2, rows=2, size_x=0.2, size_y=0.2).freeze(), s)
        return n, s, c

    def check_nodes(self):
        n, s, c = self._nodes()
        np.testing.assert_allclose(n.P[:, 2], GOLD["noise_z"], atol=5e-6, err_msg="noise " + accel.status())
        self.assertEqual(s.point["primnum"].tolist(), GOLD["scatter_prim"])
        np.testing.assert_allclose(s.P, GOLD["scatter_P"], atol=1e-5, err_msg="scatter")
        np.testing.assert_allclose(c.P[:12], GOLD["copy_P_head"], atol=1e-5, err_msg="copy_to_points")

    def test_nodes_active_backend(self):
        self.check_nodes()

    def test_nodes_numpy_backend(self):
        with accel.disabled():
            self.check_nodes()


if __name__ == "__main__":
    unittest.main()
