"""The road_network library node (nodes/road_network.py): strips of road, a patch at every junction."""
import unittest
from pathlib import Path

import numpy as np

from minihou import Evaluator, Graph
from minihou.library import NodeLibrary

ROOT = Path(__file__).resolve().parent.parent


def signed_areas(geo):
    t = geo.topology
    out = []
    for f in range(geo.nprims):
        q = geo.P[t.vertex_point[t.prim_offsets[f]:t.prim_offsets[f + 1]]][:, :2].astype(np.float64)
        out.append(0.5 * np.sum(q[:, 0] * np.roll(q[:, 1], -1) - np.roll(q[:, 0], -1) * q[:, 1]))
    return np.array(out)


class RoadNetworkTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.lib = NodeLibrary([ROOT / "nodes"])
        cls.lib.refresh()

    @classmethod
    def tearDownClass(cls):
        cls.lib.unload()

    def roads(self, curves, **params):
        nodes, prev = {}, None
        for i, pts in enumerate(curves):
            nodes[f"c{i}"] = {"type": "curve", "params": {"points": pts}, **({"inputs": {"geo": prev}} if prev else {})}
            prev = f"c{i}"
        nodes["r"] = {"type": "road_network", "inputs": {"curves": prev}, "params": params}
        return Evaluator().cook(Graph.from_dict({"nodes": nodes, "display": "r"}))

    def test_a_crossroads_is_four_strips_and_one_square_with_no_overlap(self):
        g = self.roads(["-10,0,0 0,0,0 10,0,0", "0,-10,0 0,0,0 0,10,0"], width=4)
        self.assertEqual(g.prim["part"].tolist(), [0, 0, 0, 0, 1])
        self.assertEqual(np.diff(g.topology.prim_offsets).tolist(), [4, 4, 4, 4, 4])
        a = signed_areas(g)
        self.assertTrue((a > 0).all())                                   # all face up
        self.assertAlmostEqual(a.sum(), 2 * 20 * 4 - 4 * 4)               # two 20 x 4 roads, their crossing counted once
        j = int(g.prim["junction"][-1])                                   # an input point at the crossing: the one at the origin
        self.assertGreaterEqual(j, 0)
        self.assertEqual(g.prim["road"].tolist(), [0, 1, 2, 3, -1])

    def test_a_t_junction_and_a_y_junction(self):
        t = self.roads(["-10,0,0 0,0,0 10,0,0", "0,0,0 0,10,0"], width=4)
        self.assertAlmostEqual(signed_areas(t).sum(), 20 * 4 + 10 * 4 - 4 * 2)
        y = self.roads(["0,0,0 10,0,0", "0,0,0 -5,8.66,0", "0,0,0 -5,-8.66,0"], width=3)
        self.assertEqual(int((y.prim["part"] == 1).sum()), 1)
        self.assertTrue((signed_areas(y) > 0).all())
        for n in (3, 5, 6):                                             # any number of roads meets in one patch
            spokes = [f"0,0,0 {10 * np.cos(a):.3f},{10 * np.sin(a):.3f},0" for a in np.linspace(0, 2 * np.pi, n + 1)[:-1]]
            g = self.roads(spokes, width=1.5)
            self.assertEqual(int((g.prim["part"] == 1).sum()), 1)
            self.assertTrue((signed_areas(g) > 0).all())

    def test_bends_a_straight_road_and_a_dead_end(self):
        bend = self.roads(["-10,0,0 0,0,0 0,10,0"], width=4)
        self.assertEqual(bend.prim["part"].tolist(), [0, 0, 1])           # the outer wedge is filled
        self.assertTrue((signed_areas(bend) > 0).all())
        straight = self.roads(["-10,0,0 0,0,0 10,0,0"], width=4)
        self.assertEqual(straight.nprims, 2)                              # no patch where nothing meets
        self.assertAlmostEqual(signed_areas(straight).sum(), 20 * 4)

    def test_width_weld_group_and_curvy_input(self):
        wide = self.roads(["-10,0,0 0,0,0 10,0,0", "0,-10,0 0,0,0 0,10,0"], width=8)
        self.assertAlmostEqual(signed_areas(wide).sum(), 2 * 20 * 8 - 64)
        # curves whose ends nearly touch join when closer than the weld distance, and not otherwise
        near = ["-10,0,0 0,0,0", "0.02,0,0 10,0,0"]
        self.assertEqual(self.roads(near, weld=0.1).prim["part"].tolist().count(1), 0)   # one straight road
        self.assertEqual(self.roads(near, weld=0.0).nprims, 2)
        smooth = self.roads(["-10,0,0 -5,1,0 0,0,0 5,-1,0 10,0,0", "0,0,0 0,10,0"], width=3)
        self.assertTrue((signed_areas(smooth) > 0).all())
        self.assertEqual(smooth.vertex["uv"].shape, (smooth.nvertices, 2))

    def test_empty_input_and_the_bundled_example_cook(self):
        from minihou.geometry import Geometry
        fn = Evaluator().registry.get("road_network").fn
        self.assertEqual(fn(None, width=3, weld=0.05, corner_extra=0, min_segment=0, group="").nprims, 0)
        points = Geometry.points_only(np.zeros((3, 3), np.float32))        # points are not curves: nothing to build
        self.assertEqual(fn(points, width=3, weld=0.05, corner_extra=0, min_segment=0, group="").nprims, 0)
        import json
        g = Graph.from_dict(json.loads((ROOT / "examples" / "graphs" / "road_network.json").read_text(encoding="utf-8")))
        geo = Evaluator().cook(g)
        self.assertGreater(geo.nprims, 15)


if __name__ == "__main__":
    unittest.main()
