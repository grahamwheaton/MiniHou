"""Real Blender source-binding integration tests (external engine required)."""
import json
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
sys.path[:0] = [str(ROOT), str(ROOT / 'blender_addon')]
from tests.test_blender_addon import Engine
from tests import test_blender_modifiers as stack_tests

SCATTER = {'version': 1, 'nodes': {
    'i': {'type': 'modifier_input'},
    's': {'type': 'scatter', 'inputs': {'geo': 'i'}, 'params': {'count': 7, 'seed': 12}},
    'inst': {'type': 'instance', 'inputs': {'points': 's'}, 'params': {
        'objects': '@source', 'align': 'up', 'random_spin': 0, 'scale_min': 1, 'scale_max': 1}},
    'o': {'type': 'output', 'inputs': {'geo': 'inst'}}}, 'display': 'o'}

@unittest.skipIf(bpy is None, 'requires real Blender')
class SourceObjectTests(unittest.TestCase):
    def setUp(self):
        from minihou_link import modifiers, proxy
        modifiers.unregister()
        bpy.ops.wm.read_factory_settings(use_empty=True)
        self.m, self.proxy = modifiers, proxy
        self._registered_addon = False

    tearDown = stack_tests.ModifierStackTests.tearDown
    cook = stack_tests.ModifierStackTests.cook
    native_translate = stack_tests.ModifierStackTests.native_translate

    @classmethod
    def setUpClass(cls):
        cls.tmp = tempfile.TemporaryDirectory()
        cls.path = Path(cls.tmp.name) / 'source.json'
        cls.path.write_text(json.dumps(SCATTER), encoding='utf8')
        bootstrap = Path(cls.tmp.name) / 'engine_bootstrap.json'
        bootstrap.write_text(json.dumps(stack_tests.GRAPH), encoding='utf8')
        cls.engine = Engine(bootstrap)

    @classmethod
    def tearDownClass(cls):
        cls.engine.stop()
        cls.tmp.cleanup()

    def test_multiple_imports_native_modifier_pickers_and_materials(self):
        from tests.test_blender_addon import evaluated_positions
        bpy.ops.mesh.primitive_monkey_add()
        monkey = bpy.context.object
        monkey_mat = bpy.data.materials.new("Monkey material")
        monkey.data.materials.append(monkey_mat)
        cube, cube_mat = self.setup_source("Second input", offset=3)
        graph = {"nodes": {
            "input": {"type": "modifier_input"},
            "monkey": {"type": "object_input"},
            "cube": {"type": "object_input"},
            "both": {"type": "merge", "inputs": {"geos": ["monkey", "cube"]}},
            "copies": {"type": "copy_to_points", "inputs": {"source": "both", "points": "input"}}
        }, "display": "copies"}
        self.path.write_text(json.dumps(graph), encoding="utf8")
        bpy.ops.mesh.primitive_plane_add()
        obj = bpy.context.object
        mod = self.m.add_modifier(obj, str(self.path), port=self.engine.port)
        self.m.set_object(mod, "monkey", monkey)
        self.m.set_object(mod, "cube", cube)
        for key in ("monkey", "cube"):
            self.assertFalse(self.m._object_socket(mod, key).hide_in_modifier)
        self.cook()
        bpy.context.view_layer.update()
        for key in ("monkey", "cube"):
            self.assertTrue(mod.is_input_used(self.m._object_socket(mod, key).identifier))
        self.assertTrue(mod.is_input_used(self.m._ensure_source(mod).identifier))
        self.assertEqual(len(evaluated_positions(obj)), 4 * (len(monkey.data.vertices) + 8))
        self.assertIn(monkey_mat, list(self.m._cache(mod).data.materials))
        self.assertIn(cube_mat, list(self.m._cache(mod).data.materials))
        # Source-only edits invalidate the imported mesh without editing the destination.
        before = evaluated_positions(obj).copy()
        for v in cube.data.vertices:
            v.co.x += 2
        cube.data.update()
        self.cook()
        self.assertFalse(np.allclose(before, evaluated_positions(obj)))
        self.m.set_object(mod, "cube", obj)
        self.m.pump(port=self.engine.port, force=True)
        self.assertIn("cycle", self.m.last_error())

    def test_multiple_native_instance_object_pickers(self):
        import copy
        first, mat_a = self.setup_source("First")
        second, mat_b = self.setup_source("Second", offset=3)
        graph = copy.deepcopy(SCATTER)
        graph["nodes"]["inst"]["params"].update(objects="@source @second", pick="cycle")
        self.path.write_text(json.dumps(graph), encoding="utf8")
        bpy.ops.mesh.primitive_plane_add(size=4)
        obj = bpy.context.object
        mod = self.m.add_modifier(obj, str(self.path), port=self.engine.port)
        self.m.set_source(mod, first)
        self.m.set_object(mod, "second", second)
        self.cook()
        vertices, materials, faces = self.realized(obj)
        self.assertEqual(len(vertices), 7 * 8)
        self.assertEqual(set(materials), {mat_a, mat_b})
        second.name = "Renamed input"
        self.cook()
        self.assertEqual(self.m.get_object(mod, "second"), second)

    def test_group_input_labels_preserve_native_object_selection(self):
        source, mat = self.setup_source("Monkey")
        graph = {"nodes": {"input": {"type": "modifier_input"},
            "socket1": {"type": "object_input", "input_owner": "input", "input_label": "Monkey"},
            "copy": {"type": "copy_to_points", "inputs": {"source": "socket1", "points": "input"}}}, "display": "copy"}
        self.path.write_text(json.dumps(graph), encoding="utf8")
        bpy.ops.mesh.primitive_plane_add()
        mod = self.m.add_modifier(bpy.context.object, str(self.path), port=self.engine.port)
        self.m.set_object(mod, "socket1", source)
        identifier = self.m._object_socket(mod, "socket1").identifier
        self.assertEqual(self.m._object_socket(mod, "socket1").name, "Monkey")
        graph["nodes"]["socket1"]["input_label"] = "Suzanne"
        mod.node_group[self.m.GRAPH] = json.dumps(graph)
        self.cook()
        self.assertEqual(self.m._object_socket(mod, "socket1").identifier, identifier)
        self.assertEqual(self.m._object_socket(mod, "socket1").name, "Suzanne")
        self.assertEqual(self.m.get_object(mod, "socket1"), source)
        del graph["nodes"]["socket1"]
        del graph["nodes"]["copy"]
        graph["display"] = "input"
        mod.node_group[self.m.GRAPH] = json.dumps(graph)
        self.cook()
        self.assertFalse(any(s.identifier == identifier for s in self.proxy._sockets(mod.node_group, "INPUT")))

    def setup_source(self, name='Source', offset=0):
        bpy.ops.mesh.primitive_cube_add()
        source = bpy.context.object
        source.name = name
        for v in source.data.vertices:
            v.co.x += offset
        source.data.update()
        mat = bpy.data.materials.new(name + ' material')
        source.data.materials.append(mat)
        return source, mat

    def setup_plane(self, source, name='Plane'):
        self.path.write_text(json.dumps(SCATTER), encoding='utf8')
        bpy.ops.mesh.primitive_plane_add(size=4)
        obj = bpy.context.object
        obj.name = name
        mod = self.m.add_modifier(obj, str(self.path), port=self.engine.port)
        self.m.set_source(mod, source)
        return obj, mod

    def instances(self, obj):
        bpy.context.view_layer.update()
        dg = bpy.context.evaluated_depsgraph_get()
        return [(i.object.original, i.matrix_world.copy()) for i in dg.object_instances
                if i.is_instance and i.parent and i.parent.original == obj]

    def realized(self, obj):
        # Realize on a disposable evaluator, preserving the production instance branch.
        copy = obj.copy()
        bpy.context.scene.collection.objects.link(copy)
        ng = bpy.data.node_groups.new('Test realize', 'GeometryNodeTree')
        ng.interface.new_socket('Geometry', in_out='INPUT', socket_type='NodeSocketGeometry')
        ng.interface.new_socket('Geometry', in_out='OUTPUT', socket_type='NodeSocketGeometry')
        a, b = ng.nodes.new('NodeGroupInput'), ng.nodes.new('NodeGroupOutput')
        r = ng.nodes.new('GeometryNodeRealizeInstances')
        ng.links.new(a.outputs['Geometry'], r.inputs['Geometry'])
        ng.links.new(r.outputs['Geometry'], b.inputs['Geometry'])
        copy.modifiers.new('Test realize', 'NODES').node_group = ng
        try:
            bpy.context.view_layer.update()
            ev = copy.evaluated_get(bpy.context.evaluated_depsgraph_get())
            mesh = ev.to_mesh()
            pos = np.array([tuple(v.co) for v in mesh.vertices])
            materials = [mesh.materials[p.material_index].original if p.material_index < len(mesh.materials) and mesh.materials[p.material_index] else None
                         for p in mesh.polygons]
            result = pos, materials, len(mesh.polygons)
            ev.to_mesh_clear()
            return result
        finally:
            bpy.data.objects.remove(copy, do_unlink=True)
            bpy.data.node_groups.remove(ng)

    def test_live_native_geometry_material_edits_keep_engine_cache(self):
        source, material = self.setup_source()
        array = source.modifiers.new('Source native array', 'ARRAY')
        array.count = 2
        obj, mod = self.setup_plane(source)
        self.cook()
        self.assertEqual(len(self.instances(obj)), 7)
        before, mats, faces = self.realized(obj)
        self.assertEqual((len(before), faces), (112, 84))
        self.assertEqual(set(mats), {material})
        cache = self.m._cache(mod)
        for v in source.data.vertices:
            v.co.z *= 3
        source.data.update()
        replacement = bpy.data.materials.new('Replacement')
        source.data.materials[0] = replacement
        self.assertEqual(self.cook(), 0)
        after, mats, _ = self.realized(obj)
        self.assertEqual(cache, self.m._cache(mod))
        self.assertGreater(np.ptp(after[:, 2]), np.ptp(before[:, 2]))
        self.assertEqual(set(mats), {replacement})
        self.assertEqual(len(source.data.vertices), 8)
        self.assertEqual(source.modifiers[0], array)

    def test_original_space_preserves_local_pivot_ignores_source_placement(self):
        source, _ = self.setup_source(offset=2)
        obj, mod = self.setup_plane(source)
        self.cook()
        before, _, _ = self.realized(obj)
        source.location = (100, 30, 10)
        source.rotation_euler = (0.4, 0.2, 1.0)
        source.scale = (3, 2, 4)
        self.assertEqual(self.cook(), 0)
        after, _, _ = self.realized(obj)
        np.testing.assert_allclose(after, before, atol=1e-5)
        centers = np.array([tuple(m.translation) for _, m in self.instances(obj)])
        self.assertAlmostEqual(float(before[:, 0].mean() - centers[:, 0].mean()), 2, places=4)

    def test_independent_selection_duplicate_and_rename(self):
        a, ma = self.setup_source('A')
        b, mb = self.setup_source('B')
        obj, first = self.setup_plane(a)
        self.cook()
        duplicate = obj.copy()
        bpy.context.scene.collection.objects.link(duplicate)
        self.cook()
        second = duplicate.modifiers[0]
        self.assertEqual(self.m.get_source(second), a)
        self.assertNotEqual(self.m.instance_id(first), self.m.instance_id(second))
        self.m.set_source(second, b)
        self.cook()
        a.name = 'Source renamed with spaces'
        self.cook()
        self.assertEqual(self.m.get_source(first), a)
        self.assertEqual({s for s, _ in self.instances(obj)}, {a})
        self.assertEqual({s for s, _ in self.instances(duplicate)}, {b})
        self.assertEqual(set(self.realized(obj)[1]), {ma})
        self.assertEqual(set(self.realized(duplicate)[1]), {mb})

    def test_same_object_multiple_modifier_copy_and_reorder(self):
        a, _ = self.setup_source('A')
        b, _ = self.setup_source('B')
        obj, first = self.setup_plane(a)
        self.cook()
        bpy.context.view_layer.objects.active = obj
        self.assertEqual(bpy.ops.object.modifier_copy(modifier=first.name), {'FINISHED'})
        second = obj.modifiers[1]
        self.cook()
        self.assertEqual(self.m.get_source(second), a)
        self.assertNotEqual(self.m.instance_id(first), self.m.instance_id(second))
        self.m.set_source(second, b)
        self.cook()
        self.assertEqual(self.m.get_source(first), a)
        self.assertEqual({s for s, _ in self.instances(obj)}, {b})
        obj.modifiers.move(1, 0)
        self.cook()
        self.assertEqual(self.m.get_source(first), a)
        self.assertEqual(self.m.get_source(second), b)
        self.assertEqual({s for s, _ in self.instances(obj)}, {a})
        self.assertEqual(self.cook(), 0)

    def test_file_roundtrip_reconnects_source_pointer(self):
        source, _ = self.setup_source()
        obj, mod = self.setup_plane(source)
        self.cook()
        ident = self.m.instance_id(mod)
        blend = str(Path(self.tmp.name) / 'sources.blend')
        self.m.register()
        bpy.ops.wm.save_as_mainfile(filepath=blend)
        bpy.ops.wm.open_mainfile(filepath=blend)
        obj = bpy.data.objects['Plane']
        mod = obj.modifiers[0]
        self.assertEqual(self.m.instance_id(mod), ident)
        self.assertEqual(self.m.get_source(mod), bpy.data.objects['Source'])
        self.cook(force=True)
        self.assertEqual(len(self.instances(obj)), 7)

    def test_invalid_missing_self_cache_cycle_recover_without_losing_cache(self):
        source, _ = self.setup_source()
        obj, mod = self.setup_plane(source)
        self.cook()
        cache = self.m._cache(mod)
        positions = [tuple(v.co) for v in cache.data.vertices]
        cyclic, _ = self.setup_source('Cyclic')
        self.native_translate(cyclic, 0)
        ng = cyclic.modifiers[0].node_group
        info = ng.nodes.new('GeometryNodeObjectInfo')
        info.inputs['Object'].default_value = obj
        out = next(n for n in ng.nodes if n.bl_idname == 'NodeGroupOutput')
        ng.links.new(info.outputs['Geometry'], out.inputs['Geometry'])
        for invalid in (None, obj, cache, cyclic):
            self.m.set_source(mod, invalid)
            self.m.pump(port=self.engine.port)
            self.assertIn('Error', self.m.status(mod))
            self.assertEqual(self.m._cache(mod), cache)
            self.assertEqual([tuple(v.co) for v in cache.data.vertices], positions)
            self.assertEqual({s for s, _ in self.instances(obj)}, {source})
            self.m.set_source(mod, source)
            self.cook()
            self.assertEqual(len(self.instances(obj)), 7)
        bpy.data.objects.remove(source, do_unlink=True)
        self.m.pump(port=self.engine.port)
        self.assertIn('Error', self.m.status(mod))
        replacement, _ = self.setup_source('Replacement')
        self.m.set_source(mod, replacement)
        self.cook()
        self.assertEqual({s for s, _ in self.instances(obj)}, {replacement})

    def test_reload_noninstancing_graph_recovers_after_source_deletion(self):
        source, _ = self.setup_source()
        obj, mod = self.setup_plane(source)
        self.cook()
        bpy.data.objects.remove(source, do_unlink=True)
        self.m.pump(port=self.engine.port)
        self.assertIn('Error', self.m.status(mod))
        path = Path(self.tmp.name) / 'replacement_transform.json'
        graph = json.loads(json.dumps(stack_tests.GRAPH))
        graph['nodes']['t']['params'] = {'translate': [0, 0, 3]}
        path.write_text(json.dumps(graph), encoding='utf8')
        mod.node_group[self.m.PATH] = str(path)
        self.m.reload_graph(mod, port=self.engine.port)
        self.assertEqual(json.loads(mod.node_group[self.m.GRAPH])['nodes']['t']['type'], 'transform')
        self.cook()
        vertices, _, faces = self.realized(obj)
        self.assertEqual((len(vertices), faces), (4, 1))
        np.testing.assert_allclose(vertices[:, 2], 3)
        self.assertFalse(self.instances(obj))
        self.assertEqual(self.cook(), 0)

    def test_driver_dependencies_owner_cache_other_minihou_are_rejected(self):
        source, _ = self.setup_source()
        obj, mod = self.setup_plane(source)
        self.cook()
        cache = self.m._cache(mod)
        other_source, _ = self.setup_source('Other source')
        other, other_mod = self.setup_plane(other_source, 'Other MiniHou')
        self.cook()
        native, _ = self.native_translate(source, 0)
        tree = native.node_group
        # Drivers may live on the Object, its Mesh datablock, or its native GN tree.
        for datablock in (source, source.data, tree):
            datablock['dependency_probe'] = 0.0
            for target in (obj, cache, other):
                curve = datablock.driver_add('["dependency_probe"]')
                driver = curve.driver
                variable = driver.variables.new()
                variable.name = 'destination'
                variable.type = 'SINGLE_PROP'
                variable.targets[0].id = target
                variable.targets[0].data_path = 'location.x'
                driver.expression = 'destination'
                self.m.pump(port=self.engine.port)
                self.assertIn('Error', self.m.status(mod), (datablock.name, target.name))
                datablock.driver_remove('["dependency_probe"]')
                self.cook()
                self.assertEqual(len(self.instances(obj)), 7)
        self.assertEqual(self.cook(), 0)

    def test_active_source_becoming_cyclic_disconnects_then_recovers(self):
        source, _ = self.setup_source()
        obj, mod = self.setup_plane(source)
        self.cook()
        cache = self.m._cache(mod)
        positions = [tuple(v.co) for v in cache.data.vertices]
        native, _ = self.native_translate(source, 0)
        ng = native.node_group
        info = ng.nodes.new('GeometryNodeObjectInfo')
        info.inputs['Object'].default_value = obj
        out = next(n for n in ng.nodes if n.bl_idname == 'NodeGroupOutput')
        ng.links.new(info.outputs['Geometry'], out.inputs['Geometry'])
        self.m.pump(port=self.engine.port)
        self.assertIn('Error', self.m.status(mod))
        self.assertEqual(self.m.get_source(mod), source)
        self.assertEqual(self.m._cache(mod), cache)
        self.assertEqual([tuple(v.co) for v in cache.data.vertices], positions)
        active = [n for n in mod.node_group.nodes
                  if n.name.startswith(self.proxy.INST_PREFIX + 'obj')
                  and n.bl_idname == 'GeometryNodeObjectInfo']
        self.assertTrue(active)
        self.assertTrue(all(n.inputs['Object'].default_value is None for n in active))
        # Repair the native dependency without touching the persistent picker.
        info.inputs['Object'].default_value = None
        gin = next(n for n in ng.nodes if n.bl_idname == 'NodeGroupInput')
        ng.links.new(gin.outputs['Geometry'], out.inputs['Geometry'])
        self.cook()
        self.assertEqual(self.m.get_source(mod), source)
        self.assertEqual(len(self.instances(obj)), 7)
        self.assertEqual(self.cook(), 0)

    def test_downstream_native_array_and_triangulate_before_minihou(self):
        source, material = self.setup_source()
        obj, first = self.setup_plane(source)
        self.cook()
        before, _, _ = self.realized(obj)
        # Native mesh-only modifiers require an explicit Realize in the real stack.
        # Prefix evaluation must not silently reinterpret the user's native stack.
        realize = obj.modifiers.new('Explicit native Realize Instances', 'NODES')
        ng = bpy.data.node_groups.new('Native realize', 'GeometryNodeTree')
        ng.interface.new_socket('Geometry', in_out='INPUT', socket_type='NodeSocketGeometry')
        ng.interface.new_socket('Geometry', in_out='OUTPUT', socket_type='NodeSocketGeometry')
        gin, gout = ng.nodes.new('NodeGroupInput'), ng.nodes.new('NodeGroupOutput')
        node = ng.nodes.new('GeometryNodeRealizeInstances')
        ng.links.new(gin.outputs['Geometry'], node.inputs['Geometry'])
        ng.links.new(node.outputs['Geometry'], gout.inputs['Geometry'])
        realize.node_group = ng
        array = obj.modifiers.new('Downstream native array', 'ARRAY')
        array.count = 2
        array.use_relative_offset = False
        array.use_constant_offset = True
        array.constant_offset_displace = (10, 0, 0)
        obj.modifiers.new('Downstream native triangulate', 'TRIANGULATE')
        path = Path(self.tmp.name) / 'identity.json'
        path.write_text(json.dumps(stack_tests.GRAPH), encoding='utf8')
        second = self.m.add_modifier(obj, str(path), port=self.engine.port)
        self.cook()
        after, mats, faces = self.realized(obj)
        self.assertEqual((len(after), faces), (112, 168))
        expected = np.concatenate([before, before + [10, 0, 0]])
        np.testing.assert_allclose(np.sort(after, axis=0), np.sort(expected, axis=0), atol=1e-5)
        self.assertEqual(set(mats), {material})
        self.assertEqual(len(source.data.vertices), 8)
        self.assertEqual(len(self.m._cache(first).data.vertices), 7)
        self.assertEqual(len(self.m._cache(second).data.vertices), 112)

    def test_downstream_native_and_minihou_realize_prefix_and_live_changes(self):
        source, material = self.setup_source()
        obj, first = self.setup_plane(source)
        self.cook()
        before, _, _ = self.realized(obj)
        self.native_translate(obj, 3)
        path = Path(self.tmp.name) / 'transform.json'
        graph = json.loads(json.dumps(stack_tests.GRAPH))
        graph['nodes']['t']['params'] = {'translate': [5, 0, 0]}
        path.write_text(json.dumps(graph), encoding='utf8')
        second = self.m.add_modifier(obj, str(path), port=self.engine.port)
        self.cook()
        after, mats, faces = self.realized(obj)
        self.assertEqual((len(after), faces), (56, 42))
        np.testing.assert_allclose(after, before + [8, 0, 0], atol=1e-5)
        self.assertEqual(set(mats), {material})
        for v in source.data.vertices:
            v.co.z *= 2
        source.data.update()
        self.assertEqual(self.cook(), 1)  # downstream consumes realized changes; scatter unchanged
        changed, _, _ = self.realized(obj)
        self.assertGreater(np.ptp(changed[:, 2]), np.ptp(after[:, 2]))
        self.assertEqual(self.cook(), 0)
        obj.modifiers.move(1, 0)  # native translation now affects scatter positions upstream
        self.cook()
        self.assertEqual(self.m.get_source(first), source)
        self.assertNotEqual(self.m.instance_id(first), self.m.instance_id(second))

if __name__ == '__main__':
    result = unittest.TextTestRunner(verbosity=2).run(unittest.defaultTestLoader.loadTestsFromTestCase(SourceObjectTests))
    sys.exit(0 if result.wasSuccessful() else 1)
