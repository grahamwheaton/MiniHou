"""Curves (open polylines): topology flag, propagation through nodes, the link, and the curve nodes."""
import unittest

import numpy as np

from minihou import Evaluator, Geometry, Graph, NodeError, topo
from minihou.link.diff import DiffState
from minihou.link.protocol import TOPO_PRIM_CLOSED, GeoUpdate, Mirror


def cook(nodes):
    nodes = dict(nodes)
    last = list(nodes)[-1]
    nodes["out"] = {"type": "output", "inputs": {"geo": last}}
    g = Evaluator().cook(Graph.from_dict({"nodes": nodes, "display": "out"}))
    g.validate()
    return g


def chain(*steps, src):
    nodes = dict(src)
    prev = list(nodes)[-1]
    for i, (typ, params) in enumerate(steps):
        nodes[f"n{i}"] = {"type": typ, "inputs": {"geo": prev}, "params": params}
        prev = f"n{i}"
    return cook(nodes)


HELIX = {"h": {"type": "helix", "params": {"radius": 1, "height": 2, "turns": 2, "points": 80}}}
LINE = {"l": {"type": "line", "params": {"length": 4, "points": 5}}}


def mixed():
    """A quad and an open 3-point curve sharing nothing."""
    P = np.array([[0, 0, 0], [1, 0, 0], [1, 1, 0], [0, 1, 0], [3, 0, 0], [4, 0, 0], [5, 1, 0]], np.float32)
    g = Geometry.from_arrays(P, [0, 1, 2, 3, 4, 5, 6], [0, 4, 7], closed=[True, False])
    g.prim["id"] = np.array([10, 20], np.int32)
    g.vertex["k"] = np.arange(7, dtype=np.float32)
    return g


class TopologyTests(unittest.TestCase):
    def test_closed_flag_basics(self):
        g = mixed()
        self.assertTrue(g.topology.has_open)
        self.assertEqual(g.topology.closed_mask().tolist(), [True, False])
        allc = Geometry.from_arrays(g.P, [0, 1, 2, 3], [0, 4], closed=[True])
        self.assertIsNone(allc.topology.closed)                      # canonical: all closed -> None
        self.assertNotEqual(g.topology.digest, Geometry.from_arrays(g.P, [0, 1, 2, 3, 4, 5, 6], [0, 4, 7]).topology.digest)
        e = topo.edges(g)
        self.assertEqual(len(e.points), 4 + 2)                        # no closing edge on the curve
        self.assertEqual(int((e.of_vertex == -1).sum()), 1)
        self.assertFalse(topo.boundary_points(g)[4:].any())           # curve points are not polygon boundary

    def test_flags_survive_nodes(self):
        g = chain(("transform", {"translate": [0, 0, 1]}), ("sort", {"domain": "primitives", "key": "reverse"}),
                  ("blast", {"group": "0", "domain": "points"}), ("divide", {}), src=LINE)
        self.assertTrue(g.topology.has_open)
        self.assertEqual(g.topology.prim_sizes().tolist(), [4])          # deleting an end point shortens the curve
        g = chain(("blast", {"group": "2", "domain": "points"}), src=LINE)
        self.assertEqual(g.topology.prim_sizes().tolist(), [2, 2])       # a middle point splits it
        g = chain(("blast", {"group": "1 3", "domain": "points"}), src=LINE)
        self.assertEqual(g.nprims, 0)                                     # runs of one point are dropped
        m = cook({"l": LINE["l"], "b": {"type": "box"}, "m": {"type": "merge", "inputs": {"geos": ["b", "l"]}},
                  "d": {"type": "divide", "inputs": {"geo": "m"}}})
        self.assertEqual(m.topology.closed_mask().tolist(), [True] * 12 + [False])
        c = cook({"l": LINE["l"], "p": {"type": "grid", "params": {"columns": 2, "rows": 2}},
                  "copy": {"type": "copy_to_points", "inputs": {"source": "l", "points": "p"}}})
        self.assertEqual(c.topology.closed_mask().tolist(), [False] * 4)

    def test_polygon_only_nodes_say_so(self):
        for typ in ("subdivide", "polyextrude", "clip", "polyfill"):
            with self.assertRaises(NodeError) as cm:
                chain((typ, {}), src=LINE)
            self.assertIn("open curve", str(cm.exception))
        g = chain(("wrangle", {"snippet": "i@isclosed = @closed", "run_over": "primitives"}), src=LINE)
        self.assertEqual(g.prim["isclosed"].tolist(), [0])
        g = cook({"l": LINE["l"], "b": {"type": "box"}, "m": {"type": "merge", "inputs": {"geos": ["b", "l"]}},
                  "x": {"type": "blast", "inputs": {"geo": "m"}, "params": {"group": "@closed == 0"}},
                  "s": {"type": "subdivide", "inputs": {"geo": "x"}}})
        self.assertEqual(g.nprims, 24)

    def test_link_round_trip(self):
        d = DiffState()
        h, arrays = d.build(mixed().freeze(), 1)
        self.assertEqual(arrays[TOPO_PRIM_CLOSED].tolist(), [1, 0])
        m = Mirror()
        m.apply(GeoUpdate.from_message(h, arrays))
        self.assertEqual(m.prim_closed.tolist(), [True, False])
        same_arrays = Geometry.from_arrays(mixed().P, [0, 1, 2, 3, 4, 5, 6], [0, 4, 7])   # curve became a polygon
        h, arrays = d.build(same_arrays.freeze(), 2)
        self.assertTrue(h["topology_changed"])
        m.apply(GeoUpdate.from_message(h, arrays))
        self.assertIsNone(m.prim_closed)


