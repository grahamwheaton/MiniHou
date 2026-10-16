"""Nested modifier layout: Group Input -> one private plumbing group -> Group Output (external engine required).

MINIHOU_ENGINE_PYTHON=<supported external Python> blender --background --factory-startup
    --python tools/blender_selftest.py
"""
import json
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

try:
    import bpy
except ImportError:
    bpy = None

ROOT = Path(__file__).resolve().parent.parent
sys.path[:0] = [str(ROOT), str(ROOT / "blender_addon")]
from tests.test_blender_addon import Engine, evaluated_positions
from tests import test_blender_modifiers as stack_tests
from tests import test_blender_source_objects as source_tests  # module import: do not re-collect its tests
SCATTER = source_tests.SCATTER


@unittest.skipIf(bpy is None, "requires real Blender")
class NestedLayoutTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.tmp = tempfile.TemporaryDirectory()
        cls.path = Path(cls.tmp.name) / "stack.json"
        cls.path.write_text(json.dumps(stack_tests.GRAPH), encoding="utf-8")
        cls.engine = Engine(cls.path, http_port=0)

    @classmethod
    def tearDownClass(cls):
        cls.engine.stop()
        cls.tmp.cleanup()

    def setUp(self):
        from minihou_link import geonodes, modifiers, proxy
        modifiers.unregister()
        geonodes.unregister()
        bpy.ops.wm.read_factory_settings(use_empty=True)
        self.m, self.proxy, self.g = modifiers, proxy, geonodes
        self._registered_addon = False
        self.path.write_text(json.dumps(stack_tests.GRAPH), encoding="utf-8")

    tearDown = stack_tests.ModifierStackTests.tearDown
    cook = stack_tests.ModifierStackTests.cook
    mesh = stack_tests.ModifierStackTests.mesh
    set_params = stack_tests.ModifierStackTests.set_params
    setup_source = source_tests.SourceObjectTests.setup_source
    instances = source_tests.SourceObjectTests.instances

    # -- helpers -----------------------------------------------------------------------------------------------------
    def plumbing_trees(self):
        return [t for t in bpy.data.node_groups if self.proxy.PLUMBING_KEY in t]

    def assert_nested(self, ng):
        """Outer tree is exactly Group Input -> plumbing group node -> Group Output, wired socket for socket."""
        p = self.proxy
        self.assertEqual(sorted(n.bl_idname for n in ng.nodes),
                         ["GeometryNodeGroup", "NodeGroupInput", "NodeGroupOutput"])
        node = p.plumbing_node(ng)
        self.assertIsNotNone(node)
        inner = node.node_tree
        self.assertEqual(node.label, p.PLUMBING_LABEL)
        self.assertEqual(inner.name, p.plumbing_name(ng))
        self.assertEqual(inner.users, 1, "plumbing must be private to one outer group")
        for key in (self.m.KEY, self.g.KEY):
            self.assertNotIn(key, inner, "plumbing must never look like a MiniHou group itself")
        self.assertEqual([(s.name, s.socket_type) for s in p._sockets(inner, "INPUT")],
                         [(s.name, s.socket_type) for s in p._sockets(ng, "INPUT")])
        self.assertEqual([s.socket_type for s in p._sockets(inner, "OUTPUT")], ["NodeSocketGeometry"])
        gin = next(n for n in ng.nodes if n.bl_idname == "NodeGroupInput")
        gout = next(n for n in ng.nodes if n.bl_idname == "NodeGroupOutput")
        mapping = json.loads(inner[p.PLUMBING_KEY])
        for src in p._sockets(ng, "INPUT"):
            dst = next(s for s in node.inputs if s.identifier == mapping[src.identifier])
            self.assertEqual(dst.name, src.name)
            self.assertTrue(dst.is_linked)
            self.assertEqual(dst.links[0].from_socket.identifier, src.identifier)
            self.assertEqual(dst.links[0].from_node, gin)
        self.assertEqual(gout.inputs["Geometry"].links[0].from_node, node)
        # Every generated node lives inside.
        self.assertIsNotNone(inner.nodes.get(p.CACHE_INFO_NODE))
        self.assertFalse([n for n in ng.nodes if n.name.startswith(p._GENERATED)])
        return inner

    def flat_modifier(self, obj, path):
        """Build a modifier exactly as add-on 0.1.5 did: every helper degrades to the flat layout."""
        p = self.proxy
        with patch.object(p, "ensure_nested", side_effect=lambda ng: ng):
            mod = self.m.add_modifier(obj, str(path), port=self.engine.port)
        mod.node_group["minihou_object_usage_version"] = 2
        self.assertFalse(p.is_nested(mod.node_group))
        self.assertIsNotNone(mod.node_group.nodes.get(p.CACHE_INFO_NODE))
        return mod

    # -- tests -------------------------------------------------------------------------------------------------------
    def test_new_modifier_is_nested_passthrough_then_cooks(self):
        obj = self.mesh(x=1)
        mod = self.m.add_modifier(obj, str(self.path), port=self.engine.port)
        ng = mod.node_group
        inner = self.assert_nested(ng)
        for key in (self.m.KEY, self.m.GRAPH, self.m.PATH, self.m.PARAMS, self.m.STATUS):
            self.assertIn(key, ng)
        self.assertEqual(ng[self.m.KEY], self.m.instance_id(mod))
        stores = [n for n in inner.nodes if n.name.startswith(self.proxy.USE_NODE_PREFIX)]
        self.assertGreaterEqual(len(stores), 2)  # Scale, Translate (+ Source Object compare on 5.2)
        self.assertTrue(self.proxy.passthrough(ng))
        bpy.context.view_layer.update()
        self.assertAlmostEqual(evaluated_positions(obj)[0, 0], 1)
        self.set_params(mod, Scale=2, Translate=(1, 0, 0))
        self.assertEqual(self.cook(), 1)
        self.assertAlmostEqual(evaluated_positions(obj)[0, 0], 3, places=5)
        self.assertFalse(self.proxy.passthrough(ng))
        self.assertEqual(self.proxy.cache_info(ng).inputs["Object"].default_value, self.m._cache(mod))
        self.assert_nested(ng)
        before = len(bpy.data.node_groups)
        for _ in range(3):
            self.assertEqual(self.cook(), 0)
        self.assertEqual(len(bpy.data.node_groups), before)
        self.assertEqual(len(self.plumbing_trees()), 1)
        if bpy.app.version >= (5, 2, 0):
            bpy.context.view_layer.update()
            for item in self.proxy._sockets(ng, "INPUT"):
                if item.socket_type != "NodeSocketGeometry":
                    self.assertTrue(mod.is_input_used(item.identifier), item.name)

    def test_flat_group_migrates_keeping_values_pickers_cache_and_instances(self):
        path = Path(self.tmp.name) / "scatter.json"
        path.write_text(json.dumps(SCATTER), encoding="utf-8")
        source, _ = self.setup_source()
        bpy.ops.mesh.primitive_plane_add(size=4)
        obj = bpy.context.object
        mod = self.flat_modifier(obj, path)
        self.m.set_source(mod, source)
        with patch.object(self.proxy, "ensure_nested", side_effect=lambda ng: ng):
            self.cook()
        ng = mod.node_group
        self.assertFalse(self.proxy.is_nested(ng))
        self.assertTrue([n for n in ng.nodes if n.name.startswith(self.proxy.INST_PREFIX)])
        ng["minihou_object_usage_version"] = 2
        ident, cache, before = self.m.instance_id(mod), self.m._cache(mod), self.instances(obj)
        props = {k: ng[k] for k in ng.keys() if k not in {"minihou_object_usage_version"}}
        self.assertEqual(len(before), 7)
        self.cook()  # migration happens on the next cook
        self.assertEqual(mod.node_group.name, ng.name)  # same outer datablock, migrated in place
        inner = self.assert_nested(ng)
        self.assertEqual(ng["minihou_object_usage_version"], self.m.USAGE_VERSION)
        self.assertEqual(self.m.instance_id(mod), ident)
        self.assertEqual(self.m._cache(mod), cache)
        self.assertEqual(self.m.get_source(mod), source)
        for key, value in props.items():
            if key != self.m.STATUS:
                self.assertEqual(ng[key], value, key)
        self.assertEqual(self.m.status(mod), "Ready")
        self.assertTrue([n for n in inner.nodes if n.name.startswith(self.proxy.INST_PREFIX + "obj")])
        after = self.instances(obj)
        self.assertEqual(len(after), 7)
        self.assertEqual([o for o, _ in after], [o for o, _ in before])
        for (_, a), (_, b) in zip(before, after):
            self.assertTrue(all(abs(x - y) < 1e-5 for ra, rb in zip(a, b) for x, y in zip(ra, rb)))
        # Pickers still drive the output after migration.
        other, _ = self.setup_source("Other", offset=2)
        self.m.set_source(mod, other)
        self.cook()
        self.assertEqual({o for o, _ in self.instances(obj)}, {other})
        self.assertEqual(len(self.plumbing_trees()), 1)

    def test_uncooked_flat_group_migrates_as_passthrough(self):
        obj = self.mesh(x=1)
        mod = self.flat_modifier(obj, self.path)
        self.set_params(mod, Scale=4)
        self.m._interface(mod, {"interface": self.m.parameters(mod)})
        self.assert_nested(mod.node_group)
        self.assertTrue(self.proxy.passthrough(mod.node_group))
        bpy.context.view_layer.update()
        self.assertAlmostEqual(evaluated_positions(obj)[0, 0], 1)
        self.assertEqual(self.m.parameters(mod)[0]["name"], "Scale")
        self.assertAlmostEqual(self.proxy.read_values(mod, mod.node_group, self.m.parameters(mod))["Scale"], 4)
        self.cook()
        self.assertAlmostEqual(evaluated_positions(obj)[0, 0], 4, places=5)

    def test_duplicate_gets_private_plumbing_and_removal_cleans_up(self):
        obj = self.mesh(x=1)
        mod = self.m.add_modifier(obj, str(self.path), port=self.engine.port)
        self.set_params(mod, Scale=2)
        self.cook()
        copy = obj.copy()  # shares the modifier's node group, like Shift+D
        copy.name = "Copy"
        bpy.context.scene.collection.objects.link(copy)
        self.cook()
        a, b = obj.modifiers[0].node_group, copy.modifiers[0].node_group
        self.assertNotEqual(a.name, b.name)
        inner_a, inner_b = self.assert_nested(a), self.assert_nested(b)
        self.assertNotEqual(inner_a.name, inner_b.name)
        self.assertNotEqual(self.proxy.cache_info(a).inputs["Object"].default_value,
                            self.proxy.cache_info(b).inputs["Object"].default_value)
        self.set_params(copy.modifiers[0], Scale=5)
        self.cook()
        self.assertAlmostEqual(evaluated_positions(obj)[0, 0], 2, places=5)
        self.assertAlmostEqual(evaluated_positions(copy)[0, 0], 5, places=5)
        self.assertEqual(len(self.plumbing_trees()), 2)
        copy.modifiers.remove(copy.modifiers[0])
        self.cook()
        self.assertEqual(len(self.plumbing_trees()), 1)
        obj.modifiers.remove(obj.modifiers[0])
        self.m.pump(port=self.engine.port)
        self.assertEqual(self.plumbing_trees(), [])
        self.assertFalse([t for t in bpy.data.node_groups if self.m.KEY in t])

    def test_file_roundtrip_keeps_one_private_plumbing_group(self):
        obj = self.mesh(x=1)
        mod = self.m.add_modifier(obj, str(self.path), port=self.engine.port)
        self.set_params(mod, Scale=3)
        self.cook()
        saved = Path(self.tmp.name) / "nested_roundtrip.blend"
        name = obj.name
        bpy.ops.wm.save_as_mainfile(filepath=str(saved))
        bpy.ops.wm.open_mainfile(filepath=str(saved))
        obj = bpy.data.objects[name]
        mod = obj.modifiers[0]
        self.cook(force=True)
        self.assert_nested(mod.node_group)
        self.assertEqual(len(self.plumbing_trees()), 1)
        self.assertAlmostEqual(evaluated_positions(obj)[0, 0], 3, places=5)

    def test_reload_with_changed_interface_remirrors_plumbing(self):
        obj = self.mesh(x=1)
        mod = self.m.add_modifier(obj, str(self.path), port=self.engine.port)
        self.set_params(mod, Scale=3)
        self.cook()
        graph = json.loads(json.dumps(stack_tests.GRAPH))
        graph["exposed"] = [{"name": "Size", "node": "t", "param": "uniform_scale"},
                            {"name": "Scale", "node": "t", "param": "uniform_scale"}]
        self.path.write_text(json.dumps(graph), encoding="utf-8")
        self.m.reload_graph(mod, port=self.engine.port)
        inner = self.assert_nested(mod.node_group)
        names = [s.name for s in self.proxy._sockets(inner, "INPUT")]
        self.assertIn("Size", names)
        self.assertNotIn("Translate", names)
        self.assertEqual(self.proxy.read_values(mod, mod.node_group, self.m.parameters(mod))["Scale"], 3)
        self.set_params(mod, Size=2, Scale=2)
        self.cook()
        self.assertAlmostEqual(evaluated_positions(obj)[0, 0], 2, places=5)

    def test_live_edit_forwards_params_into_nested_modifier(self):
        import urllib.request
        obj = self.mesh(x=1)
        mod = self.m.add_modifier(obj, str(self.path), port=self.engine.port)
        self.cook()
        server, _ = self.m.edit_live(obj, mod)
        self.assertEqual(server["state"], "ready")
        request = urllib.request.Request(server["editor_url"].rstrip("/") + "/api/edit",
            data=json.dumps({"ops": [{"op": "set", "id": "t", "params": {"uniform_scale": 3}}],
                             "wait": True}).encode(), headers={"Content-Type": "application/json"})
        with urllib.request.urlopen(request) as response:
            self.assertNotIn("error", json.load(response))
        self.cook()
        self.assert_nested(mod.node_group)
        values = self.proxy.read_values(mod, mod.node_group, self.m.parameters(mod))
        self.assertAlmostEqual(values["Scale"], 3)
        self.assertAlmostEqual(evaluated_positions(obj)[0, 0], 3, places=5)

    def test_wired_placements_are_nested_and_copies_get_private_plumbing(self):
        obj = self.mesh(x=1)
        tree = bpy.data.node_groups.new("Native GN", "GeometryNodeTree")
        tree.interface.new_socket("Geometry", in_out="INPUT", socket_type="NodeSocketGeometry")
        tree.interface.new_socket("Geometry", in_out="OUTPUT", socket_type="NodeSocketGeometry")
        gin, gout = tree.nodes.new("NodeGroupInput"), tree.nodes.new("NodeGroupOutput")
        node = self.g.add_group(tree, str(self.path), port=self.engine.port)
        node.inputs["Scale"].default_value = 2
        tree.links.new(gin.outputs["Geometry"], node.inputs["Geometry"])
        tree.links.new(node.outputs["Geometry"], gout.inputs["Geometry"])
        obj.modifiers.new("GN", "NODES").node_group = tree
        self.assert_nested(node.node_tree)
        self.assertFalse(self.g.is_group(self.proxy.plumbing_node(node.node_tree)))
        self.assertEqual([path for path, _ in self.g._placements(tree)], [(node.name,)])
        self.g.pump(port=self.engine.port)
        self.assertEqual(self.g.last_error(), "")
        bpy.context.view_layer.update()
        self.assertAlmostEqual(evaluated_positions(obj)[0, 0], 2, places=5)
        other = self.mesh("Other", x=3)
        other.modifiers.new("GN", "NODES").node_group = tree.copy()
        self.g.pump(port=self.engine.port)
        self.assertEqual(self.g.last_error(), "")
        copied = next(n for n in other.modifiers[0].node_group.nodes if self.g.is_group(n))
        self.assertNotEqual(copied.node_tree[self.g.KEY], node.node_tree[self.g.KEY])
        self.assert_nested(copied.node_tree)
        self.assert_nested(node.node_tree)
        bpy.context.view_layer.update()
        self.assertAlmostEqual(evaluated_positions(other)[0, 0], 6, places=5)


if __name__ == "__main__":
    unittest.main()
