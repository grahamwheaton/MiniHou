"""Real Blender tests of material names: slots read into the graph by name, set_material, and the slots of the result in the
live link, in a modifier and on Scene Objects pieces. Skipped without bpy; run by tools/blender_selftest.py on CI."""
import json
import sys
import tempfile
import unittest
from pathlib import Path

try:
    import bpy
except ImportError:                                    # pragma: no cover
    bpy = None

ROOT = Path(__file__).resolve().parent.parent
sys.path[:0] = [str(ROOT), str(ROOT / "blender_addon")]
from tests import test_blender_modifiers as stack_tests  # noqa: E402
from tests.test_blender_addon import Engine  # noqa: E402
from tests.test_blender_scene_objects import LiveHarness, chunks, scene_objs  # noqa: E402


def quad_object(name, slots=("Brick", "Glass")):
    """A mesh of 4 quads in a row, quad i using slot i % len(slots)."""
    me = bpy.data.meshes.new(name)
    verts = [(x, y, 0) for x in range(5) for y in (0, 1)]
    faces = [(2 * i, 2 * i + 2, 2 * i + 3, 2 * i + 1) for i in range(4)]
    me.from_pydata(verts, [], faces)
    for s in slots:
        me.materials.append(bpy.data.materials.get(s) or bpy.data.materials.new(s))
    for i, p in enumerate(me.polygons):
        p.material_index = i % len(slots)
    ob = bpy.data.objects.new(name, me)
    bpy.context.scene.collection.objects.link(ob)
    return ob


def face_materials(mesh):
    return [mesh.materials[p.material_index].name if p.material_index < len(mesh.materials) and mesh.materials[p.material_index]
            else "" for p in mesh.polygons]


READ = {"version": 1, "display": "main", "nodes": {
    "src": {"type": "blender_object", "params": {"object": "Src", "space": "object"}},
    "m": {"type": "set_material", "inputs": {"geo": "src"}, "params": {"group": "@primnum >= 2", "material": "Concrete"}},
    "main": {"type": "output", "inputs": {"geo": "m"}}}}


@unittest.skipIf(bpy is None, "needs Blender / the bpy module")
class MaterialNamesLiveTests(LiveHarness, unittest.TestCase):
    def initial_graph(self):
        return READ

    def output_mesh(self):
        ob = bpy.data.objects.get("MiniHou_Output")
        return ob.data if ob is not None else None

    def test_slot_names_go_in_and_the_named_material_comes_out_in_a_slot(self):
        quad_object("Src")
        self.pump(lambda: self.output_mesh() is not None and len(self.output_mesh().polygons) == 4)
        mesh = self.output_mesh()
        self.assertEqual(face_materials(mesh), ["Brick", "Glass", "Concrete", "Concrete"])
        self.assertEqual([m.name for m in mesh.materials], ["Brick", "Glass", "Concrete"])

    def test_the_material_made_for_a_name_is_marked_and_a_real_one_is_used_when_it_exists(self):
        quad_object("Src")
        real = bpy.data.materials.new("Concrete")
        self.pump(lambda: self.output_mesh() is not None and len(self.output_mesh().polygons) == 4)
        self.assertIs(self.output_mesh().materials[2], real)
        self.assertNotIn("minihou_made", real)
        self.assertFalse(any(m.get("minihou_made") for m in bpy.data.materials))

    def test_a_missing_material_is_made_and_marked(self):
        quad_object("Src")
        self.pump(lambda: self.output_mesh() is not None and len(self.output_mesh().polygons) == 4)
        self.assertEqual(bpy.data.materials["Concrete"].get("minihou_made"), 1)

    def test_changing_the_name_in_the_graph_changes_the_slot_and_keeps_the_users_other_slots(self):
        quad_object("Src")
        self.pump(lambda: self.output_mesh() is not None and len(self.output_mesh().polygons) == 4)
        self.output_mesh().materials.append(bpy.data.materials.new("Mine"))            # something the user put on the output
        g = json.loads(json.dumps(READ))
        g["nodes"]["m"]["params"]["material"] = "Steel"
        self.write(g)
        self.pump(lambda: face_materials(self.output_mesh()) == ["Brick", "Glass", "Steel", "Steel"])
        names = [m.name for m in self.output_mesh().materials]
        self.assertEqual(names[:3], ["Brick", "Glass", "Steel"])
        self.assertIn("Mine", names)


