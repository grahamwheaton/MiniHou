"""uv_project: planar, box, cylindrical and spherical UVs, seams and poles, groups."""
import unittest

import numpy as np

from minihou import Evaluator, Graph


def cook(src, **params):
    return Evaluator().cook(Graph.from_dict({"nodes": {
        "s": src, "u": {"type": "uv_project", "inputs": {"geo": "s"}, "params": params},
        "o": {"type": "output", "inputs": {"geo": "u"}}}, "display": "o"}))


def face_spans(g):
    pv = np.repeat(np.arange(g.nprims), g.topology.prim_sizes())
    uv = g.vertex["uv"]
    return np.array([np.ptp(uv[pv == i], axis=0) for i in range(g.nprims)])


class UVTests(unittest.TestCase):
    def test_planar_and_box(self):
        g = cook({"type": "grid", "params": {"columns": 3, "rows": 3, "size_x": 1, "size_y": 1}}, method="planar")
        corners = g.P[g.topology.vertex_point]
        np.testing.assert_allclose(g.vertex["uv"], corners[:, :2] + 0.5, atol=1e-6)
        self.assertNotIn("uv", g.point)                                   # the grid's point uv is replaced
        b = cook({"type": "box", "params": {"size": [1, 1, 1]}}, method="box")
        np.testing.assert_allclose(face_spans(b), 1.0, atol=1e-6)         # every face fills the tile
        self.assertTrue(b.vertex["uv"].min() >= -1e-6 and b.vertex["uv"].max() <= 1 + 1e-6)

    def test_cylindrical_and_spherical_seams_and_poles(self):
        sphere = {"type": "sphere", "params": {"radius": 1, "rows": 12, "columns": 16}}
        for m in ("cylindrical", "spherical"):
            spans = face_spans(cook(sphere, method=m))
            self.assertLessEqual(float(spans[:, 0].max()), 1 / 16 + 1e-6, m)   # no face stretched over the seam

    def test_group_and_rotation(self):
        grid = {"type": "grid", "params": {"columns": 3, "rows": 3, "size_x": 1, "size_y": 1}}
        g = cook(grid, method="planar", group="0", rotate_uv=90)
        pv = np.repeat(np.arange(g.nprims), g.topology.prim_sizes())
        corners = g.P[g.topology.vertex_point][:, :2]
        np.testing.assert_allclose(g.vertex["uv"][pv == 0], np.c_[-corners[pv == 0, 1], corners[pv == 0, 0]] + 0.5,
                                   atol=1e-6)
        np.testing.assert_allclose(g.vertex["uv"][pv != 0], corners[pv != 0] + 0.5, atol=1e-6)  # grid's own uv kept


if __name__ == "__main__":
    unittest.main()
