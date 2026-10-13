"""Materials in USD: names written as Material prims and face bindings, read back from MiniHou files exactly and from other tools'
files by their bindings, and shading networks kept by reference. Skipped when usd-core (pxr) is not installed."""
import os
import tempfile
import unittest

import numpy as np

import minihou  # noqa: F401
from minihou import Evaluator, Geometry, Graph, usd_io
from minihou import materials as M
from tests.test_materials import boxes, cook, names_per_face

try:
    from pxr import Gf, Sdf, Usd, UsdGeom, UsdShade, Vt
except ImportError:                                   # pragma: no cover - optional dependency
    Usd = None

needs_usd = unittest.skipIf(Usd is None, "usd-core (pxr) not installed")


def quad_strip(n=4):
    """n quads in a row."""
    P = np.array([[x, y, 0] for x in range(n + 1) for y in (0, 1)], np.float32)
    vp = np.array([v for i in range(n) for v in (2 * i, 2 * i + 2, 2 * i + 3, 2 * i + 1)], np.int32)
    return Geometry.from_arrays(P, vp, np.arange(0, 4 * n + 1, 4, dtype=np.int32))


def foreign_file(path, networks=True):
    """A file as another tool would write it: a 4-quad mesh whose faces are bound to two materials by subsets (face 3 unbound), a
    second mesh bound directly, and a material with a shading network beyond a preview surface."""
    stage = Usd.Stage.CreateNew(path)
    UsdGeom.SetStageUpAxis(stage, UsdGeom.Tokens.z)
    root = UsdGeom.Xform.Define(stage, "/World")
    stage.SetDefaultPrim(root.GetPrim())
    UsdGeom.Scope.Define(stage, "/World/Materials")

    def material(name, color, extra=False):
        mat = UsdShade.Material.Define(stage, f"/World/Materials/{name}")
        sh = UsdShade.Shader.Define(stage, f"/World/Materials/{name}/Surface")
        sh.CreateIdAttr("UsdPreviewSurface")
        sh.CreateInput("diffuseColor", Sdf.ValueTypeNames.Color3f).Set(Gf.Vec3f(*color))
        mat.CreateSurfaceOutput().ConnectToSource(sh.ConnectableAPI(), "surface")
        if extra:                                                      # something this library never writes
            tex = UsdShade.Shader.Define(stage, f"/World/Materials/{name}/Noise")
            tex.CreateIdAttr("UsdPrimvarReader_float2")
            tex.GetPrim().SetCustomDataByKey("test:marker", "keep me")
        return mat
    red, blue = material("Red", (1, 0, 0), extra=networks), material("Blue", (0, 0, 1))
    green = material("Green", (0, 1, 0))
    g = quad_strip(4)
    mesh = UsdGeom.Mesh.Define(stage, "/World/Wall")
    P, vp = g.P, g.topology.vertex_point
    mesh.CreatePointsAttr().Set(Vt.Vec3fArray.FromNumpy(np.ascontiguousarray(P)))
    mesh.CreateFaceVertexCountsAttr().Set(Vt.IntArray.FromNumpy(np.full(4, 4, np.int32)))
    mesh.CreateFaceVertexIndicesAttr().Set(Vt.IntArray.FromNumpy(vp))
    api = UsdShade.MaterialBindingAPI.Apply(mesh.GetPrim())
    s1 = UsdGeom.Subset.CreateGeomSubset(mesh, "red", "face", Vt.IntArray([0, 1]), "materialBind", UsdGeom.Tokens.nonOverlapping)
    s2 = UsdGeom.Subset.CreateGeomSubset(mesh, "blue", "face", Vt.IntArray([2]), "materialBind", UsdGeom.Tokens.nonOverlapping)
    UsdShade.MaterialBindingAPI.Apply(s1.GetPrim()).Bind(red)
    UsdShade.MaterialBindingAPI.Apply(s2.GetPrim()).Bind(blue)
    api.SetMaterialBindSubsetsFamilyType(UsdGeom.Tokens.nonOverlapping)
    floor = UsdGeom.Mesh.Define(stage, "/World/Floor")
    floor.CreatePointsAttr().Set(Vt.Vec3fArray([(0, 0, -1), (1, 0, -1), (1, 1, -1), (0, 1, -1)]))
    floor.CreateFaceVertexCountsAttr().Set(Vt.IntArray([4]))
    floor.CreateFaceVertexIndicesAttr().Set(Vt.IntArray([0, 1, 2, 3]))
    UsdShade.MaterialBindingAPI.Apply(floor.GetPrim()).Bind(green)
    stage.GetRootLayer().Save()