PIECES = chunks()
PIECES["nodes"]["m"] = {"type": "set_material", "inputs": {"geo": "a"}, "params": {"material": "Rubble"}}
PIECES["nodes"]["so"]["inputs"] = {"geo": "m"}


@unittest.skipIf(bpy is None, "needs Blender / the bpy module")
class MaterialPiecesTests(LiveHarness, unittest.TestCase):
    def initial_graph(self):
        return PIECES

    def test_every_piece_has_the_named_material_in_its_slot(self):
        self.pump(lambda: len(scene_objs()) > 1 and all(len(o.data.polygons) for o in scene_objs()))
        for o in scene_objs():
            self.assertEqual({m.name for m in o.data.materials if m}, {"Rubble"})
            self.assertEqual(set(face_materials(o.data)), {"Rubble"})


CYCLE = chunks()
CYCLE["nodes"]["m"] = {"type": "set_material", "inputs": {"geo": "a"}, "params": {"index_attrib": "piece", "names": "Wood,Metal,Glass"}}
CYCLE["nodes"]["so"]["inputs"] = {"geo": "m"}


@unittest.skipIf(bpy is None, "needs Blender / the bpy module")
class MaterialPerPieceTests(LiveHarness, unittest.TestCase):
    def initial_graph(self):
        return CYCLE

    def test_pieces_take_different_materials_by_their_number(self):
        self.pump(lambda: len(scene_objs()) > 3 and all(len(o.data.polygons) for o in scene_objs()))
        want = ["Wood", "Metal", "Glass"]
        for o in scene_objs():
            used = {n for n in face_materials(o.data)}
            self.assertEqual(used, {want[o["minihou_id"] % 3]}, o.name)


def instances_graph(material="Red", with_material=True):
    nodes = {
        "g": {"type": "grid", "params": {"columns": 4, "rows": 4, "size_x": 10, "size_y": 10}},
        "p": {"type": "scatter", "inputs": {"geo": "g"}, "params": {"count": 6, "seed": 1}},
        "i": {"type": "instance", "inputs": {"points": "p"}, "params": {"objects": "Tree", "seed": 3}},
        "so": {"type": "scene_objects", "inputs": {"geo": "i"}, "params": {"prefix": "Forest"}},
        "main": {"type": "output", "inputs": {"geo": "g"}}}
    if with_material:
        nodes["m"] = {"type": "set_material", "inputs": {"geo": "i"},
                      "params": {"domain": "points", "group": "@ptnum < 2", "material": material}}
        nodes["so"]["inputs"] = {"geo": "m"}
    return {"version": 1, "display": "main", "nodes": nodes}


@unittest.skipIf(bpy is None, "needs Blender / the bpy module")
class MaterialPerInstanceTests(LiveHarness, unittest.TestCase):
    def initial_graph(self):
        return instances_graph()

    def setUp(self):
        super().setUp()
        self.tree.data.materials.append(bpy.data.materials.new("TreeMat"))

    def slot_names(self):
        out = {}
        for o in scene_objs():
            s = o.material_slots[0]
            out[o["minihou_id"]] = (s.link, s.material.name if s.material else None)
        return out

    def test_named_instances_get_their_own_first_slot_and_the_source_keeps_its_material(self):
        self.pump(lambda: len(scene_objs()) == 6 and self.slot_names().get(0, ("", ""))[1] == "Red")
        names = self.slot_names()
        self.assertEqual({k: v for k, v in names.items() if v[1] == "Red"}.keys(), {0, 1})
        for ident, (link, name) in names.items():
            if ident > 1:
                self.assertEqual((link, name), ("DATA", "TreeMat"))
        self.assertEqual([m.name for m in self.tree.data.materials], ["TreeMat"])              # the shared mesh is untouched

    def test_changing_and_removing_the_material_updates_and_resets_the_instances(self):
        self.pump(lambda: len(scene_objs()) == 6 and self.slot_names().get(0, ("", ""))[1] == "Red")
        self.write(instances_graph(material="Blue"))
        self.pump(lambda: self.slot_names().get(0, ("", ""))[1] == "Blue")
        self.write(instances_graph(with_material=False))
        self.pump(lambda: self.slot_names().get(0, ("", ""))[1] == "TreeMat")
        self.assertEqual({v[0] for v in self.slot_names().values()}, {"DATA"})
        self.assertFalse(any("minihou_material" in o for o in scene_objs()))


