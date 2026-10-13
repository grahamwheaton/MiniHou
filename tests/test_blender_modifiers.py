"""Real Blender tests: external engine, evaluated stack meshes, isolated graph instances.

MINIHOU_ENGINE_PYTHON=<supported external Python> blender --background --factory-startup
    --python tests/test_blender_modifiers.py
"""
import json
import time
import sys
import tempfile
import unittest
from pathlib import Path

import numpy as np

try:
    import bpy
except ImportError:
    bpy = None

ROOT = Path(__file__).resolve().parent.parent
sys.path[:0] = [str(ROOT), str(ROOT / "blender_addon")]
from tests.test_blender_addon import Engine, evaluated_positions

GRAPH = {
    "version": 1,
    "nodes": {
        "i": {"type": "modifier_input"},
        "t": {"type": "transform", "inputs": {"geo": "i"}},
        "o": {"type": "output", "inputs": {"geo": "t"}},
    },
    "display": "o",
    "exposed": [
        {"name": "Scale", "node": "t", "param": "uniform_scale"},
        {"name": "Translate", "node": "t", "param": "translate"},
    ],
}


@unittest.skipIf(bpy is None, "requires real Blender")
class ModifierStackTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        if bpy is None:
            return
        cls.tmp = tempfile.TemporaryDirectory()
        cls.path = Path(cls.tmp.name) / "modifier.json"
        cls.path.write_text(json.dumps(GRAPH), encoding="utf-8")
        cls.engine = Engine(cls.path)

    @classmethod
    def tearDownClass(cls):
        if bpy is not None:
            cls.engine.stop()
            cls.tmp.cleanup()

    def setUp(self):
        from minihou_link import modifiers, proxy
        modifiers.unregister()
        bpy.ops.wm.read_factory_settings(use_empty=True)
        self.m, self.proxy = modifiers, proxy
        self._registered_addon = False
        self.path.write_text(json.dumps(GRAPH), encoding="utf-8")

    def tearDown(self):
        if self._registered_addon:
            import minihou_link
            minihou_link.unregister()
        else:
            self.m.unregister()

    def mesh(self, name="Source", x=1):
        mesh = bpy.data.meshes.new(name)
        mesh.from_pydata([(x, 0, 0)], [], [])
        obj = bpy.data.objects.new(name, mesh)
        bpy.context.scene.collection.objects.link(obj)
        return obj

    def add(self, obj, scale=1.0, translate=(0, 0, 0)):
        mod = self.m.add_modifier(obj, str(self.path), port=self.engine.port)
        self.set_params(mod, Scale=scale, Translate=translate)
        return mod

    def set_params(self, mod, **values):
        self.proxy.write_values(mod, mod.node_group, self.m.parameters(mod), values)

    def cook(self, force=False):
        count = self.m.pump(port=self.engine.port, force=force)
        self.assertEqual(self.m.last_error(), "")
        bpy.context.view_layer.update()
        return count

    def assert_x(self, obj, expected):
        np.testing.assert_allclose(evaluated_positions(obj)[:, 0], expected, atol=1e-5)

    def native_translate(self, obj, value):
        mod = obj.modifiers.new("Native GN Translate", "NODES")
        ng = bpy.data.node_groups.new("Native transform", "GeometryNodeTree")
        ng.interface.new_socket("Geometry", in_out="INPUT", socket_type="NodeSocketGeometry")
        ng.interface.new_socket("Geometry", in_out="OUTPUT", socket_type="NodeSocketGeometry")
        gin = ng.nodes.new("NodeGroupInput")
        gout = ng.nodes.new("NodeGroupOutput")
        trans = ng.nodes.new("GeometryNodeTransform")
        trans.inputs["Translation"].default_value = (value, 0, 0)
        ng.links.new(gin.outputs["Geometry"], trans.inputs["Geometry"])
        ng.links.new(trans.outputs["Geometry"], gout.inputs["Geometry"])
        mod.node_group = ng
        return mod, trans

    def test_repeat_block_after_native_and_chained_minihou_modifiers(self):
        path = ROOT / "examples/modifiers/repeat_transform.json"
        obj = self.mesh(x=1)
        material = bpy.data.materials.new("Repeat test material")
        obj.data.materials.append(material)
        self.native_translate(obj, 4)
        a = self.m.add_modifier(obj, str(path), port=self.engine.port)
        b = self.m.add_modifier(obj, str(path), port=self.engine.port)
        self.set_params(a, **{"Repeat Count": 3, "Step": [2, 0, 0]})
        self.set_params(b, **{"Repeat Count": 2, "Step": [1, 0, 0]})
        self.cook()
        self.assert_x(obj, [13])  # (initial 1 + native 4) + 3*2 + 2*1
        self.set_params(a, **{"Repeat Count": 0, "Step": [2, 0, 0]})
        self.cook()
        self.assert_x(obj, [7])
        self.assertEqual(obj.data.vertices[0].co.x, 1)
        evaluated = obj.evaluated_get(bpy.context.evaluated_depsgraph_get())
        self.assertEqual(evaluated.data.materials[0].original, material)

    def test_native_prefix_and_chained_independent_instances(self):
        obj = self.mesh()
        original = tuple(obj.data.vertices[0].co)
        _, native = self.native_translate(obj, 2)
        first = self.add(obj, scale=2)
        self.native_translate(obj, 1)
        second = self.add(obj, translate=(4, 0, 0))
        self.assertEqual(self.cook(), 2)
        self.assert_x(obj, [11])  # ((1+2)*2+1)+4
        self.assertNotEqual(self.m.instance_id(first), self.m.instance_id(second))
        self.assertEqual(tuple(obj.data.vertices[0].co), original)
        native.inputs["Translation"].default_value = (3, 0, 0)
        self.cook()
        self.assert_x(obj, [13])
        self.set_params(first, Scale=3)
        self.cook()
        self.assert_x(obj, [17])
        self.assertEqual(self.proxy.read_values(second, second.node_group, self.m.parameters(second))["Scale"], 1)
        self.assertEqual(self.cook(), 0)  # no engine/write feedback loop

    def test_reorder_changes_noncommutative_result(self):
        obj = self.mesh()
        first = self.add(obj, scale=2)
        second = self.add(obj, translate=(4, 0, 0))
        self.cook()
        self.assert_x(obj, [6])
        obj.modifiers.move(1, 0)
        self.cook()
        self.assert_x(obj, [10])
        self.assertEqual(self.cook(), 0)

    def test_object_and_modifier_duplication_isolates_ids_and_parameters(self):
        obj = self.mesh()
        first = self.add(obj, scale=2)
        self.cook()
        original_id = self.m.instance_id(first)
        duplicate = obj.copy()
        duplicate.name = "AAA Duplicate"  # scans before source; source ownership must still win
        bpy.context.scene.collection.objects.link(duplicate)
        second = duplicate.modifiers[0]
        self.cook()
        self.assertEqual(self.m.instance_id(first), original_id)
        self.assertNotEqual(self.m.instance_id(second), original_id)
        self.set_params(second, Scale=5)
        self.cook()
        self.assert_x(obj, [2])
        self.assert_x(duplicate, [5])
        copied = obj.modifiers.new("Duplicate MiniHou", "NODES")
        copied.node_group = first.node_group
        self.cook()
        self.assertEqual(self.m.instance_id(first), original_id)
        self.assertNotEqual(self.m.instance_id(copied), original_id)
        self.set_params(copied, Scale=3)
        self.cook()
        self.assert_x(obj, [6])
        self.assert_x(duplicate, [5])

    def test_save_reload_snapshot_and_cache_persist(self):
        import minihou_link
        if not hasattr(bpy.types.Scene, "minihou_graph_path"):
            minihou_link.register()
            self._registered_addon = True
        bpy.context.window_manager.minihou_port = self.engine.port
        self.assertEqual(bpy.context.scene.minihou_link_port, self.engine.port)
        obj = self.mesh()
        first = self.add(obj, scale=3)
        self.cook()
        ident = self.m.instance_id(first)
        blend = str(Path(self.tmp.name) / "roundtrip.blend")
        bpy.ops.wm.save_as_mainfile(filepath=blend)
        self.m.register()
        bpy.ops.wm.open_mainfile(filepath=blend)
        self.assertIsNone(self.m._client)  # persistent load handler released old instances
        obj = bpy.data.objects["Source"]
        self.assertEqual(self.m.instance_id(obj.modifiers[0]), ident)
        self.assert_x(obj, [3])
        for attempt in range(40):                      # on Windows the engine may be reading the file this very moment
            try:
                self.path.unlink()  # persisted graph survives unavailable original file
                break
            except PermissionError:
                if attempt == 39:
                    raise
                time.sleep(0.1)
        self.assertEqual(bpy.context.window_manager.minihou_port, self.engine.port)
        self.assertEqual(self.m.pump(force=True), 1)  # restored endpoint; no explicit host/port
        self.assertEqual(self.m.last_error(), "")
        self.assert_x(obj, [3])

    def test_removal_cleans_only_owned_cache(self):
        obj = self.mesh()
        mod = self.add(obj, scale=3)
        self.cook()
        cache_name = self.m._cache(mod).name
        group_name = mod.node_group.name
        ordinary = self.mesh("Do not remove", 4)
        obj.modifiers.remove(mod)
        self.cook()
        self.assertNotIn(cache_name, bpy.data.objects)
        self.assertNotIn(group_name, bpy.data.node_groups)
        self.assertIn(ordinary.name, bpy.data.objects)
        self.assert_x(obj, [1])

    def test_duplicate_in_another_scene_has_independent_identity(self):
        obj = self.mesh()
        first = self.add(obj, scale=2)
        self.cook()
        original_id = self.m.instance_id(first)
        original_scene = bpy.context.scene
        scene = bpy.data.scenes.new("Other Scene")
        duplicate = obj.copy()
        scene.collection.objects.link(duplicate)
        bpy.context.window.scene = scene
        self.cook()
        second = duplicate.modifiers[0]
        self.assertNotEqual(self.m.instance_id(second), original_id)
        self.assertEqual(self.m.instance_id(first), original_id)
        self.set_params(second, Scale=7)
        self.cook()
        self.assert_x(duplicate, [7])
        bpy.context.window.scene = original_scene
        self.cook()
        self.assert_x(obj, [2])

    def test_fractional_frame_reaches_engine_and_invalidates_cache(self):
        graph = {"nodes": {
            "i": {"type": "modifier_input"},
            "w": {"type": "wrangle", "inputs": {"geo": "i"},
                  "params": {"snippet": "@P.x = @Frame"}},
            "o": {"type": "output", "inputs": {"geo": "w"}},
        }, "display": "o"}
        path = Path(self.tmp.name) / "subframe.json"
        path.write_text(json.dumps(graph), encoding="utf-8")
        obj = self.mesh()
        self.m.add_modifier(obj, str(path), port=self.engine.port)
        bpy.context.scene.frame_set(5, subframe=0.25)
        self.assertEqual(self.cook(), 1)
        self.assert_x(obj, [5.25])
        bpy.context.scene.frame_set(5, subframe=0.75)
        self.assertEqual(self.cook(), 1)
        self.assert_x(obj, [5.75])
        self.assertEqual(self.cook(), 0)

    def test_native_material_slots_and_assignment_changes_roundtrip(self):
        bpy.ops.mesh.primitive_cube_add()
        obj = bpy.context.object
        base = bpy.data.materials.new("Source material")
        added = bpy.data.materials.new("Native assigned material")
        changed = bpy.data.materials.new("Changed native material")
        obj.data.materials.append(base)
        obj.data.materials.append(None)  # preserve explicitly empty material slots too
        native, _ = self.native_translate(obj, 0)
        ng = native.node_group
        gin = next(n for n in ng.nodes if n.bl_idname == "NodeGroupInput")
        gout = next(n for n in ng.nodes if n.bl_idname == "NodeGroupOutput")
        assign = ng.nodes.new("GeometryNodeSetMaterial")
        assign.inputs["Material"].default_value = added
        ng.links.new(gin.outputs["Geometry"], assign.inputs["Geometry"])
        ng.links.new(assign.outputs["Geometry"], gout.inputs["Geometry"])
        first = self.add(obj, scale=2)
        second = self.add(obj, translate=(1, 0, 0))
        self.assertEqual(self.cook(), 2)
        for mod in (first, second):
            mesh = self.m._cache(mod).data
            self.assertTrue(all(mesh.materials[p.material_index] == added for p in mesh.polygons))
            self.assertIn(added.name, mesh.materials)
        self.assertEqual(tuple(obj.data.materials), (base, None))
        # Geometry/index arrays stay identical: the material slot identities must invalidate caches.
        assign.inputs["Material"].default_value = changed
        self.assertEqual(self.cook(), 2)
        result = obj.evaluated_get(bpy.context.evaluated_depsgraph_get()).data
        self.assertTrue(all(result.materials[p.material_index].original == changed for p in result.polygons))
        self.assertEqual(self.cook(), 0)

    def test_reload_before_first_cook_keeps_passthrough_even_on_error(self):
        from unittest.mock import patch
        obj = self.mesh()
        self.native_translate(obj, 2)
        mod = self.add(obj, scale=2)
        bpy.context.view_layer.update()
        self.assert_x(obj, [3])
        self.m.reload_graph(mod, port=self.engine.port)
        bpy.context.view_layer.update()
        self.assert_x(obj, [3])
        with patch.object(self.m._client, "cook", side_effect=RuntimeError("first cook failed")):
            self.assertEqual(self.m.pump(port=self.engine.port), 0)
        self.assertIn("first cook failed", self.m.status(mod))
        self.assert_x(obj, [3])
        self.cook(force=True)
        self.assert_x(obj, [6])

    def test_native_topology_and_attributes_preserve_shared_source_mesh(self):
        bpy.ops.mesh.primitive_cube_add()
        obj = bpy.context.object
        shared = bpy.data.objects.new("Shared unmodified mesh", obj.data)
        bpy.context.scene.collection.objects.link(shared)
        weight = obj.data.attributes.new("weight", "FLOAT", "POINT")
        weight.data.foreach_set("value", np.full(8, 0.25, np.float32))
        color = obj.data.color_attributes.new("Color", "FLOAT_COLOR", "POINT")
        color.data.foreach_set("color", np.tile([0.25, 0.5, 0.75, 1], 8).astype(np.float32))
        obj.data.color_attributes.render_color_index = 0
        array = obj.modifiers.new("Native Array", "ARRAY")
        array.count = 2
        array.relative_offset_displace = (2, 0, 0)
        first = self.add(obj, scale=2)
        obj.modifiers.new("Downstream Triangulate", "TRIANGULATE")
        second = self.add(obj, translate=(1, 0, 0))
        self.cook()
        first_cache = self.m._cache(first).data
        self.assertEqual((len(first_cache.vertices), len(first_cache.polygons)), (16, 12))
        result = obj.evaluated_get(bpy.context.evaluated_depsgraph_get()).data
        self.assertEqual((len(result.vertices), len(result.polygons)), (16, 24))
        values = np.empty(16, np.float32)
        result.attributes["weight"].data.foreach_get("value", values)
        np.testing.assert_allclose(values, 0.25)
        colors = np.empty(16 * 4, np.float32)
        result.color_attributes["Color"].data.foreach_get("color", colors)
        np.testing.assert_allclose(colors.reshape(-1, 4), np.tile([0.25, 0.5, 0.75, 1], (16, 1)))
        self.assertEqual((len(obj.data.vertices), len(obj.data.polygons)), (8, 6))
        self.assertEqual(shared.data, obj.data)
        self.assertEqual(len(evaluated_positions(shared)), 8)
        np.testing.assert_allclose(np.sort(evaluated_positions(obj)[:, 0]),
                                   np.sort(np.repeat([-1, 3, 7, 11], 4)))
        self.assertEqual(len(self.m._cache(second).data.polygons), 24)

    def test_registered_ui_adds_selected_meshes_and_multiple_modifiers(self):
        import minihou_link
        registered = hasattr(bpy.types.Scene, "minihou_graph_path")
        if not registered:
            minihou_link.register()
        try:
            a, b = self.mesh("UI A"), self.mesh("UI B", 2)
            empty = bpy.data.objects.new("Ignore non-mesh", None)
            bpy.context.scene.collection.objects.link(empty)
            for obj in (a, b, empty):
                obj.select_set(True)
            bpy.context.view_layer.objects.active = a
            bpy.context.scene.minihou_graph_path = str(self.path)
            bpy.context.window_manager.minihou_port = self.engine.port
            self.assertEqual(bpy.ops.minihou.add_modifier(), {"FINISHED"})
            self.assertEqual(bpy.ops.minihou.add_modifier(), {"FINISHED"})
            self.assertEqual((len(a.modifiers), len(b.modifiers), len(empty.modifiers)), (2, 2, 0))
            ids = {self.m.instance_id(m) for o in (a, b) for m in o.modifiers}
            self.assertEqual(len(ids), 4)
            self.set_params(a.modifiers[0], Scale=3)
            self.assertEqual(bpy.ops.minihou.refresh_modifiers(), {"FINISHED"})
            self.assert_x(a, [3])
            self.assert_x(b, [2])
        finally:
            if not registered:
                minihou_link.unregister()

    def test_edit_and_render_guards(self):
        obj = self.mesh()
        mod = self.add(obj, scale=2)
        self.cook()
        obj.select_set(True)
        bpy.context.view_layer.objects.active = obj
        bpy.ops.object.mode_set(mode="EDIT")
        self.set_params(mod, Scale=3)
        self.assertEqual(self.cook(), 0)
        self.assertIn("Edit mode", self.m.status(mod))
        bpy.ops.object.mode_set(mode="OBJECT")
        self.cook()
        self.assert_x(obj, [3])
        self.m.register()
        self.assertIn(self.m._render_pre, bpy.app.handlers.render_init)
        self.m._render_pre(None)
        self.set_params(mod, Scale=4)
        self.assertEqual(self.m.pump(port=self.engine.port, force=True), 0)
        self.assert_x(obj, [3])
        self.m._render_done(None)
        self.cook()
        self.assert_x(obj, [4])

    def test_native_feedback_dependency_is_rejected(self):
        source = self.mesh("Source with MiniHou")
        self.add(source, scale=2)
        consumer = self.mesh("Consumer")
        native, _ = self.native_translate(consumer, 0)
        ng = native.node_group
        info = ng.nodes.new("GeometryNodeObjectInfo")
        info.inputs["Object"].default_value = source
        gout = next(n for n in ng.nodes if n.bl_idname == "NodeGroupOutput")
        ng.links.new(info.outputs["Geometry"], gout.inputs["Geometry"])
        mod = self.add(consumer)
        self.m.pump(port=self.engine.port)
        self.assertIn("feedback prevention", self.m.status(mod))
        self.assert_x(source, [2])

    def test_error_isolation_recovery_and_downstream_wait(self):
        broken = self.mesh("A Broken")
        first = self.add(broken, scale=2)
        second = self.add(broken, translate=(3, 0, 0))
        other = self.mesh("B Healthy")
        healthy = self.add(other, scale=4)
        self.cook()
        snapshot = first.node_group[self.m.GRAPH]
        bad = json.loads(snapshot)
        bad["nodes"]["i"]["type"] = "grid"  # explicit replacement-only rejection
        first.node_group[self.m.GRAPH] = json.dumps(bad)
        self.set_params(healthy, Scale=6)
        self.assertEqual(self.m.pump(port=self.engine.port), 1)
        self.assertIn("modifier_input", self.m.last_error())
        self.assertIn("Error", self.m.status(first))
        self.assertIn("preceding", self.m.status(second))
        self.assert_x(broken, [5])  # retained last valid result
        self.assert_x(other, [6])
        self.assertEqual(self.m.pump(port=self.engine.port), 0)
        first.node_group[self.m.GRAPH] = snapshot
        self.cook()
        self.assertEqual(self.m.status(first), "Ready")
        self.assertEqual(self.m.status(second), "Ready")

    def test_explicit_reload_template_preserves_overrides(self):
        path = Path(self.tmp.name) / "independent_template.json"
        path.write_text(json.dumps(GRAPH), encoding="utf-8")
        obj = self.mesh()
        mod = self.m.add_modifier(obj, str(path), port=self.engine.port)
        self.set_params(mod, Scale=3)
        self.cook()
        self.assert_x(obj, [3])
        changed = json.loads(json.dumps(GRAPH))
        changed["nodes"]["extra"] = {"type": "transform", "inputs": {"geo": "t"},
                                       "params": {"translate": [10, 0, 0]}}
        changed["nodes"]["o"]["inputs"]["geo"] = "extra"
        path.write_text(json.dumps(changed), encoding="utf-8")
        self.cook()
        self.assert_x(obj, [3])  # external edits are not silently adopted by instances
        self.m.reload_graph(mod, port=self.engine.port)
        self.cook()
        self.assert_x(obj, [13])
        self.assertEqual(self.proxy.read_values(mod, mod.node_group, self.m.parameters(mod))["Scale"], 3)

    def test_object_transform_and_disabled_modifier(self):
        obj = self.mesh()
        obj.location = (100, 20, 5)
        first = self.add(obj, scale=2)
        second = self.add(obj, translate=(3, 0, 0))
        self.cook()
        self.assert_x(obj, [5])  # local coordinates must not inherit inverse world transform
        first.show_viewport = False
        self.cook()
        self.assert_x(obj, [4])
        first.show_viewport = True
        self.cook()
        self.assert_x(obj, [5])


if __name__ == "__main__":
    suite = unittest.defaultTestLoader.loadTestsFromTestCase(ModifierStackTests)
    result = unittest.TextTestRunner(verbosity=2).run(suite)
    sys.exit(0 if result.wasSuccessful() else 1)
