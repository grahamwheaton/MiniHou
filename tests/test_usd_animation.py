"""USD export of a frame range: what changes between frames becomes time samples, and it reads back frame by frame."""
import os
import shutil
import tempfile
import unittest

import numpy as np

import minihou  # noqa: F401
from minihou import Graph, actions, paths, usd_io
from minihou.link.session import Session
from minihou.nodes.usd_nodes import export_frames
from tests.test_nodes import run

try:
    from pxr import Usd, UsdGeom
except ImportError:                                   # pragma: no cover - optional dependency
    Usd = None

needs_usd = unittest.skipIf(Usd is None, "usd-core (pxr) not installed")


def box(size=1.0):
    return run("box", size=(size, size, size))


@needs_usd
class AnimationTests(unittest.TestCase):
    def setUp(self):
        self.dir = os.path.realpath(tempfile.mkdtemp(prefix="minihou_anim_"))
        self.addCleanup(shutil.rmtree, self.dir, True)
        self.path = os.path.join(self.dir, "anim.usdc")

    def test_frames_that_move_points_write_time_samples_and_read_back_per_frame(self):
        def frames():
            for f in (1, 2, 3, 4):
                g = box().copy()
                g.point["P"] = g.P + np.array([f * 10.0, 0, 0], np.float32)
                yield float(f), {"crate": g}
        rep = usd_io.write_animation(self.path, frames(), fps=24)
        self.assertEqual(rep.to_dict()["frames"], [1.0, 4.0, 4])
        stage = Usd.Stage.Open(self.path)
        self.assertEqual((stage.GetStartTimeCode(), stage.GetEndTimeCode(), stage.GetTimeCodesPerSecond()), (1.0, 4.0, 24.0))
        mesh = stage.GetPrimAtPath("/MiniHou/crate/mesh")
        self.assertEqual(mesh.GetAttribute("points").GetNumTimeSamples(), 4)
        self.assertLessEqual(mesh.GetAttribute("faceVertexIndices").GetNumTimeSamples(), 2)           # unchanged: written once (and at the end)
        for f in (1, 2, 3, 4):
            geo, _ = usd_io.read_geometry(self.path, time=f)
            self.assertAlmostEqual(float(geo.P[:, 0].mean()), f * 10.0, places=4)
            self.assertEqual(geo.nprims, 6)
        geo, _ = usd_io.read_geometry(self.path, time=2.5)                                           # between samples: interpolated
        self.assertAlmostEqual(float(geo.P[:, 0].mean()), 25.0, places=4)

    def test_topology_and_which_primitives_exist_can_change_between_frames(self):
        from minihou.geometry import Geometry
        a = box()
        grid = run("grid", columns=4, rows=3)
        with_points = Geometry.from_arrays(np.vstack([a.P, np.array([[5, 5, 5], [6, 6, 6]], np.float32)]), a.topology.vertex_point,
                                           a.topology.prim_offsets)
        usd_io.write_animation(self.path, iter([(1.0, {"thing": a}), (2.0, {"thing": grid}), (3.0, {"thing": with_points})]), fps=24)
        g1, _ = usd_io.read_geometry(self.path, time=1)
        g2, _ = usd_io.read_geometry(self.path, time=2)
        g3, _ = usd_io.read_geometry(self.path, time=3)
        self.assertEqual((g1.npoints, g1.nprims), (8, 6))
        self.assertEqual((g2.npoints, g2.nprims), (12, 6))                                            # the 4 x 3 point grid
        self.assertEqual((g3.npoints, g3.nprims), (10, 6))                                            # the box with two loose points
        np.testing.assert_allclose(g1.P, a.P, atol=1e-6)
        np.testing.assert_allclose(g2.P, grid.P, atol=1e-6)
        stage = Usd.Stage.Open(self.path)
        pts = UsdGeom.Imageable(stage.GetPrimAtPath("/MiniHou/thing/points")).GetVisibilityAttr()
        self.assertEqual([pts.Get(f) for f in (1, 2, 3)], ["invisible", "invisible", "inherited"])      # it only exists from frame 3

    def test_an_attribute_that_a_frame_lacks_does_not_leak_into_it(self):
        a = box().copy()
        a.point["weight"] = np.arange(8, dtype=np.float32)
        b = box().copy()                                                                              # same size, no `weight`
        usd_io.write_animation(self.path, iter([(1.0, {"t": a}), (2.0, {"t": b}), (3.0, {"t": a})]), fps=24)
        self.assertIn("weight", usd_io.read_geometry(self.path, time=1)[0].point)
        self.assertNotIn("weight", usd_io.read_geometry(self.path, time=2)[0].point)
        np.testing.assert_allclose(usd_io.read_geometry(self.path, time=3)[0].point["weight"], np.arange(8))

    def test_the_export_node_writes_a_frame_range_of_a_time_dependent_graph(self):
        g = Graph.from_dict({"nodes": {"box": {"type": "box"},
                                       "w": {"type": "wrangle", "inputs": {"geo": "box"}, "params": {"snippet": "@P.x += @Frame * 10;"}},
                                       "out": {"type": "usd_export", "inputs": {"geo": "w"},
                                               "params": {"file": "out/anim.usdc", "frames": "range", "start": 2, "end": 6, "step": 2, "name": "crate"}}},
                             "display": "out"})
        paths.set_root(self.dir)
        self.addCleanup(paths.set_root, None)
        s = Session(g)
        s.start()
        self.addCleanup(s.stop)
        frame_before = s.frame
        r = actions.run(s, "out", "save")
        self.assertIn("over 3 frames (2-6)", r["message"])
        self.assertEqual(r["report"]["frames"], [2.0, 6.0, 3])
        self.assertEqual(s.frame, frame_before)                                                       # the timeline did not move
        target = os.path.join(self.dir, "out", "anim.usdc")
        for f in (2, 4, 6):
            geo, _ = usd_io.read_geometry(target, time=f)
            self.assertAlmostEqual(float(geo.P[:, 0].mean()), f * 10.0, places=3)

    def test_frame_lists_and_limits(self):
        p = dict(frames="range", start=1, end=10, step=4)
        self.assertEqual(export_frames(p), [1.0, 5.0, 9.0])
        self.assertIsNone(export_frames(dict(p, frames="current")))
        with self.assertRaisesRegex(ValueError, "empty"):
            export_frames(dict(p, end=0))
        with self.assertRaisesRegex(ValueError, "more than"):
            export_frames(dict(p, start=0, end=10**6, step=1))

    def test_a_single_frame_file_still_has_no_time_samples(self):
        usd_io.write_stage(self.path, {"crate": box()})
        stage = Usd.Stage.Open(self.path)
        self.assertEqual(stage.GetPrimAtPath("/MiniHou/crate/mesh").GetAttribute("points").GetNumTimeSamples(), 0)


if __name__ == "__main__":
    unittest.main()
