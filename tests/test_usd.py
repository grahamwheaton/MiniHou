"""USD export and import (minihou.usd_io, the usd_import node, the Session export): exact round trips of MiniHou geometry, files
built by other tools, and the guards around them. Skipped when usd-core (pxr) is not installed."""
import os
import shutil
import tempfile
import unittest

import numpy as np

import minihou  # noqa: F401
from minihou import Geometry, usd_io
from tests.test_nodes import run

try:
    from pxr import Gf, Sdf, Usd, UsdGeom, Vt
except ImportError:                                   # pragma: no cover - optional dependency
    Usd = None

needs_usd = unittest.skipIf(Usd is None, "usd-core (pxr) not installed")


def mixed():
    """A quad, a polyline and loose points sharing one point table (points 3 is used by the quad and the curve)."""
    P = np.array([[0, 0, 0], [1, 0, 0], [1, 1, 0], [0, 1, 0], [2, 2, 1], [3, 2, 1], [5, 5, 5], [6, 6, 6]], np.float32)
    # prims: quad (closed), curve (open) 3-4-5, a triangle (closed) - the order is mixed on purpose
    vp = np.array([0, 1, 2, 3, 3, 4, 5, 1, 2, 4], np.int32)
    offs = np.array([0, 4, 7, 10], np.int32)
    g = Geometry.from_arrays(P, vp, offs, closed=np.array([True, False, True]))
    g.point["Cd"] = np.arange(24, dtype=np.float32).reshape(8, 3) / 24
    g.point["weight"] = np.arange(8, dtype=np.float32)
    g.point["group_hot"] = np.array([0, 1, 1, 0, 0, 1, 0, 0], np.int32)
    g.point["id"] = np.arange(8, dtype=np.int32) * 10
    g.vertex["uv"] = np.arange(20, dtype=np.float32).reshape(10, 2) / 20
    g.vertex["N"] = np.tile(np.array([0, 0, 1], np.float32), (10, 1))
    g.vertex["edge_group_cut"] = np.array([1, 0, 0, 1, 0, 0, 1, 1, 0, 0], np.int32)
    g.prim["tag"] = np.array([7, 8, 9], np.int32)
    g.prim["group_big"] = np.array([1, 0, 1], np.int32)
    g.prim["wide"] = np.arange(15, dtype=np.float32).reshape(3, 5)               # more than 4 components
    g.prim["flag"] = np.array([True, False, True])
    g.detail["note"] = "hello"
    g.detail["count"] = 3
    g.detail["report"] = {"links": 4, "broken": [1, 2]}
    return g.freeze()


def same(test, a, b):
    test.assertEqual((a.npoints, a.nvertices, a.nprims), (b.npoints, b.nvertices, b.nprims))
    np.testing.assert_allclose(a.P, b.P, atol=1e-6)
    np.testing.assert_array_equal(a.topology.vertex_point, b.topology.vertex_point)
    np.testing.assert_array_equal(a.topology.prim_offsets, b.topology.prim_offsets)
    np.testing.assert_array_equal(a.topology.closed_mask(), b.topology.closed_mask())
    for dom in ("point", "vertex", "prim"):
        ta, tb = a.table(dom), b.table(dom)
        test.assertEqual(sorted(ta.names()), sorted(tb.names()), dom)
        for n in ta.names():
            np.testing.assert_allclose(np.asarray(ta[n], np.float64), np.asarray(tb[n], np.float64), atol=1e-6, err_msg=f"{dom}.{n}")
            test.assertEqual(ta[n].shape, tb[n].shape, f"{dom}.{n}")