@needs_usd
class ExportTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)

    def path(self, name="m.usda"):
        return os.path.join(self.tmp.name, name)

    def test_one_material_on_every_face_binds_the_mesh_directly(self):
        geo = cook({"b": {"type": "box"}, "m": {"type": "set_material", "inputs": {"geo": "b"}, "params": {"material": "Wood Planks"}}}, "m")
        rep = usd_io.write_stage(self.path(), {"out": geo})
        self.assertEqual(rep.outputs["out"]["materials"], ["Wood Planks"])
        stage = Usd.Stage.Open(self.path())
        mat = UsdShade.MaterialBindingAPI(stage.GetPrimAtPath("/MiniHou/out/mesh")).ComputeBoundMaterial()[0]
        self.assertEqual(str(mat.GetPath()), "/MiniHou/Looks/Wood_Planks")                  # a valid identifier
        self.assertEqual(mat.GetPrim().GetCustomDataByKey("minihou:material"), "Wood Planks")   # the real name is kept
        self.assertEqual([c.GetName() for c in stage.GetPrimAtPath("/MiniHou/out/mesh").GetChildren()], [])

    def test_several_materials_bind_by_material_bind_subsets_and_unnamed_faces_stay_unbound(self):
        geo = cook(boxes("Wood", "Metal", None), "out")
        usd_io.write_stage(self.path(), {"out": geo})
        stage = Usd.Stage.Open(self.path())
        mesh = stage.GetPrimAtPath("/MiniHou/out/mesh")
        api = UsdShade.MaterialBindingAPI(mesh)
        got = {}
        for sub in api.GetMaterialBindSubsets():
            mat = UsdShade.MaterialBindingAPI(sub.GetPrim()).ComputeBoundMaterial()[0]
            got[mat.GetPrim().GetName()] = list(sub.GetIndicesAttr().Get())
        self.assertEqual(got, {"Wood": list(range(0, 6)), "Metal": list(range(6, 12))})        # faces 12..17 have no material of ours
        self.assertEqual(UsdGeom.Subset.GetFamilyType(UsdGeom.Mesh(mesh), "materialBind"), UsdGeom.Tokens.nonOverlapping)

    def test_the_preview_colour_is_the_mean_cd_of_the_faces(self):
        geo = cook({"b": {"type": "box"}, "m": {"type": "set_material", "inputs": {"geo": "b"}, "params": {"material": "Tint"}}}, "m").copy()
        geo.prim["Cd"] = np.tile(np.array([[0.2, 0.4, 0.6]], np.float32), (geo.nprims, 1))
        usd_io.write_stage(self.path(), {"out": geo})
        stage = Usd.Stage.Open(self.path())
        shader = UsdShade.Shader(stage.GetPrimAtPath("/MiniHou/Looks/Tint/PreviewSurface"))
        np.testing.assert_allclose(list(shader.GetInput("diffuseColor").Get()), [0.2, 0.4, 0.6], atol=1e-6)

    def test_no_names_no_looks(self):
        usd_io.write_stage(self.path(), {"out": cook({"b": {"type": "box"}}, "b")})
        self.assertFalse(Usd.Stage.Open(self.path()).GetPrimAtPath("/MiniHou/Looks").IsValid())

    def test_an_output_called_looks_does_not_clash(self):
        geo = cook({"b": {"type": "box"}, "m": {"type": "set_material", "inputs": {"geo": "b"}, "params": {"material": "A"}}}, "m")
        rep = usd_io.write_stage(self.path(), {"Looks": geo})
        self.assertNotEqual(rep.outputs["Looks"]["prim"], "/MiniHou/Looks")
        back, _ = usd_io.read_geometry(self.path())
        self.assertEqual(M.used(back), ["A"])


