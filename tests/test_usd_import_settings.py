"""The usd_import node's settings (Primitives patterns, purposes, path attribute, missing file, time offset and scale, Reload) and the
usd_export node's round trip with them. Skipped when usd-core (pxr) is not installed."""
import os
import shutil
import tempfile
import unittest

import numpy as np

import minihou  # noqa: F401
from minihou import Evaluator, Graph, actions, timectx, usd_io
from minihou.link.session import Session

try:
    from pxr import Gf, Usd, UsdGeom, Vt
except ImportError:                                   # pragma: no cover - optional dependency
    Usd = None

needs_usd = unittest.skipIf(Usd is None, "usd-core (pxr) not installed")


def cook_import(path, **params):
    g = Graph.from_dict({"nodes": {"imp": {"type": "usd_import", "params": dict(file=path, **params)}}, "display": "imp"})
    return Evaluator().cook(g)


@needs_usd
class ImportSettings(unittest.TestCase):
    def setUp(self):
        self.dir = os.path.realpath(tempfile.mkdtemp(prefix="minihou_usdset_"))
        self.addCleanup(shutil.rmtree, self.dir, True)
        self.path = os.path.join(self.dir, "scene.usda")
        stage = Usd.Stage.CreateNew(self.path)
        UsdGeom.Xform.Define(stage, "/World")
        for name, x, purpose in (("chair", 0.0, "default"), ("table", 5.0, "default"), ("lamp", 10.0, "default"), ("chair_proxy", 0.0, "proxy")):
            m = UsdGeom.Mesh.Define(stage, f"/World/{name}")
            m.CreatePointsAttr([(x, 0, 0), (x + 1, 0, 0), (x + 1, 1, 0), (x, 1, 0)])
            m.CreateFaceVertexCountsAttr([4])
            m.CreateFaceVertexIndicesAttr([0, 1, 2, 3])
            if purpose != "default":
                UsdGeom.Imageable(m).CreatePurposeAttr(purpose)
        stage.GetRootLayer().Save()

    def xs(self, geo):
        return sorted({float(v) for v in geo.P[:, 0]})

    def test_the_whole_file_by_default_without_the_proxy_purpose(self):
        g = cook_import(self.path)
        self.assertEqual(g.nprims, 3)
        self.assertIn("proxy purpose", g.detail["usd_skipped"])

    def test_primitives_patterns_include_exclude_and_wildcards(self):
        self.assertEqual(self.xs(cook_import(self.path, prim_path="/World/table")), [5.0, 6.0])                    # one prim: as before
        self.assertEqual(cook_import(self.path, prim_path="/World/chair /World/lamp").nprims, 2)
        self.assertEqual(cook_import(self.path, prim_path="/World/*").nprims, 3)
        self.assertEqual(cook_import(self.path, prim_path="/World/?a*").nprims, 2)                                 # table, lamp
        g = cook_import(self.path, prim_path="/World ^/World/table")
        self.assertEqual(self.xs(g), [0.0, 1.0, 10.0, 11.0])
        self.assertEqual(cook_import(self.path, prim_path="/World/c*").nprims, 1)
        with self.assertRaisesRegex(Exception, "no prim"):
            cook_import(self.path, prim_path="/World/missing")
        with self.assertRaisesRegex(Exception, "must start with /"):
            cook_import(self.path, prim_path="World/chair /x")

    def test_purposes(self):
        self.assertEqual(cook_import(self.path, purpose="proxy").nprims, 1)
        self.assertEqual(cook_import(self.path, purpose="default, proxy").nprims, 4)
        self.assertEqual(cook_import(self.path, purpose="all").nprims, 4)
        with self.assertRaisesRegex(Exception, "unknown purpose"):
            cook_import(self.path, purpose="fast")
        self.assertEqual(usd_io.parse_purposes(""), ("default", "render"))

    def test_the_path_attribute_says_which_prim_each_primitive_came_from(self):
        g = cook_import(self.path, path_attrib="usd_prim")
        paths = g.detail["usd_paths"]
        self.assertEqual(sorted(paths), ["/World/chair", "/World/lamp", "/World/table"])
        by_face = [paths[i] for i in g.prim["usd_prim"]]
        for face, name in enumerate(by_face):
            x = float(g.P[g.topology.vertex_point[g.topology.prim_offsets[face]], 0])
            self.assertEqual(x, {"/World/chair": 0.0, "/World/table": 5.0, "/World/lamp": 10.0}[name])
        self.assertNotIn("usd_prim", cook_import(self.path).prim)                                                  # off by default
        with self.assertRaisesRegex(Exception, "valid attribute name"):
            cook_import(self.path, path_attrib="bad name")

    def test_missing_file_error_or_empty(self):
        gone = os.path.join(self.dir, "gone.usd")
        with self.assertRaisesRegex(Exception, "no such file"):
            cook_import(gone)
        self.assertEqual(cook_import(gone, missing_file="empty").npoints, 0)
        self.assertEqual(cook_import("", missing_file="empty").npoints, 0)
        with self.assertRaisesRegex(Exception, "set 'file'"):
            cook_import("")

    def test_relative_paths_start_in_the_workspace(self):
        from minihou import paths
        paths.set_root(self.dir)
        self.addCleanup(paths.set_root, None)
        self.assertEqual(cook_import("scene.usda").nprims, 3)

    def test_time_offset_and_scale_map_the_frame_to_the_files_time(self):
        anim = os.path.join(self.dir, "anim.usda")
        stage = Usd.Stage.CreateNew(anim)
        m = UsdGeom.Mesh.Define(stage, "/m")
        m.CreateFaceVertexCountsAttr([3])
        m.CreateFaceVertexIndicesAttr([0, 1, 2])
        pts = m.CreatePointsAttr()
        pts.Set([(0, 0, 0), (1, 0, 0), (0, 1, 0)], 0)
        pts.Set([(10, 0, 0), (11, 0, 0), (10, 1, 0)], 10)
        stage.GetRootLayer().Save()

        def x_at(frame, **p):
            with timectx.at(frame, 24):
                return float(cook_import(anim, time="frame", **p).P[0, 0])
        self.assertAlmostEqual(x_at(5), 5.0, places=4)
        self.assertAlmostEqual(x_at(15, time_offset=10), 5.0, places=4)                  # the file's frame 0 shows at frame 10
        self.assertAlmostEqual(x_at(10, time_scale=2.0), 5.0, places=4)                  # twice as slow
        self.assertAlmostEqual(x_at(30, time_offset=10, time_scale=4.0), 5.0, places=4)
        with timectx.at(5, 24):
            self.assertAlmostEqual(float(cook_import(anim).P[0, 0]), 0.0, places=4)       # time=default ignores the frame

    def test_reload_button_bumps_the_counter_and_the_node_cooks_again(self):
        s = Session(Graph.from_dict({"nodes": {"imp": {"type": "usd_import", "params": {"file": self.path}}}, "display": "imp"}))
        s.start()
        self.addCleanup(s.stop)
        r = actions.run(s, "imp", "reload")
        self.assertIn("Reloaded", r["message"])
        self.assertEqual(s.graph.nodes["imp"].params["reload"], 1)
        actions.run(s, "imp", "reload")
        self.assertEqual(s.graph.nodes["imp"].params["reload"], 2)
        with self.assertRaises(Exception):
            actions.run(s, "imp", "save")


if __name__ == "__main__":
    unittest.main()