@needs_usd
class UsdRoundTripTests(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.mkdtemp(prefix="minihou_usd_")
        self.addCleanup(shutil.rmtree, self.dir, True)

    def path(self, name):
        return os.path.join(self.dir, name)

    def test_mixed_geometry_comes_back_exactly_in_every_format(self):
        g = mixed()
        for ext in (".usda", ".usdc", ".usd"):
            p = self.path("m" + ext)
            rep = usd_io.write_stage(p, {"out": g})
            self.assertEqual(rep.outputs["out"]["faces"], 2)
            self.assertEqual((rep.outputs["out"]["curves"], rep.outputs["out"]["points"]), (1, 2))
            back, info = usd_io.read_geometry(p)
            same(self, g, back)
            self.assertEqual(back.detail, g.detail)
            self.assertEqual(info.skipped, [])
            self.assertEqual(back.point["id"].dtype, np.int32)
            self.assertEqual(back.prim["flag"].dtype, np.bool_)

    def test_the_usd_file_looks_right_to_other_tools(self):
        p = self.path("m.usda")
        usd_io.write_stage(p, {"out": mixed()})
        stage = Usd.Stage.Open(p)
        self.assertEqual(UsdGeom.GetStageUpAxis(stage), "Z")
        self.assertEqual(UsdGeom.GetStageMetersPerUnit(stage), 1.0)
        self.assertEqual(stage.GetDefaultPrim().GetPath(), "/MiniHou")
        mesh = UsdGeom.Mesh(stage.GetPrimAtPath("/MiniHou/out/mesh"))
        self.assertEqual(mesh.GetSubdivisionSchemeAttr().Get(), "none")              # not a subdivision surface
        self.assertEqual(mesh.GetOrientationAttr().Get(), "rightHanded")
        self.assertEqual(list(mesh.GetFaceVertexCountsAttr().Get()), [4, 3])
        self.assertEqual(len(mesh.GetPointsAttr().Get()), 5)                          # only the points the faces use
        self.assertEqual(mesh.GetNormalsInterpolation(), "faceVarying")
        self.assertEqual(len(mesh.GetNormalsAttr().Get()), 7)
        api = UsdGeom.PrimvarsAPI(mesh.GetPrim())
        st = api.GetPrimvar("st")
        self.assertEqual((st.GetInterpolation(), str(st.GetTypeName())), ("faceVarying", "texCoord2f[]"))
        cd = api.GetPrimvar("displayColor")
        self.assertEqual((cd.GetInterpolation(), str(cd.GetTypeName())), ("vertex", "color3f[]"))
        self.assertEqual(api.GetPrimvar("tag").GetInterpolation(), "uniform")
        self.assertEqual(api.GetPrimvar("wide").GetElementSize(), 5)
        cur = UsdGeom.BasisCurves(stage.GetPrimAtPath("/MiniHou/out/curves"))
        self.assertEqual((str(cur.GetTypeAttr().Get()), str(cur.GetWrapAttr().Get())), ("linear", "nonperiodic"))
        self.assertEqual(list(cur.GetCurveVertexCountsAttr().Get()), [3])
        self.assertEqual(len(UsdGeom.Points(stage.GetPrimAtPath("/MiniHou/out/points")).GetPointsAttr().Get()), 2)

    def test_groups_are_primvars_and_geomsubsets(self):
        p = self.path("m.usda")
        usd_io.write_stage(p, {"out": mixed()})
        stage = Usd.Stage.Open(p)
        mesh = UsdGeom.Mesh(stage.GetPrimAtPath("/MiniHou/out/mesh"))
        subs = {s.GetPrim().GetCustomDataByKey("minihou:group"): s for s in UsdGeom.Subset.GetAllGeomSubsets(mesh)}
        self.assertEqual(set(subs), {"group_big", "group_hot"})
        self.assertEqual(str(subs["group_big"].GetElementTypeAttr().Get()), "face")
        self.assertEqual(list(subs["group_big"].GetIndicesAttr().Get()), [0, 1])      # quad and triangle are the mesh's faces 0, 1
        self.assertEqual(str(subs["group_hot"].GetElementTypeAttr().Get()), "point")
        back, _ = usd_io.read_geometry(p)
        np.testing.assert_array_equal(back.prim["group_big"], mixed().prim["group_big"])          # the primvar is the truth
        np.testing.assert_array_equal(back.vertex["edge_group_cut"], mixed().vertex["edge_group_cut"])

    def test_subsets_alone_become_groups_on_a_foreign_file(self):
        p = self.path("f.usda")
        stage = Usd.Stage.CreateNew(p)
        mesh = UsdGeom.Mesh.Define(stage, "/Quads")
        mesh.CreatePointsAttr([(0, 0, 0), (1, 0, 0), (1, 1, 0), (0, 1, 0), (2, 0, 0), (2, 1, 0)])
        mesh.CreateFaceVertexCountsAttr([4, 4])
        mesh.CreateFaceVertexIndicesAttr([0, 1, 2, 3, 1, 4, 5, 2])
        UsdGeom.Subset.CreateGeomSubset(mesh, "right", "face", Vt.IntArray([1]), "regions", "nonOverlapping")
        UsdGeom.Subset.CreateGeomSubset(mesh, "corner", "point", Vt.IntArray([0, 5]), "pts", "unrestricted")
        stage.GetRootLayer().Save()
        g, _ = usd_io.read_geometry(p)
        np.testing.assert_array_equal(g.prim["group_right"], [0, 1])
        np.testing.assert_array_equal(g.point["group_corner"], [1, 0, 0, 0, 0, 1])

    def test_a_box_keeps_outward_normals_through_a_round_trip(self):
        box = run("box", divisions=2)
        p = self.path("b.usdc")
        usd_io.write_stage(p, {"box": box})
        back, _ = usd_io.read_geometry(p)
        P = back.P.astype(np.float64)
        t = back.topology
        for i in range(back.nprims):
            v = P[t.vertex_point[t.prim_offsets[i]:t.prim_offsets[i + 1]]]
            n = np.cross(v[1] - v[0], v[2] - v[0])
            self.assertGreater(float(n @ (v.mean(0))), 0.0)                              # the face normal points away from the centre

    def test_names_that_are_not_valid_identifiers_survive(self):
        g = run("box").copy()
        g.point["my attr!"] = np.arange(g.npoints, dtype=np.float32)
        g.point["group_a b"] = (np.arange(g.npoints) % 2).astype(np.int32)
        g.vertex["weight"] = np.zeros(g.nvertices, np.float32)
        g.point["weight"] = np.ones(g.npoints, np.float32)                                # the same name in two tables
        g.freeze()
        p = self.path("n.usda")
        usd_io.write_stage(p, {"b": g})
        back, _ = usd_io.read_geometry(p)
        same(self, g, back)

    def test_empty_and_degenerate_geometry(self):
        e = Geometry.points_only(np.zeros((0, 3), np.float32)).freeze()
        p = self.path("e.usda")
        usd_io.write_stage(p, {"e": e})
        back, _ = usd_io.read_geometry(p)
        self.assertEqual(back.npoints, 0)
        # a 2-corner "face" and a 1-point "curve" cannot be USD prims: dropped and counted
        g = Geometry.from_arrays(np.zeros((6, 3), np.float32) + np.arange(6)[:, None], [0, 1, 2, 3, 4, 5], [0, 2, 3, 6],
                                 closed=[True, False, True])
        rep = usd_io.write_stage(self.path("d.usda"), {"d": g.freeze()})
        self.assertEqual(rep.outputs["d"]["dropped"], {"faces": 1, "curves": 1})
        self.assertEqual((rep.outputs["d"]["faces"], rep.outputs["d"]["curves"]), (1, 0))

    def test_overwrite_extension_and_no_half_files(self):
        p = self.path("x.usda")
        usd_io.write_stage(p, {"a": run("box")})
        with self.assertRaises(FileExistsError):
            usd_io.write_stage(p, {"a": run("box")})
        usd_io.write_stage(p, {"a": run("sphere")}, overwrite=True)
        with self.assertRaises(ValueError):
            usd_io.write_stage(self.path("x.obj"), {"a": run("box")})
        self.assertEqual(sorted(os.listdir(self.dir)), ["x.usda"])
        g = run("sphere")
        back, _ = usd_io.read_geometry(p)
        self.assertEqual(back.nprims, g.nprims)

    def test_the_graph_rides_along_and_comes_back(self):
        graph = {"version": 1, "nodes": {"b": {"type": "box"}}, "display": "b"}
        p = self.path("g.usdc")
        usd_io.write_stage(p, {"b": run("box")}, graph=graph)
        self.assertEqual(usd_io.read_graph(p), graph)
        usd_io.write_stage(self.path("h.usdc"), {"b": run("box")})
        self.assertIsNone(usd_io.read_graph(self.path("h.usdc")))

    def test_two_outputs_become_two_prims_and_read_back_together(self):
        p = self.path("two.usda")
        rep = usd_io.write_stage(p, {"main": run("box"), "second out!": run("sphere", rows=6, columns=8)})
        self.assertEqual(rep.outputs["second out!"]["prim"], "/MiniHou/second_out_")
        back, _ = usd_io.read_geometry(p)
        self.assertEqual(back.nprims, run("box").nprims + run("sphere", rows=6, columns=8).nprims)
        only, _ = usd_io.read_geometry(p, prim_path="/MiniHou/second_out_")
        self.assertEqual(only.nprims, run("sphere", rows=6, columns=8).nprims)
        with self.assertRaises(ValueError):
            usd_io.read_geometry(p, prim_path="/Nope")


@needs_usd
class UsdForeignFileTests(unittest.TestCase):
    """Files made by other tools: Y-up, indexed face-varying primvars, left-handed meshes, transforms, other prim types."""

    def setUp(self):
        self.dir = tempfile.mkdtemp(prefix="minihou_usd_")
        self.addCleanup(shutil.rmtree, self.dir, True)

    def stage(self, name="f.usda", up="Y"):
        p = os.path.join(self.dir, name)
        s = Usd.Stage.CreateNew(p)
        UsdGeom.SetStageUpAxis(s, up)
        return p, s

    def test_y_up_indexed_st_left_handed_and_a_parent_transform(self):
        p, s = self.stage()
        UsdGeom.SetStageMetersPerUnit(s, 0.01)                                           # centimetres
        top = UsdGeom.Xform.Define(s, "/Top")
        top.AddTranslateOp().Set((100.0, 0.0, 0.0))
        mesh = UsdGeom.Mesh.Define(s, "/Top/Quad")
        mesh.CreatePointsAttr([(0, 0, 0), (0, 0, -100), (100, 0, -100), (100, 0, 0)])    # in the XZ plane (Y up); left handed: faces +Y
        mesh.CreateFaceVertexCountsAttr([4])
        mesh.CreateFaceVertexIndicesAttr([0, 1, 2, 3])
        mesh.CreateOrientationAttr("leftHanded")
        api = UsdGeom.PrimvarsAPI(mesh.GetPrim())
        pv = api.CreatePrimvar("st", Sdf.ValueTypeNames.TexCoord2fArray, "faceVarying")
        pv.Set(Vt.Vec2fArray([(0, 0), (1, 0), (1, 1), (0, 1)]))                         # two distinct values, indexed
        pv.SetIndices(Vt.IntArray([0, 1, 2, 3]))
        mesh.CreateNormalsAttr([(0, 1, 0)] * 4)
        mesh.SetNormalsInterpolation("faceVarying")
        s.GetRootLayer().Save()
        g, rep = usd_io.read_geometry(p)
        self.assertEqual(rep.up_axis, "Y")
        # world: translated +100 cm in X, then cm -> m, Y-up -> Z-up: (x, y, z) -> (x, -z, y)
        np.testing.assert_allclose(sorted(map(tuple, g.P.round(4).tolist())), sorted([(1, 0, 0), (2, 0, 0), (2, 1, 0), (1, 1, 0)]), atol=1e-5)
        self.assertEqual(g.nprims, 1)
        self.assertEqual(g.vertex["uv"].shape, (4, 2))
        np.testing.assert_allclose(g.vertex["N"], [[0, 0, 1]] * 4, atol=1e-5)               # +Y became +Z, normals follow
        # left handed: the winding was reversed, so the face now faces +Z like its normals
        v = g.P[g.topology.vertex_point[:4]].astype(np.float64)
        self.assertGreater(float(np.cross(v[1] - v[0], v[2] - v[0])[2]), 0.0)

    def test_keep_up_axis_and_units_are_options(self):
        p, s = self.stage()
        UsdGeom.SetStageMetersPerUnit(s, 0.01)
        mesh = UsdGeom.Mesh.Define(s, "/M")
        mesh.CreatePointsAttr([(0, 0, 0), (100, 0, 0), (0, 100, 0)])
        mesh.CreateFaceVertexCountsAttr([3])
        mesh.CreateFaceVertexIndicesAttr([0, 1, 2])
        s.GetRootLayer().Save()
        g, _ = usd_io.read_geometry(p, up_axis="keep", apply_units=False)
        np.testing.assert_allclose(g.P, [[0, 0, 0], [100, 0, 0], [0, 100, 0]])

    def test_mirroring_transform_flips_the_winding_back(self):
        p, s = self.stage(up="Z")
        x = UsdGeom.Xform.Define(s, "/X")
        x.AddScaleOp().Set((-1.0, 1.0, 1.0))
        mesh = UsdGeom.Mesh.Define(s, "/X/T")
        mesh.CreatePointsAttr([(1, 0, 0), (2, 0, 0), (1, 1, 0)])
        mesh.CreateFaceVertexCountsAttr([3])
        mesh.CreateFaceVertexIndicesAttr([0, 1, 2])
        s.GetRootLayer().Save()
        g, _ = usd_io.read_geometry(p)
        v = g.P[g.topology.vertex_point[:3]].astype(np.float64)
        self.assertLess(float(v[:, 0].max()), 0.0)
        self.assertGreater(float(np.cross(v[1] - v[0], v[2] - v[0])[2]), 0.0)               # still counter-clockwise from +Z

    def test_curves_points_and_unsupported_prims(self):
        p, s = self.stage(up="Z")
        cur = UsdGeom.BasisCurves.Define(s, "/C")
        cur.CreateTypeAttr("cubic")
        cur.CreateBasisAttr("bezier")
        cur.CreateCurveVertexCountsAttr([4])
        cur.CreatePointsAttr([(0, 0, 0), (1, 0, 0), (2, 1, 0), (3, 1, 0)])
        pts = UsdGeom.Points.Define(s, "/P")
        pts.CreatePointsAttr([(5, 5, 5), (6, 6, 6)])
        UsdGeom.Cube.Define(s, "/Cube")
        UsdGeom.PointInstancer.Define(s, "/Inst")
        s.GetRootLayer().Save()
        g, rep = usd_io.read_geometry(p)
        self.assertEqual((g.npoints, g.nprims), (6, 1))
        self.assertFalse(g.topology.closed_mask().any())
        self.assertTrue(any("Cube" in x for x in rep.skipped))
        self.assertTrue(any("control polygon" in n for n in rep.notes))

    def test_guide_and_proxy_purposes_are_left_out(self):
        p, s = self.stage(up="Z")
        for name, purpose in (("Render", "render"), ("Guide", "guide"), ("Proxy", "proxy")):
            m = UsdGeom.Mesh.Define(s, "/" + name)
            m.CreatePointsAttr([(0, 0, 0), (1, 0, 0), (0, 1, 0)])
            m.CreateFaceVertexCountsAttr([3])
            m.CreateFaceVertexIndicesAttr([0, 1, 2])
            m.CreatePurposeAttr(purpose)
        s.GetRootLayer().Save()
        g, rep = usd_io.read_geometry(p)
        self.assertEqual(g.nprims, 1)
        self.assertEqual(len(rep.skipped), 2)

    def test_time_samples_use_the_frame(self):
        p, s = self.stage(up="Z")
        m = UsdGeom.Mesh.Define(s, "/M")
        m.CreateFaceVertexCountsAttr([3])
        m.CreateFaceVertexIndicesAttr([0, 1, 2])
        pa = m.CreatePointsAttr()
        pa.Set([(0, 0, 0), (1, 0, 0), (0, 1, 0)], 1)
        pa.Set([(0, 0, 5), (1, 0, 5), (0, 1, 5)], 10)
        s.GetRootLayer().Save()
        self.assertAlmostEqual(float(usd_io.read_geometry(p, time=1)[0].P[:, 2].max()), 0.0)
        self.assertAlmostEqual(float(usd_io.read_geometry(p, time=10)[0].P[:, 2].max()), 5.0)

    def test_a_file_that_is_not_usd_is_a_clear_error(self):
        p = os.path.join(self.dir, "bad.usda")
        with open(p, "w", encoding="utf-8") as f:
            f.write("this is not usd")
        with self.assertRaises(ValueError):
            usd_io.read_geometry(p)

    def test_exported_files_are_structurally_valid_usd(self):
        """The usd-core wheel has no usdchecker, so check what it would: topology, primvar sizes against their interpolation,
        subset families, and that every prim has the schema attributes viewers need."""
        p = os.path.join(self.dir, "ok.usda")
        usd_io.write_stage(p, {"out": mixed()})
        stage = Usd.Stage.Open(p)
        mesh = UsdGeom.Mesh(stage.GetPrimAtPath("/MiniHou/out/mesh"))
        ok, why = UsdGeom.Mesh.ValidateTopology(mesh.GetFaceVertexIndicesAttr().Get(), mesh.GetFaceVertexCountsAttr().Get(),
                                                len(mesh.GetPointsAttr().Get()))
        self.assertTrue(ok, why)
        for prim in stage.Traverse():
            if not prim.IsA(UsdGeom.PointBased):
                continue
            pb = UsdGeom.PointBased(prim)
            npts = len(pb.GetPointsAttr().Get())
            if prim.IsA(UsdGeom.Mesh):
                counts = mesh.GetFaceVertexCountsAttr().Get()
                expect = {"vertex": npts, "varying": npts, "faceVarying": len(mesh.GetFaceVertexIndicesAttr().Get()), "uniform": len(counts),
                          "constant": 1}
            else:
                expect = {"vertex": npts, "varying": npts, "uniform": len(prim.GetAttribute("curveVertexCounts").Get())
                          if prim.IsA(UsdGeom.BasisCurves) else 1, "constant": 1}
            for pv in UsdGeom.PrimvarsAPI(prim).GetPrimvars():
                flat = pv.ComputeFlattened()
                if flat is None:                                         # the schema's displayColor / displayOpacity, unauthored
                    continue
                self.assertEqual(len(flat), expect[pv.GetInterpolation()] * max(pv.GetElementSize(), 1), (prim.GetPath(), pv.GetName()))
            self.assertTrue(pb.GetExtentAttr().HasAuthoredValue(), prim.GetPath())
        for family in ("minihou_face_groups", "minihou_point_groups"):
            subs = UsdGeom.Subset.GetGeomSubsets(mesh, "face" if "face" in family else "point", family)
            ok, why = UsdGeom.Subset.ValidateFamily(mesh, "face" if "face" in family else "point", family)
            self.assertTrue(subs and ok, (family, why))


def post(web, path, body):
    """(status, json) of a POST to the editor API."""
    import json
    import urllib.error
    import urllib.request
    req = urllib.request.Request(web.url.rstrip("/") + path, data=json.dumps(body).encode(), method="POST",
                                 headers={"Content-Type": "application/json"})
    try:
        with urllib.request.urlopen(req, timeout=20) as r:
            return r.status, json.loads(r.read())
    except urllib.error.HTTPError as e:
        return e.code, json.loads(e.read())


@needs_usd
class UsdNodeAndExportTests(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.mkdtemp(prefix="minihou_usd_")
        self.addCleanup(shutil.rmtree, self.dir, True)

    def session(self, nodes, display, **kw):
        from minihou import Graph
        from minihou.link.session import Session
        s = Session(Graph.from_dict({"version": 1, "nodes": nodes, "display": display}))
        s.start()
        self.addCleanup(s.stop)
        self.assertTrue(s.wait_idle(10))
        return s

    def test_usd_import_node_reads_a_file_and_recooks_when_it_changes(self):
        from minihou import Evaluator, Graph
        p = os.path.join(self.dir, "in.usda")
        usd_io.write_stage(p, {"a": run("box")})
        g = Graph.from_dict({"nodes": {"u": {"type": "usd_import", "params": {"file": p}}}, "display": "u"})
        ev = Evaluator()
        first = ev.cook(g)
        self.assertEqual(first.nprims, 6)
        self.assertEqual(first.detail["usd_prims"], 1)
        ev.cook(g)
        self.assertEqual(ev.last_stats.cooked, [])                           # unchanged file: cached
        usd_io.write_stage(p, {"a": run("sphere", rows=6, columns=8)}, overwrite=True)
        os.utime(p, ns=(2_000_000_000_000_000_000, 2_000_000_000_000_000_000))     # make sure the stamp differs on coarse clocks
        self.assertEqual(ev.cook(g).nprims, run("sphere", rows=6, columns=8).nprims)
        self.assertEqual(ev.last_stats.cooked, ["u"])                        # the edited file recooked it

    def test_usd_import_reports_skipped_prims_and_bad_input(self):
        from minihou import Evaluator, Graph, NodeError
        p = os.path.join(self.dir, "mixed.usda")
        s = Usd.Stage.CreateNew(p)
        m = UsdGeom.Mesh.Define(s, "/M")
        m.CreatePointsAttr([(0, 0, 0), (1, 0, 0), (0, 1, 0)])
        m.CreateFaceVertexCountsAttr([3])
        m.CreateFaceVertexIndicesAttr([0, 1, 2])
        UsdGeom.Sphere.Define(s, "/S")
        s.GetRootLayer().Save()

        def cook(**params):
            return Evaluator().cook(Graph.from_dict({"nodes": {"u": {"type": "usd_import", "params": params}}, "display": "u"}))
        out = cook(file=p)
        self.assertIn("/S (Sphere)", out.detail["usd_skipped"])
        for params, text in (({"file": ""}, "set 'file'"), ({"file": os.path.join(self.dir, "nope.usda")}, "no such file"),
                             ({"file": os.path.join(self.dir, "x.obj")}, "not a USD file"), ({"file": p, "prim_path": "/Zzz"}, "no prim")):
            with self.assertRaises(NodeError) as cm:
                cook(**params)
            self.assertIn(text, str(cm.exception))

    def test_export_is_full_size_even_at_a_proxy_level_and_leaves_the_session_alone(self):
        s = self.session({"g": {"type": "grid", "params": {"columns": 100, "rows": 100}}}, "g")
        s.set_proxy(level=0.1, link_tris=500)
        self.assertTrue(s.wait_idle(10))
        self.assertLess(s.latest_good.geo.nprims, 1000)                      # the session works on the small one
        p = os.path.join(self.dir, "full.usdc")
        rep = s.export_usd(p)
        self.assertEqual(rep["outputs"]["g"]["faces"], 99 * 99)
        back, _ = usd_io.read_geometry(p)
        self.assertEqual(back.nprims, 99 * 99)
        self.assertEqual(s.proxy_level, 0.1)
        self.assertLess(s.latest_good.geo.nprims, 1000)
        self.assertEqual(usd_io.read_graph(p)["display"], "g")               # the graph rides along
        with self.assertRaises(FileExistsError):
            s.export_usd(p)

    def test_extra_outputs_become_sibling_prims_and_a_single_node_can_be_exported(self):
        s = self.session({"b": {"type": "box"}, "s": {"type": "sphere", "params": {"rows": 6, "columns": 8}},
                          "o1": {"type": "output", "inputs": {"geo": "b"}},
                          "o2": {"type": "output", "params": {"object": "Second"}, "inputs": {"geo": "s"}}}, "o1")
        p = os.path.join(self.dir, "two.usda")
        rep = s.export_usd(p)
        self.assertEqual(sorted(rep["outputs"]), ["Second", "o1"])
        stage = Usd.Stage.Open(p)
        self.assertTrue(stage.GetPrimAtPath("/MiniHou/o1/mesh") and stage.GetPrimAtPath("/MiniHou/Second/mesh"))
        one = os.path.join(self.dir, "one.usda")
        rep = s.export_usd(one, node="s")
        self.assertEqual(list(rep["outputs"]), ["s"])
        with self.assertRaises(minihou.GraphError):
            s.export_usd(os.path.join(self.dir, "z.usda"), node="nope")

    def test_http_export_checks_paths_and_overwrite(self):
        from minihou.web import WebServer
        s = self.session({"b": {"type": "box"}}, "b")
        web = WebServer(s, "127.0.0.1", 0, root=self.dir)
        web.start()
        self.addCleanup(web.stop)
        code, r = post(web, "/api/export_usd", {"path": "out/box.usdc"})
        self.assertEqual((code, r["outputs"]["b"]["faces"]), (200, 6))
        self.assertTrue(os.path.isfile(os.path.join(self.dir, "out", "box.usdc")))
        code, r = post(web, "/api/export_usd", {"path": "out/box.usdc"})
        self.assertEqual(code, 409)
        self.assertIn("overwrite", r["error"])
        self.assertEqual(post(web, "/api/export_usd", {"path": "out/box.usdc", "overwrite": True})[0], 200)
        self.assertEqual(post(web, "/api/export_usd", {"path": "../escape.usda"})[0], 403)
        self.assertEqual(post(web, "/api/export_usd", {"path": "notes.txt"})[0], 400)
        self.assertEqual(post(web, "/api/export_usd", {"path": "x.usda", "node": "nope"})[0], 400)
        self.assertEqual(post(web, "/api/export_usd", {"path": "x.usda", "node": 5})[0], 400)
        self.assertFalse(os.path.exists(os.path.join(self.dir, "x.usda")))

    def test_what_the_graph_made_comes_back_through_usd_import(self):
        s = self.session({"g": {"type": "grid", "params": {"columns": 8, "rows": 8}},
                          "n": {"type": "noise", "params": {"amplitude": [0, 0, 1]}, "inputs": {"geo": "g"}}}, "n")
        p = os.path.join(self.dir, "grid.usdc")
        s.export_usd(p)
        from minihou import Evaluator, Graph
        got = Evaluator().cook(Graph.from_dict({"nodes": {"u": {"type": "usd_import", "params": {"file": p}}}, "display": "u"}))
        want = s.latest_good.geo
        np.testing.assert_allclose(got.P, want.P, atol=1e-6)
        np.testing.assert_array_equal(got.topology.vertex_point, want.topology.vertex_point)
        self.assertIn("uv", got.point)


class UsdWithoutPxrTests(unittest.TestCase):
    def test_nodes_still_register_and_fail_only_when_used(self):
        import sys
        import unittest.mock as mock
        from minihou import Evaluator, Graph, NodeError
        from minihou.sdk import REGISTRY
        self.assertIn("usd_import", REGISTRY)
        with mock.patch.dict(sys.modules, {"pxr": None}):
            with tempfile.TemporaryDirectory() as d:
                p = os.path.join(d, "a.usda")
                with open(p, "w", encoding="utf-8") as f:
                    f.write("#usda 1.0\n")
                g = Graph.from_dict({"nodes": {"u": {"type": "usd_import", "params": {"file": p}}}, "display": "u"})
                with self.assertRaises(NodeError) as cm:
                    Evaluator().cook(g)
                self.assertIn("pip install usd-core", str(cm.exception))


class UsdMissingTests(unittest.TestCase):
    def test_a_missing_pxr_is_a_clear_error(self):
        if Usd is not None:
            import sys
            import unittest.mock as mock
            with mock.patch.dict(sys.modules, {"pxr": None}):
                with self.assertRaises(usd_io.UsdUnavailable):
                    usd_io.pxr()
        else:
            with self.assertRaises(usd_io.UsdUnavailable):
                usd_io.pxr()


if __name__ == "__main__":
    unittest.main()