@needs_usd
class RoundTripTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.p = os.path.join(self.tmp.name, "m.usda")

    def test_a_minihou_file_gives_back_the_same_names_and_faces(self):
        geo = cook(boxes("Wood Planks", "Metal", None), "out")
        usd_io.write_stage(self.p, {"out": geo})
        back, _ = usd_io.read_geometry(self.p)
        self.assertEqual(M.names(back), M.names(geo))
        self.assertEqual(names_per_face(back), names_per_face(geo))

    def test_the_binding_subsets_are_not_read_as_groups(self):
        geo = cook(boxes("Wood", "Metal"), "out")
        usd_io.write_stage(self.p, {"out": geo})
        back, _ = usd_io.read_geometry(self.p)
        self.assertEqual([n for n in back.prim.names() if n.startswith("group_")], [])

    def test_two_outputs_with_their_own_lists_come_back_whole(self):
        a = cook(boxes("Wood", "Metal"), "out")
        b = cook({"b": {"type": "box", "params": {"center": [9, 0, 0]}},
                  "m": {"type": "set_material", "inputs": {"geo": "b"}, "params": {"material": "Glass"}}}, "m")
        usd_io.write_stage(self.p, {"a": a, "b": b})
        back, _ = usd_io.read_geometry(self.p)
        self.assertEqual(sorted(set(names_per_face(back))), ["Glass", "Metal", "Wood"])
        self.assertEqual(sorted(names_per_face(back)), sorted(names_per_face(a) + names_per_face(b)))

    def test_through_the_graph_import_set_material_export(self):
        geo = cook(boxes("Wood", "Metal"), "out")
        src = os.path.join(self.tmp.name, "src.usda")
        usd_io.write_stage(src, {"out": geo})
        g = Graph.from_dict({"nodes": {"u": {"type": "usd_import", "params": {"file": src}},
                                       "m": {"type": "set_material", "inputs": {"geo": "u"},
                                             "params": {"group": "@primnum < 3", "material": "Glass"}}}, "display": "m"})
        out = Evaluator().cook(g)
        faces = names_per_face(out)
        self.assertEqual(faces[:3], ["Glass"] * 3)
        self.assertEqual(faces[6:], ["Metal"] * 6)