class CurveNodeTests(unittest.TestCase):
    def test_line_helix_curve(self):
        g = cook(LINE)
        np.testing.assert_allclose(g.P[:, 2], [0, 1, 2, 3, 4])
        h = cook(HELIX)
        self.assertAlmostEqual(float(h.P[:, 2].max()), 2.0, places=5)
        np.testing.assert_allclose(np.linalg.norm(h.P[:, :2], axis=1), 1, atol=1e-5)
        c = cook({"c": {"type": "curve", "params": {"points": "0,0,0 1,0,0 1,1,0", "closed": True}}})
        self.assertFalse(c.topology.has_open)
        with self.assertRaises(NodeError):
            cook({"c": {"type": "curve", "params": {"points": "0,0 1"}}})

    def test_resample_and_carve(self):
        g = chain(("resample", {"length": 0.5}), src=LINE)
        self.assertEqual(g.npoints, 9)
        np.testing.assert_allclose(g.point["curveu"], np.linspace(0, 1, 9), atol=1e-6)
        g = chain(("resample", {"by": "count", "segments": 4}), src=HELIX)
        self.assertEqual(g.npoints, 5)
        # closed polygon outline: no duplicate end point
        sq = chain(("resample", {"length": 0.25}), src={"c": {"type": "curve", "params": {"points": "0,0,0 1,0,0 1,1,0 0,1,0", "closed": True}}})
        self.assertEqual(sq.npoints, 16)
        self.assertFalse(sq.topology.has_open)
        cv = chain(("carve", {"start": 0.25, "end": 0.6}), src=LINE)
        np.testing.assert_allclose(cv.P[:, 2], [1.0, 2.0, 2.4], atol=1e-6)        # ends cut, original point 2 kept

    def test_sweep_makes_closed_tubes_with_uvs(self):
        g = chain(("resample", {"length": 0.2}), ("sweep", {"radius": 0.1, "divisions": 6}), src=HELIX)
        self.assertTrue(np.all(topo.edges(g).count == 2))             # capped tube: closed
        self.assertFalse(g.topology.has_open)
        uv = g.vertex["uv"]
        self.assertTrue(uv.min() >= 0 and uv.max() <= 1)
        # every ring point is 'radius' from the curve (rotation-minimising frames keep the section round)
        src = chain(("resample", {"length": 0.2}), src=HELIX)
        from minihou import spatial
        _, d = spatial.nearest(src.P, g.P)
        self.assertTrue(np.all(d <= 0.1 + 1e-4))
        self.assertGreater(float(np.median(d)), 0.09)
        w = chain(("attribute_create", {"name": "width", "value": [3, 0, 0]}), ("sweep", {"radius": 0.1}), src=LINE)
        self.assertAlmostEqual(float(np.abs(w.P[:, 0]).max()), 0.3, places=4)

    def test_frames_do_not_flip(self):
        g = chain(("sweep", {"radius": 1.0, "divisions": 4, "caps": False}), src=HELIX)
        ring = g.P.reshape(-1, 4, 3)
        # the first corner of consecutive rings stays close (no sudden 90/180 degree flips)
        jumps = np.linalg.norm(np.diff(ring[:, 0], axis=0), axis=1)
        self.assertLess(float(jumps.max()), 0.6)

    def test_convert_line_polypath_and_wireframe(self):
        segs = chain(("convert_line", {"join": False}), src={"b": {"type": "box"}})
        self.assertEqual((segs.nprims, set(segs.topology.prim_sizes().tolist())), (12, {2}))
        grid = {"g": {"type": "grid", "params": {"columns": 4, "rows": 1, "size_x": 3, "size_y": 0}}}
        joined = chain(("convert_line", {"join": False}), ("polypath", {}), src={"l": LINE["l"]})
        self.assertEqual(joined.nprims, 1)
        line_pieces = chain(("convert_line", {"join": False}), ("polypath", {}), src={"h": HELIX["h"]})
        self.assertEqual((line_pieces.nprims, line_pieces.nvertices), (1, 80))
        wire = chain(("convert_line", {}), ("sweep", {"radius": 0.02}), src={"b": {"type": "box"}})
        self.assertEqual(wire.nprims, 12 * 8 + 24)
        _ = grid

    def test_blur_smooths_curves_keeping_ends(self):
        noisy = chain(("resample", {"length": 0.1}), ("noise", {"amplitude": [0.3, 0.3, 0], "mode": "vector", "frequency": 3}),
                      src=LINE)
        smooth = chain(("resample", {"length": 0.1}), ("noise", {"amplitude": [0.3, 0.3, 0], "mode": "vector", "frequency": 3}),
                       ("attribute_blur", {"iterations": 30}), src=LINE)
        np.testing.assert_allclose(smooth.P[[0, -1]], noisy.P[[0, -1]])
        self.assertLess(np.abs(smooth.P[:, 0]).mean(), np.abs(noisy.P[:, 0]).mean())

    def test_measure_length(self):
        g = chain(("measure", {"measure": "perimeter", "name": "len"}), src=LINE)
        self.assertAlmostEqual(float(g.prim["len"][0]), 4.0, places=5)


if __name__ == "__main__":
    unittest.main()
