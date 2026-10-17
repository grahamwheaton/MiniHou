"""Real GN placements, using a separate external engine and evaluated Blender meshes.

Set MINIHOU_ENGINE_PYTHON, then run Blender --background --factory-startup
--python tests/test_blender_geonodes.py. No engine dependencies are installed in Blender.
"""
import functools
import json
import os
import subprocess
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
from tests.test_blender_addon import Engine, evaluated_positions, free_port
from tests.test_blender_modifiers import GRAPH


@functools.lru_cache(maxsize=None)
def _engine_has_mujoco():
    engine_py = os.environ.get("MINIHOU_ENGINE_PYTHON", sys.executable)
    try:
        res = subprocess.run([engine_py, "-c", "import mujoco"],
                             capture_output=True, timeout=5)
        return res.returncode == 0
    except Exception:
        return False


@unittest.skipIf(bpy is None, "requires actual Blender")
class GeometryNodesPlacementTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.tmp = tempfile.TemporaryDirectory()
        cls.path = Path(cls.tmp.name) / "wired_transform.json"
        cls.path.write_text(json.dumps(GRAPH), encoding="utf-8")
        cls.engine = Engine(cls.path, http_port=0)

    @classmethod
    def tearDownClass(cls):
        cls.engine.stop()
        cls.tmp.cleanup()

    def setUp(self):
        from minihou_link import geonodes
        self.g = geonodes
        self.g.unregister()
        bpy.ops.wm.read_factory_settings(use_empty=True)

    def tearDown(self):
        self.g.unregister()

    def mesh(self, name="Source", x=1, points=None):
        mesh = bpy.data.meshes.new(name)
        mesh.from_pydata(points or [(x, 0, 0)], [], [])
        obj = bpy.data.objects.new(name, mesh)
        bpy.context.scene.collection.objects.link(obj)
        return obj

    def tree(self, name="Native GN"):
        tree = bpy.data.node_groups.new(name, "GeometryNodeTree")
        tree.interface.new_socket("Geometry", in_out="INPUT", socket_type="NodeSocketGeometry")
        tree.interface.new_socket("Geometry", in_out="OUTPUT", socket_type="NodeSocketGeometry")
        return tree, tree.nodes.new("NodeGroupInput"), tree.nodes.new("NodeGroupOutput")

    def attach(self, obj, tree):
        mod = obj.modifiers.new("Mixed Geometry Nodes", "NODES")
        mod.node_group = tree
        return mod

    def native(self, tree, x):
        node = tree.nodes.new("GeometryNodeTransform")
        node.inputs["Translation"].default_value = (x, 0, 0)
        return node

    def mini(self, tree, scale=1, translate=0):
        node = self.g.add_group(tree, str(self.path), port=self.engine.port)
        node.inputs["Scale"].default_value = scale
        node.inputs["Translate"].default_value = (translate, 0, 0)
        return node

    def chain(self, tree, *nodes):
        for first, second in zip(nodes, nodes[1:]):
            tree.links.new(next(s for s in first.outputs if s.type == "GEOMETRY"),
                           next(s for s in second.inputs if s.type == "GEOMETRY"))

    def cook(self, force=False):
        count = self.g.pump(port=self.engine.port, force=force)
        self.assertEqual(self.g.last_error(), "")
        bpy.context.view_layer.update()
        return count

    def assert_x(self, obj, expected):
        np.testing.assert_allclose(np.sort(evaluated_positions(obj)[:, 0]), np.sort(expected), atol=1e-5)

    def mixed(self, points=None):
        obj = self.mesh(points=points)
        tree, gin, gout = self.tree()
        prefix, middle, suffix = (self.native(tree, x) for x in (2, 1, 5))
        first, second = self.mini(tree, scale=2), self.mini(tree, translate=4)
        first.name, second.name = "First MiniHou", "Second MiniHou"
        self.chain(tree, gin, prefix, first, middle, second, suffix, gout)
        self.attach(obj, tree)
        return obj, tree, prefix, first, second

    def test_native_before_between_after_and_independent_parameters(self):
        obj, tree, prefix, first, second = self.mixed()
        self.assertTrue(self.g.contains(tree))
        self.assertEqual(self.cook(), 2)
        self.assert_x(obj, [16])  # ((1+2)*2+1)+4+5
        prefix.inputs["Translation"].default_value = (3, 0, 0)
        self.cook()
        self.assert_x(obj, [18])
        first.inputs["Scale"].default_value = 3
        self.cook()
        self.assert_x(obj, [22])
        self.assertEqual(second.inputs["Scale"].default_value, 1)
        second.inputs["Translate"].default_value = (8, 0, 0)
        self.cook()
        self.assert_x(obj, [26])
        obj.data.vertices[0].co.x = 2
        obj.data.update()
        self.cook()
        self.assert_x(obj, [29])
        for _ in range(3):
            self.assertEqual(self.cook(), 0)
        self.assertEqual(obj.data.vertices[0].co.x, 2)

    def test_branch_join_and_reused_nested_group_interface(self):
        obj = self.mesh()
        inner, inner_in, inner_out = self.tree("Reusable nested native group")
        inner.interface.new_socket("Factor", in_out="INPUT", socket_type="NodeSocketFloat").default_value = 1
        mini = self.mini(inner)
        inner.links.new(inner_in.outputs["Factor"], mini.inputs["Scale"])
        self.chain(inner, inner_in, mini, inner_out)
        tree, gin, gout = self.tree()
        left, right = tree.nodes.new("GeometryNodeGroup"), tree.nodes.new("GeometryNodeGroup")
        left.node_tree = right.node_tree = inner
        left.inputs["Factor"].default_value = 2
        right.inputs["Factor"].default_value = 5
        join = tree.nodes.new("GeometryNodeJoinGeometry")
        for node in (left, right):
            self.chain(tree, gin, node, join)
        self.chain(tree, join, gout)
        self.attach(obj, tree)
        self.assertEqual(self.cook(), 2)
        self.assert_x(obj, [2, 5])
        right.inputs["Factor"].default_value = 7
        self.cook()
        self.assert_x(obj, [2, 7])
        self.assertEqual(left.inputs["Factor"].default_value, 2)
        self.assertEqual(self.cook(), 0)

    def test_duplicate_object_isolates_input_and_placement_parameters(self):
        obj, tree, _, _, _ = self.mixed()
        self.cook()
        duplicate = obj.copy()
        duplicate.data = obj.data.copy()
        duplicate.name = "AAA Duplicate"
        bpy.context.scene.collection.objects.link(duplicate)
        duplicate.data.vertices[0].co.x = 4
        duplicate.data.update()
        self.cook()
        self.assert_x(obj, [16])
        self.assert_x(duplicate, [22])
        duplicate_tree = duplicate.modifiers[0].node_group
        duplicate_tree.nodes["First MiniHou"].inputs["Scale"].default_value = 3
        self.cook()
        self.assert_x(obj, [16])
        self.assert_x(duplicate, [28])
        self.assertEqual(self.cook(), 0)

    def test_stack_native_modifiers_and_second_mixed_network(self):
        obj, _, _, _, _ = self.mixed()
        tree, gin, gout = self.tree("Second mixed network")
        mini = self.mini(tree, scale=2)
        self.chain(tree, gin, self.native(tree, 3), mini, self.native(tree, 7), gout)
        self.attach(obj, tree)
        self.assertEqual(self.cook(), 3)
        self.assert_x(obj, [45])
        self.assertEqual(self.cook(), 0)

    def test_native_topology_named_attributes_and_materials_survive(self):
        bpy.ops.mesh.primitive_cube_add()
        obj = bpy.context.object
        weight = obj.data.attributes.new("test_weight", "FLOAT", "POINT")
        weight.data.foreach_set("value", np.full(8, .375, np.float32))
        material = bpy.data.materials.new("Native assigned material")
        tree, gin, gout = self.tree()
        assign = tree.nodes.new("GeometryNodeSetMaterial")
        assign.inputs["Material"].default_value = material
        first, second = self.mini(tree, scale=2), self.mini(tree, translate=4)
        triangulate = tree.nodes.new("GeometryNodeTriangulate")
        self.chain(tree, gin, assign, first, triangulate, second, gout)
        self.attach(obj, tree)
        self.assertEqual(self.cook(), 2)
        result = obj.evaluated_get(bpy.context.evaluated_depsgraph_get()).data
        self.assertEqual((len(result.vertices), len(result.polygons)), (8, 12))
        np.testing.assert_allclose([v.value for v in result.attributes["test_weight"].data], .375)
        self.assertTrue(all(result.materials[p.material_index].original == material for p in result.polygons))
        self.assertEqual(len(obj.data.polygons), 6)
        self.assertEqual(self.cook(), 0)

    def test_save_reopen_retains_output_and_allows_recook(self):
        obj, _, _, _, _ = self.mixed()
        self.cook()
        blend = str(Path(self.tmp.name) / "wired_roundtrip.blend")
        self.g.register()
        bpy.ops.wm.save_as_mainfile(filepath=blend)
        bpy.ops.wm.open_mainfile(filepath=blend)
        obj = bpy.data.objects["Source"]
        self.assert_x(obj, [16])
        self.cook(force=True)
        self.assert_x(obj, [16])
        first = obj.modifiers[0].node_group.nodes["First MiniHou"]
        first.inputs["Scale"].default_value = 4
        self.cook()
        self.assert_x(obj, [22])
        self.assertEqual(self.cook(), 0)

    def test_missing_engine_keeps_last_valid_geometry_and_reports_failure(self):
        obj, _, _, first, _ = self.mixed()
        self.cook()
        first.inputs["Scale"].default_value = 8
        self.g.pump(port=free_port(), force=True)
        self.assertTrue(self.g.last_error())
        self.assertTrue(self.g.status(first))
        self.assert_x(obj, [16])
        self.cook(force=True)
        self.assert_x(obj, [34])

    def test_linked_parameter_math_drives_placement(self):
        obj, tree, _, first, _ = self.mixed()
        self.cook()
        math = tree.nodes.new("ShaderNodeMath")
        math.operation = "ADD"
        math.inputs[0].default_value = 10
        math.inputs[1].default_value = 0
        tree.links.new(math.outputs[0], first.inputs["Scale"])
        self.cook()
        self.assert_x(obj, [40])  # ((1+2)*10+1)+4+5
        math.inputs[0].default_value = 3
        self.cook()
        self.assert_x(obj, [19])


    def test_non_mesh_curve_component_is_rejected_and_cache_retained(self):
        obj, tree, _, first, _ = self.mixed()
        self.cook()
        curve = tree.nodes.new("GeometryNodeCurvePrimitiveLine")
        tree.links.new(curve.outputs["Curve"], first.inputs["Geometry"])
        self.g.pump(port=self.engine.port, force=True)
        self.assertIn("mesh", self.g.last_error().lower())
        self.assert_x(obj, [16])

    def test_native_repeat_zone_is_rejected_with_clear_status(self):
        obj, tree, _, first, _ = self.mixed()
        self.cook()
        repeat_out = tree.nodes.new("GeometryNodeRepeatOutput")
        repeat_in = tree.nodes.new("GeometryNodeRepeatInput")
        repeat_in.pair_with_output(repeat_out)
        self.g.pump(port=self.engine.port, force=True)
        self.assertIn("zone", self.g.last_error().lower())
        self.assertIn("unsupported", self.g.status(first).lower())
        self.assert_x(obj, [16])

    def test_self_object_dependency_is_rejected(self):
        obj, tree, _, _, _ = self.mixed()
        self.cook()
        tree.nodes.new("GeometryNodeSelfObject")
        self.g.pump(port=self.engine.port, force=True)
        self.assertRegex(self.g.last_error().lower(), "feedback|depend")
        self.assert_x(obj, [16])

    def test_cross_object_minihou_dependency_is_rejected(self):
        source, _, _, _, _ = self.mixed()
        self.cook()
        consumer = self.mesh("Consumer")
        tree, gin, gout = self.tree("Unsupported object dependency")
        node = self.mini(tree, scale=3)
        self.chain(tree, gin, node, gout)
        self.attach(consumer, tree)
        self.cook()
        self.assert_x(consumer, [3])
        info = tree.nodes.new("GeometryNodeObjectInfo")
        info.inputs["Object"].default_value = source
        self.chain(tree, info, node)
        self.g.pump(port=self.engine.port, force=True)
        self.assertRegex(self.g.last_error().lower(), "feedback|depend")
        self.assert_x(consumer, [3])
        self.assert_x(source, [16])

    def test_legacy_modifier_before_and_after_bridged_network(self):
        from minihou_link import modifiers, proxy
        obj, _, _, _, _ = self.mixed()
        before = modifiers.add_modifier(obj, str(self.path), port=self.engine.port)
        proxy.write_values(before, before.node_group, modifiers.parameters(before),
                           {"Scale": 3, "Translate": [0, 0, 0]})
        obj.modifiers.move(1, 0)
        after = modifiers.add_modifier(obj, str(self.path), port=self.engine.port)
        proxy.write_values(after, after.node_group, modifiers.parameters(after),
                           {"Scale": 2, "Translate": [0, 0, 0]})
        try:
            self.assertEqual(self.cook(), 4)
            self.assert_x(obj, [40])  # (((1*3+2)*2+1)+4+5)*2
            proxy.write_values(before, before.node_group, modifiers.parameters(before),
                               {"Scale": 4, "Translate": [0, 0, 0]})
            self.cook()
            self.assert_x(obj, [44])
            self.assertEqual(self.cook(), 0)
        finally:
            modifiers.unregister()

    def test_native_generated_mesh_without_original_mesh_geometry(self):
        obj = self.mesh()
        obj.data.clear_geometry()
        tree, gin, gout = self.tree()
        cube = tree.nodes.new("GeometryNodeMeshCube")
        mini = self.mini(tree, scale=3)
        self.chain(tree, cube, mini, gout)
        self.attach(obj, tree)
        self.assertEqual(self.cook(), 1)
        self.assertEqual(len(obj.data.vertices), 0)
        self.assert_x(obj, [-1.5] * 4 + [1.5] * 4)
        cube.inputs["Size"].default_value = (4, 2, 2)
        self.cook()
        self.assert_x(obj, [-6] * 4 + [6] * 4)

    def test_unused_group_cooks_without_blocking_native_output(self):
        obj = self.mesh()
        tree, gin, gout = self.tree()
        unused = self.mini(tree, scale=10)
        self.chain(tree, gin, self.native(tree, 4), gout)
        self.attach(obj, tree)
        self.assertEqual(self.cook(), 1)
        self.assertIn("Ready", self.g.status(unused))
        self.assert_x(obj, [5])

    def test_duplicate_placement_node_in_same_tree_is_independent(self):
        obj = self.mesh()
        tree, gin, gout = self.tree()
        first = self.mini(tree, scale=2)
        copied = tree.nodes.new("GeometryNodeGroup")
        copied.node_tree = first.node_tree
        copied.inputs["Scale"].default_value = 5
        copied.inputs["Translate"].default_value = (0, 0, 0)
        self.chain(tree, gin, first, copied, gout)
        self.attach(obj, tree)
        self.assertEqual(self.cook(), 2)
        self.assert_x(obj, [10])
        copied.inputs["Scale"].default_value = 7
        self.cook()
        self.assert_x(obj, [14])
        self.assertEqual(first.inputs["Scale"].default_value, 2)
        self.assertEqual(self.cook(), 0)

    def native_reference(self, name, source):
        obj = self.mesh(name)
        tree, gin, gout = self.tree(name + " network")
        info = tree.nodes.new("GeometryNodeObjectInfo")
        info.inputs["Object"].default_value = source
        self.chain(tree, info, gout)
        self.attach(obj, tree)
        return obj

    def test_transitive_native_object_reference_to_minihou_is_rejected(self):
        source, _, _, _, _ = self.mixed()
        self.cook()
        intermediate = self.native_reference("Intermediate", source)
        consumer = self.mesh("Transitive consumer")
        tree, gin, gout = self.tree()
        info = tree.nodes.new("GeometryNodeObjectInfo")
        info.inputs["Object"].default_value = intermediate
        mini = self.mini(tree)
        self.chain(tree, info, mini, gout)
        self.attach(consumer, tree)
        self.g.pump(port=self.engine.port, force=True)
        self.assertRegex(self.g.last_error().lower(), "feedback|depend")

    def test_modifier_bound_object_input_cannot_hide_feedback(self):
        from minihou_link import proxy
        source, _, _, _, _ = self.mixed()
        self.cook()
        consumer = self.mesh("Bound object consumer")
        tree, gin, gout = self.tree()
        interface = tree.interface.new_socket("Source Object", in_out="INPUT", socket_type="NodeSocketObject")
        info = tree.nodes.new("GeometryNodeObjectInfo")
        tree.links.new(gin.outputs["Source Object"], info.inputs["Object"])
        mini = self.mini(tree)
        self.chain(tree, info, mini, gout)
        mod = self.attach(consumer, tree)
        proxy.set_input(mod, interface.identifier, source)
        self.g.pump(port=self.engine.port, force=True)
        self.assertRegex(self.g.last_error().lower(), "feedback|depend")

    def test_modifier_bound_collection_input_cannot_hide_feedback(self):
        from minihou_link import proxy
        source, _, _, _, _ = self.mixed()
        self.cook()
        collection = bpy.data.collections.new("Input collection")
        bpy.context.scene.collection.children.link(collection)
        collection.objects.link(source)
        consumer = self.mesh("Bound collection consumer")
        tree, gin, gout = self.tree()
        interface = tree.interface.new_socket("Sources", in_out="INPUT", socket_type="NodeSocketCollection")
        info = tree.nodes.new("GeometryNodeCollectionInfo")
        tree.links.new(gin.outputs["Sources"], info.inputs["Collection"])
        mini = self.mini(tree)
        tree.links.new(info.outputs["Instances"], mini.inputs["Geometry"])
        self.chain(tree, mini, gout)
        mod = self.attach(consumer, tree)
        proxy.set_input(mod, interface.identifier, collection)
        self.g.pump(port=self.engine.port, force=True)
        self.assertRegex(self.g.last_error().lower(), "feedback|depend")

    def test_forced_recooks_and_failed_probes_do_not_leak_datablocks(self):
        obj, tree, _, first, _ = self.mixed()
        self.cook()
        def counts():
            return (len(bpy.data.objects), len(bpy.data.meshes), len(bpy.data.node_groups))
        before = counts()
        for _ in range(4):
            self.assertEqual(self.cook(force=True), 2)
            self.assertEqual(counts(), before)
        curve = tree.nodes.new("GeometryNodeCurvePrimitiveLine")
        tree.links.new(curve.outputs["Curve"], first.inputs["Geometry"])
        for _ in range(3):
            self.g.pump(port=self.engine.port, force=True)
            self.assertTrue(self.g.last_error())
            self.assertEqual(counts(), before)

    def test_removed_placement_releases_owned_cache_without_touching_user_data(self):
        obj = self.mesh()
        tree, gin, gout = self.tree()
        self.attach(obj, tree)
        self.chain(tree, gin, gout)
        before_objects = set(bpy.data.objects.keys())
        before_meshes = set(bpy.data.meshes.keys())
        mini = self.mini(tree, scale=2)
        self.chain(tree, gin, mini, gout)
        self.cook()
        tree.nodes.remove(mini)
        self.chain(tree, gin, gout)
        self.cook()
        self.assertEqual(set(bpy.data.objects.keys()), before_objects)
        self.assertEqual(set(bpy.data.meshes.keys()), before_meshes)
        self.assert_x(obj, [1])

    def test_repaired_parameter_link_restores_ready_status_without_recook(self):
        # Stale expectation: ShaderNodeMath was previously treated as an unsupported link,
        # but is now evaluated uniformly via _probe_value. Use a varying field (Index on a 2-point mesh)
        # to test parameter failure and recovery when the link is removed.
        obj, tree, _, first, _ = self.mixed(points=[(1, 0, 0), (2, 0, 0)])
        self.cook()
        idx = tree.nodes.new("GeometryNodeInputIndex")
        link = tree.links.new(idx.outputs[0], first.inputs["Scale"])
        self.g.pump(port=self.engine.port)
        self.assertTrue(self.g.last_error())
        self.assertIn("Error", self.g.status(first))
        tree.links.remove(link)
        self.assertEqual(self.cook(), 0)
        self.assertTrue(self.g.status(first).startswith("Ready"))
        self.assert_x(obj, [16, 18])

    def test_transitive_native_object_reference_back_to_owner_is_rejected(self):
        consumer = self.mesh("Owner")
        intermediate = self.native_reference("Back reference", consumer)
        tree, gin, gout = self.tree()
        info = tree.nodes.new("GeometryNodeObjectInfo")
        info.inputs["Object"].default_value = intermediate
        mini = self.mini(tree)
        self.chain(tree, info, mini, gout)
        self.attach(consumer, tree)
        self.g.pump(port=self.engine.port, force=True)
        self.assertRegex(self.g.last_error().lower(), "cycle|feedback|depend")

    def test_modifier_attribute_mode_parameter_is_rejected_honestly(self):
        from minihou_link import proxy
        obj = self.mesh()
        obj.data.attributes.new("weights", "FLOAT", "POINT").data[0].value = 9
        tree, gin, gout = self.tree()
        interface = tree.interface.new_socket("Factor", in_out="INPUT", socket_type="NodeSocketFloat")
        mini = self.mini(tree)
        tree.links.new(gin.outputs["Factor"], mini.inputs["Scale"])
        self.chain(tree, gin, mini, gout)
        mod = self.attach(obj, tree)
        proxy.set_input(mod, interface.identifier, 3)
        self.cook()
        self.assert_x(obj, [3])
        if hasattr(mod, "properties"):
            item = getattr(mod.properties.inputs, interface.identifier)
            item.type = "ATTRIBUTE"
            item.attribute_name = "weights"
        else:
            mod[interface.identifier + "_use_attribute"] = True
            mod[interface.identifier + "_attribute_name"] = "weights"
        obj.update_tag()
        self.g.pump(port=self.engine.port, force=True)
        self.assertRegex(self.g.last_error().lower(), "attribute|field")
        self.assertIn("Error", self.g.status(mini))
        self.assert_x(obj, [3])

    def test_muted_ancestor_containing_minihou_is_not_cooked(self):
        obj = self.mesh()
        inner, inner_in, inner_out = self.tree("Muted ancestor")
        mini = self.mini(inner, scale=5)
        self.chain(inner, inner_in, mini, inner_out)
        tree, gin, gout = self.tree()
        parent = tree.nodes.new("GeometryNodeGroup")
        parent.node_tree = inner
        self.chain(tree, gin, parent, gout)
        self.attach(obj, tree)
        self.cook()
        self.assert_x(obj, [5])
        parent.mute = True
        self.assertEqual(self.g.pump(port=self.engine.port, force=True), 0)
        self.assertIn("mut", self.g.last_error().lower())

    def test_registered_node_editor_operators_with_actual_context(self):
        import minihou_link
        from minihou_link import geonodes_ui
        minihou_link.register()
        area = bpy.context.screen.areas[0]
        old_type = area.type
        try:
            obj = self.mesh()
            obj.select_set(True)
            bpy.context.view_layer.objects.active = obj
            tree, gin, gout = self.tree("Editor operator test")
            self.attach(obj, tree)
            bpy.context.scene.minihou_graph_path = str(self.path)
            bpy.context.window_manager.minihou_port = self.engine.port
            area.type = "NODE_EDITOR"
            area.ui_type = "GeometryNodeTree"
            area.spaces.active.pin = True
            area.spaces.active.path.start(tree)
            region = next(r for r in area.regions if r.type == "WINDOW")
            with bpy.context.temp_override(area=area, region=region):
                self.assertTrue(geonodes_ui.MINIHOU_PT_geonodes.poll(bpy.context))
                self.assertTrue(bpy.ops.minihou.add_group.poll())
                self.assertEqual(bpy.ops.minihou.add_group(), {"FINISHED"})
                mini = tree.nodes.active
                self.assertTrue(self.g.is_group(mini))
                mini.inputs["Scale"].default_value = 3
                self.chain(tree, gin, mini, gout)
                self.assertEqual(bpy.ops.minihou.refresh_modifiers(), {"FINISHED"})
                self.assert_x(obj, [3])
                self.assertTrue(bpy.ops.minihou.reload_group.poll())
                self.assertEqual(bpy.ops.minihou.reload_group(), {"FINISHED"})
                self.assert_x(obj, [3])
                from unittest.mock import patch
                from minihou_link import network_ui
                with patch.object(network_ui.router, "start") as route, patch.object(self.g, "edit_live", return_value=({"state": "ready"}, "live.json")) as edit:
                    self.assertEqual(bpy.ops.minihou.edit_group_network(), {"FINISHED"})
                    route.assert_called_once_with({"state": "ready"}, "live.json", live=True)
                    edit.assert_called_once_with(obj, mini)
                area.spaces.active.path.append(mini.node_tree, node=mini)
                self.assertEqual(geonodes_ui._active(bpy.context), mini)
                self.assertTrue(bpy.ops.minihou.edit_group_network.poll())
        finally:
            area.type = old_type
            minihou_link.unregister()

    def test_nested_modifier_wrapper_converts_and_uses_wired_input(self):
        from minihou_link import modifiers
        obj = self.mesh()
        old = modifiers.add_modifier(obj, str(self.path), port=self.engine.port)
        wrapper = old.node_group
        obj.modifiers.remove(old)
        tree, gin, gout = self.tree()
        prefix, suffix = self.native(tree, 3), self.native(tree, 8)
        node = tree.nodes.new("GeometryNodeGroup")
        node.node_tree = wrapper
        node.inputs["Scale"].default_value = 2
        self.chain(tree, gin, prefix, node, suffix, gout)
        self.attach(obj, tree)
        self.cook()
        self.assert_x(obj, [16])
        self.assertIn(self.g.KEY, node.node_tree)
        self.assertNotIn(modifiers.KEY, node.node_tree)

    def test_contained_object_picker_import_and_reassignment(self):
        graph = {"nodes": {"input": {"type": "modifier_input"},
                           "source": {"type": "object_input", "params": {"binding": "asset"}},
                           "copies": {"type": "copy_to_points", "inputs": {"source": "source", "points": "input"}}},
                 "display": "copies"}
        path = Path(self.tmp.name) / "object_import.json"
        path.write_text(json.dumps(graph), encoding="utf-8")
        obj, source, replacement = self.mesh(), self.mesh("Asset", 4), self.mesh("Replacement", 9)
        tree, gin, gout = self.tree()
        node = self.g.add_group(tree, str(path), port=self.engine.port)
        node.inputs["Object: asset"].default_value = source
        self.chain(tree, gin, node, gout)
        self.attach(obj, tree)
        self.cook()
        self.assert_x(obj, [5])
        source.data.vertices[0].co.x = 6
        source.data.update()
        self.cook()
        self.assert_x(obj, [7])
        node.inputs["Object: asset"].default_value = replacement
        self.cook()
        self.assert_x(obj, [10])
        node.inputs["Object: asset"].default_value = obj
        self.g.pump(force=True)
        self.assertIn("cycle", self.g.last_error())
        self.assert_x(obj, [10])

    def test_live_group_edit_uses_its_wired_mesh_and_updates_only_it(self):
        import urllib.request
        from minihou_link import modifiers
        obj, tree, prefix, first, second = self.mixed()
        self.cook()
        # Connection already points to the test engine.
        server, path = self.g.edit_live(obj, first)
        self.assertEqual(server["state"], "ready")
        request = urllib.request.Request(server["editor_url"].rstrip("/") + "/api/edit",
            data=json.dumps({"ops": [{"op": "set", "id": "t", "params": {"uniform_scale": 3}}],
                             "wait": True}).encode(), headers={"Content-Type": "application/json"})
        with urllib.request.urlopen(request) as response:
            result = json.load(response)
        self.assertNotIn("error", result)
        self.cook()
        self.assertEqual(first.inputs["Scale"].default_value, 3)
        self.assertEqual(second.inputs["Scale"].default_value, 1)
        self.assert_x(obj, [19])

    def test_native_instances_with_contained_source_and_downstream_realization(self):
        graph = {"nodes": {"input": {"type": "modifier_input"},
                           "inst": {"type": "instance", "inputs": {"points": "input"},
                                    "params": {"objects": "@source", "random_spin": 0, "align": "up"}}},
                 "display": "inst"}
        path = Path(self.tmp.name) / "native_instances.json"
        path.write_text(json.dumps(graph), encoding="utf-8")
        obj, source = self.mesh(), self.mesh("Asset", 4)
        tree, gin, gout = self.tree()
        node = self.g.add_group(tree, str(path), port=self.engine.port)
        node.inputs["Source Object"].default_value = source
        realize = tree.nodes.new("GeometryNodeRealizeInstances")
        suffix = self.native(tree, 2)
        self.chain(tree, gin, node, realize, suffix, gout)
        self.attach(obj, tree)
        self.cook()
        self.assert_x(obj, [7])
        source.data.vertices[0].co.x = 6
        source.data.update()
        self.cook()
        self.assert_x(obj, [9])
        # Copy the group occurrence: source sockets must remain independently editable.
        other = self.mesh("Second owner", 3)
        copied = tree.copy()
        self.attach(other, copied)
        self.cook()
        copied_node = next(n for n in other.modifiers[0].node_group.nodes if self.g.is_group(n))
        asset2 = self.mesh("Second asset", 10)
        copied_node.inputs["Source Object"].default_value = asset2
        self.cook()
        self.assert_x(other, [15])
        self.assert_x(obj, [9])
        self.assertNotEqual(copied_node.node_tree[self.g.KEY], node.node_tree[self.g.KEY])
        saved = Path(self.tmp.name) / "objects_roundtrip.blend"
        bpy.ops.wm.save_as_mainfile(filepath=str(saved))
        bpy.ops.wm.open_mainfile(filepath=str(saved))
        self.g._reset()
        self.cook()
        self.assert_x(bpy.data.objects["Second owner"], [15])
        self.assert_x(bpy.data.objects["Source"], [9])

    def rbd_path(self):
        graph = {"version": 1, "nodes": {
            "i": {"type": "modifier_input"},
            "c": {"type": "rbd_configure", "inputs": {"geo": "i"}},
            "s": {"type": "rbd_solver", "inputs": {"bodies": "c"}},
            "o": {"type": "output", "inputs": {"geo": "s"}},
        }, "display": "o", "exposed": [
            {"name": "Velocity", "node": "c", "param": "velocity"},
            {"name": "Gravity", "node": "s", "param": "gravity"},
        ]}
        path = Path(self.tmp.name) / "rbd_wired.json"
        path.write_text(json.dumps(graph), encoding="utf-8")
        return path

    @unittest.skipUnless(_engine_has_mujoco(), "RBD requires the optional physics extra (MuJoCo) in the engine Python environment")
    def test_rbd_wired_native_mesh_two_independent_placements(self):
        bpy.ops.mesh.primitive_cube_add()
        obj = bpy.context.object
        tree, gin, gout = self.tree("Wired rigid bodies")
        before = self.native(tree, 0)
        before.inputs["Translation"].default_value = (0, 0, 4)
        self.chain(tree, gin, before)
        path = self.rbd_path()
        first = self.g.add_group(tree, str(path), port=self.engine.port)
        second = self.g.add_group(tree, str(path), port=self.engine.port)
        first.inputs["Velocity"].default_value = (0, 0, 0)
        second.inputs["Velocity"].default_value = (2, 0, 0)
        join = tree.nodes.new("GeometryNodeJoinGeometry")
        self.chain(tree, before, first, self.native(tree, 10), join)
        self.chain(tree, before, second, self.native(tree, -10), join)
        self.chain(tree, join, gout)
        self.attach(obj, tree)
        bpy.context.scene.frame_set(1)
        self.assertEqual(self.cook(force=True), 2)
        initial = evaluated_positions(obj).copy()
        self.assertEqual(initial.shape, (16, 3))
        np.testing.assert_allclose(initial[:, 2].mean(), 4, atol=1e-5)
        bpy.context.scene.frame_set(30)
        self.assertEqual(self.cook(force=True), 2)
        final = evaluated_positions(obj)
        still = final[final[:, 0] > 0]
        moving = final[final[:, 0] < 0]
        self.assertEqual((len(still), len(moving)), (8, 8))
        self.assertLess(still[:, 2].mean(), initial[:, 2].mean() - 1)
        self.assertLess(moving[:, 2].mean(), initial[:, 2].mean() - 1)
        self.assertAlmostEqual(still[:, 0].mean(), 10, places=4)
        self.assertGreater(moving[:, 0].mean(), -9)
        self.assertEqual(tuple(first.inputs["Velocity"].default_value), (0, 0, 0))
        self.assertEqual(self.cook(), 0)
        self.assertEqual(len(obj.data.vertices), 8)
        np.testing.assert_allclose(sorted(v.co.z for v in obj.data.vertices), [-1] * 4 + [1] * 4)
        duplicate = obj.copy()
        duplicate.name = "Independent RBD duplicate"
        bpy.context.scene.collection.objects.link(duplicate)
        self.cook(force=True)
        duplicate_node = duplicate.modifiers[0].node_group.nodes[second.name]
        duplicate_node.inputs["Velocity"].default_value = (4, 0, 0)
        self.cook(force=True)
        np.testing.assert_allclose(evaluated_positions(obj), final, atol=1e-5)
        duplicate_positions = evaluated_positions(duplicate)
        duplicate_moving = duplicate_positions[duplicate_positions[:, 0] < 0]
        self.assertEqual(len(duplicate_moving), 8)
        self.assertGreater(duplicate_moving[:, 0].mean(), moving[:, 0].mean() + 1)
        original_node = obj.modifiers[0].node_group.nodes[second.name]
        self.assertEqual(tuple(original_node.inputs["Velocity"].default_value), (2, 0, 0))

    @unittest.skipUnless(_engine_has_mujoco(), "RBD requires the optional physics extra (MuJoCo) in the engine Python environment")
    def test_rbd_rejects_animated_native_input_and_retains_valid_mesh(self):
        bpy.ops.mesh.primitive_cube_add()
        obj = bpy.context.object
        tree, gin, gout = self.tree("Animated rigid body input")
        before = self.native(tree, 0)
        translation = before.inputs["Translation"]
        translation.default_value = (0, 0, 4)
        mini = self.g.add_group(tree, str(self.rbd_path()), port=self.engine.port)
        self.chain(tree, gin, before, mini, gout)
        self.attach(obj, tree)
        bpy.context.scene.frame_set(1)
        self.cook(force=True)
        initial = evaluated_positions(obj).copy()
        translation.keyframe_insert("default_value", frame=1)
        translation.default_value = (0, 0, 5)
        translation.keyframe_insert("default_value", frame=30)
        bpy.context.scene.frame_set(30)
        self.g.pump(port=self.engine.port, force=True)
        self.assertRegex(self.g.last_error().lower(), "animated|static")
        self.assertIn("Error", self.g.status(mini))
        np.testing.assert_allclose(evaluated_positions(obj), initial)

    def test_later_modifier_waits_after_earlier_wired_placement_failure(self):
        # Stale expectation: ShaderNodeMath was previously treated as an unsupported link,
        # but is now evaluated uniformly via _probe_value. Use a varying field (Index on a 2-point mesh)
        # to test that an upstream parameter failure puts downstream placements into Waiting state.
        obj, first_tree, _, first, _ = self.mixed(points=[(1, 0, 0), (2, 0, 0)])
        tree, gin, gout = self.tree("Later GN modifier")
        later = self.mini(tree, scale=2)
        self.chain(tree, gin, later, gout)
        self.attach(obj, tree)
        self.cook()
        self.assert_x(obj, [32, 36])
        idx = first_tree.nodes.new("GeometryNodeInputIndex")
        first_tree.links.new(idx.outputs[0], first.inputs["Scale"])
        self.g.pump(port=self.engine.port, force=True)
        self.assertTrue(self.g.last_error())
        self.assertTrue(self.g.status(later).startswith("Waiting"), self.g.status(later))
        self.assert_x(obj, [32, 36])

    def test_fake_user_group_does_not_duplicate_on_every_refresh(self):
        obj = self.mesh()
        tree, gin, gout = self.tree()
        mini = self.mini(tree, scale=2)
        tree.use_fake_user = True
        mini.node_tree.use_fake_user = True
        self.chain(tree, gin, mini, gout)
        mod = self.attach(obj, tree)
        self.cook()
        count = len(bpy.data.node_groups)
        ident = mini.node_tree[self.g.KEY]
        self.assertEqual(self.cook(), 0)
        self.assertEqual(len(bpy.data.node_groups), count)
        self.assertEqual(mod.node_group, tree)
        self.assertEqual(mini.node_tree[self.g.KEY], ident)
        self.assert_x(obj, [2])


if __name__ == "__main__":
    unittest.main(argv=[sys.argv[0]], verbosity=2)