@needs_usd
class ForeignFileTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.src = os.path.join(self.tmp.name, "scene.usda")
        foreign_file(self.src)

    def test_bindings_become_names_and_face_indices(self):
        geo, rep = usd_io.read_geometry(self.src)
        lst = M.names(geo)
        self.assertEqual(lst[0], "")
        self.assertEqual(sorted(lst[1:]), ["Blue", "Green", "Red"])
        faces = names_per_face(geo)
        self.assertEqual(sorted(faces), ["", "Blue", "Green", "Red", "Red"])         # wall: Red Red Blue (face 3 unbound); floor: Green
        self.assertEqual([n for n in geo.prim.names() if n.startswith("group_")], [])

    def test_faces_take_their_materials_base_colour_when_the_file_has_no_colour(self):
        geo, rep = usd_io.read_geometry(self.src)
        by_name = {}
        for n, c in zip(names_per_face(geo), geo.prim["Cd"]):
            by_name.setdefault(n, []).append(tuple(np.round(c, 3)))
        self.assertEqual(set(by_name["Red"]), {(1.0, 0.0, 0.0)})
        self.assertEqual(set(by_name["Blue"]), {(0.0, 0.0, 1.0)})
        self.assertEqual(set(by_name["Green"]), {(0.0, 1.0, 0.0)})
        self.assertTrue(any("coloured from their materials" in n for n in rep.notes))

    def test_a_colour_the_file_has_is_not_replaced(self):
        stage = Usd.Stage.Open(self.src)
        wall = UsdGeom.Mesh(stage.GetPrimAtPath("/World/Wall"))
        UsdGeom.Gprim(wall).CreateDisplayColorPrimvar(UsdGeom.Tokens.uniform).Set(Vt.Vec3fArray([(0.5, 0.5, 0.5)] * 4))
        stage.GetRootLayer().Save()
        geo, _ = usd_io.read_geometry(self.src, prim_path="/World/Wall")
        np.testing.assert_allclose(geo.prim["Cd"], 0.5)

    def test_where_each_material_came_from_is_remembered(self):
        geo, _ = usd_io.read_geometry(self.src)
        src = geo.detail["usd_materials"]
        self.assertEqual(src["Red"]["prim"], "/World/Materials/Red")
        self.assertEqual(os.path.realpath(src["Red"]["file"]), os.path.realpath(self.src))

    def test_export_references_the_original_material_so_its_network_comes_through(self):
        geo, _ = usd_io.read_geometry(self.src)
        out = os.path.join(self.tmp.name, "out", "again.usda")
        usd_io.write_stage(out, {"out": geo})
        stage = Usd.Stage.Open(out)
        red = stage.GetPrimAtPath("/MiniHou/Looks/Red")
        self.assertTrue(red.HasAuthoredReferences())
        noise = stage.GetPrimAtPath("/MiniHou/Looks/Red/Noise")                       # a child only the original has
        self.assertTrue(noise.IsValid())
        self.assertEqual(noise.GetCustomDataByKey("test:marker"), "keep me")
        ref = red.GetPrimStack()[0].referenceList.GetAddedOrExplicitItems()[0]
        self.assertFalse(os.path.isabs(ref.assetPath))                                 # relative: the pair of files moves together
        subs = UsdShade.MaterialBindingAPI(stage.GetPrimAtPath("/MiniHou/out/mesh")).GetMaterialBindSubsets()
        bound = {UsdShade.MaterialBindingAPI(s.GetPrim()).ComputeBoundMaterial()[0].GetPrim().GetName() for s in subs}
        self.assertEqual(bound, {"Red", "Blue", "Green"})
        back, _ = usd_io.read_geometry(out)                                           # and the new file reads back with the same faces
        self.assertEqual(sorted(names_per_face(back)), sorted(names_per_face(geo)))

    def test_a_vanished_source_falls_back_to_a_preview_surface(self):
        geo, _ = usd_io.read_geometry(self.src)
        os.remove(self.src)
        out = os.path.join(self.tmp.name, "again.usda")
        usd_io.write_stage(out, {"out": geo})
        stage = Usd.Stage.Open(out)
        self.assertTrue(stage.GetPrimAtPath("/MiniHou/Looks/Red/PreviewSurface").IsValid())

    def test_merging_two_imports_keeps_both_sources_and_their_names(self):
        a, _ = usd_io.read_geometry(self.src, prim_path="/World/Wall")
        b, _ = usd_io.read_geometry(self.src, prim_path="/World/Floor")
        from minihou.nodes.merge import merge
        out = merge([a, b])
        self.assertEqual(sorted(set(M.used(out))), ["Blue", "Green", "Red"])
        self.assertEqual(set(out.detail["usd_materials"]), {"Red", "Blue", "Green"})
        self.assertEqual(sorted(names_per_face(out)), sorted(names_per_face(a) + names_per_face(b)))


if __name__ == "__main__":
    unittest.main()