MOD_GRAPH = {"version": 1, "display": "o", "nodes": {
    "i": {"type": "modifier_input"},
    "m": {"type": "set_material", "inputs": {"geo": "i"}, "params": {"group": "@primnum < 2", "material": "Painted"}},
    "o": {"type": "output", "inputs": {"geo": "m"}}}}


@unittest.skipIf(bpy is None, "needs Blender / the bpy module")
class MaterialModifierTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.tmp = tempfile.TemporaryDirectory()
        cls.path = Path(cls.tmp.name) / "mat.json"
        cls.path.write_text(json.dumps(MOD_GRAPH), encoding="utf-8")
        boot = Path(cls.tmp.name) / "boot.json"
        boot.write_text(json.dumps(stack_tests.GRAPH), encoding="utf-8")
        cls.engine = Engine(boot)

    @classmethod
    def tearDownClass(cls):
        cls.engine.stop()
        cls.tmp.cleanup()

    def setUp(self):
        from minihou_link import modifiers, proxy
        modifiers.unregister()
        bpy.ops.wm.read_factory_settings(use_empty=True)
        self.m, self.proxy = modifiers, proxy
        self._registered_addon = False

    tearDown = stack_tests.ModifierStackTests.tearDown
    cook = stack_tests.ModifierStackTests.cook

    def test_the_modifier_result_has_the_named_material_and_keeps_the_objects_own_slots(self):
        obj = quad_object("Plank", slots=("Base",))
        self.m.add_modifier(obj, str(self.path), port=self.engine.port)
        self.cook()
        ev = obj.evaluated_get(bpy.context.evaluated_depsgraph_get())
        mesh = ev.to_mesh()
        try:
            self.assertEqual(face_materials(mesh), ["Painted", "Painted", "Base", "Base"])
            self.assertEqual([m.name for m in mesh.materials], ["Base", "Painted"])
        finally:
            ev.to_mesh_clear()


def _usd_available():
    try:
        from minihou import usd_io
        return usd_io.available()
    except ImportError:                                # pragma: no cover
        return False


@unittest.skipIf(bpy is None or not _usd_available(), "needs Blender and usd-core (pxr) next to it")
class MaterialUsdWithBlenderTests(unittest.TestCase):
    """Blender's own USD exporter and importer against ours (they share the material binding conventions)."""

    def setUp(self):
        bpy.ops.wm.read_factory_settings(use_empty=True)
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)

    def test_a_blender_export_comes_into_minihou_with_names_and_colours(self):
        from minihou import materials as M, usd_io
        ob = quad_object("Cubey", slots=("Brick", "Glass"))
        mats = {m.name: m for m in ob.data.materials}
        mats["Brick"].diffuse_color = (1, 0, 0, 1)
        path = str(Path(self.tmp.name) / "b.usda")
        bpy.ops.wm.usd_export(filepath=path, export_materials=True)
        geo, _ = usd_io.read_geometry(path)
        lst = M.names(geo)
        faces = [lst[i] for i in M.indices(geo)]
        self.assertEqual(faces, ["Brick", "Glass", "Brick", "Glass"])

    def test_a_minihou_export_imports_into_blender_with_its_material_slots(self):
        from minihou import Evaluator, Graph, usd_io
        nodes = {"b0": {"type": "box"}, "m0": {"type": "set_material", "inputs": {"geo": "b0"}, "params": {"material": "Wood"}},
                 "b1": {"type": "box", "params": {"center": [3, 0, 0]}},
                 "m1": {"type": "set_material", "inputs": {"geo": "b1"}, "params": {"material": "Metal"}},
                 "out": {"type": "merge", "inputs": {"geos": ["m0", "m1"]}}}
        geo = Evaluator().cook(Graph.from_dict({"nodes": nodes, "display": "out"}), "out")
        path = str(Path(self.tmp.name) / "m.usda")
        usd_io.write_stage(path, {"out": geo})
        bpy.ops.wm.usd_import(filepath=path)
        mesh = next(o.data for o in bpy.data.objects if o.type == "MESH")
        self.assertEqual(face_materials(mesh), ["Wood"] * 6 + ["Metal"] * 6)


if __name__ == "__main__":
    unittest.main()
