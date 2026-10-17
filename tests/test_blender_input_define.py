"""Object-input eyedroppers and Blender-side Add Input (add-on 0.3.1, input_define_v1) in REAL Blender.

    $env:MINIHOU_LINK_SELFTEST="rust/target_addinput/debug/examples/link_selftest.exe"
    blender --background --factory-startup --python-exit-code 1 --python tests/test_blender_input_define.py

Run with plain python it skips (no bpy). The engine is the Rust link_selftest example (blender_link.rs).
"""
import json
import os
import sys
import tempfile
import time
import unittest
from pathlib import Path

try:
    import bpy
except ImportError:
    bpy = None

ROOT = Path(__file__).resolve().parent.parent
sys.path[:0] = [str(ROOT), str(ROOT / "blender_addon")]


@unittest.skipIf(bpy is None, "requires actual Blender")
class BlenderInputDefineTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        from tests.test_blender_rust_placements import RustEngine, TRANSFORM
        cls.engine = RustEngine(TRANSFORM)
        import minihou_link
        cls.addon = minihou_link
        try:
            minihou_link.register()
        except ValueError:
            pass   # already registered by another test module in this Blender session

    @classmethod
    def tearDownClass(cls):
        cls.engine.stop()

    def setUp(self):
        from minihou_link import geonodes, modifiers, proxy, input_define, embed
        from minihou_link.instance_client import InstanceClient
        self.g, self.m, self.proxy, self.d, self.embed = geonodes, modifiers, proxy, input_define, embed
        self.m.disconnect()
        bpy.ops.wm.read_factory_settings(use_empty=True)
        bpy.context.window_manager.minihou_port = self.engine.port
        self.refs = []
        original = InstanceClient.cook

        def cook(client, *args, **kw):
            self.refs.append(kw.get("references") or {})
            return original(client, *args, **kw)
        InstanceClient.cook = cook
        self.addCleanup(setattr, InstanceClient, "cook", original)
        self.addCleanup(self.m.disconnect)
        bpy.ops.mesh.primitive_plane_add()
        self.obj = bpy.context.object
        bpy.ops.mesh.primitive_cube_add(location=(3, 0, 0))
        self.target = bpy.context.object
        bpy.context.view_layer.objects.active = self.obj
        self.mod = self.m.add_modifier(self.obj, "", port=self.engine.port)
        self.ng = self.mod.node_group
        self.pump()

    def pump(self):
        for _ in range(4):
            self.m.pump(port=self.engine.port, force=True)
            if "did not reply within" not in self.m.last_error():
                break
        self.assertEqual(self.m.last_error(), "")

    def add(self, kind, name, **fields):
        result = bpy.ops.minihou.add_input(modifier=self.mod.name, input_name=name, kind=kind, **fields)
        self.assertEqual(result, {"FINISHED"})
        return next(e for e in reversed(self.d.network_inputs(self.ng)) if e["kind"] == kind)

    def item(self, name):
        return next(i for i in self.ng.interface.items_tree if i.item_type == "SOCKET" and i.name == name)

    def test_object_input_is_native_id_socket_with_eyedropper_and_reaches_the_cook(self):
        entry = self.add("object", "Target")
        item = self.item("Target")
        self.assertEqual(item.socket_type, "NodeSocketObject")
        self.assertIsNone(item.default_value)
        # The plumbing group node mirrors it with the same native socket type.
        plumbing = self.proxy.plumbing_node(self.ng)
        self.assertEqual(plumbing.inputs["Target"].bl_idname, "NodeSocketObject")
        # Modifier panel field: an ID pointer restricted to Object (what Blender draws with the eyedropper).
        owner, prop = self.proxy.input_owner(self.mod, item.identifier)
        if prop == "value":
            rna = owner.bl_rna.properties["value"]
            self.assertEqual((rna.type, rna.fixed_type.identifier), ("POINTER", "Object"))
        self.proxy.set_input(self.mod, item.identifier, self.target)
        self.assertEqual(self.proxy.get_input(self.mod, item.identifier), self.target)
        socket_rna = plumbing.inputs["Target"].bl_rna.properties["default_value"]
        self.assertEqual((socket_rna.type, socket_rna.fixed_type.identifier), ("POINTER", "Object"))
        self.pump()
        key = "input:" + entry["node"] + ":" + entry["id"]
        sent = [r[key] for r in self.refs if key in r]
        self.assertTrue(sent, self.refs)
        self.assertEqual(sent[-1]["name"], self.target.name)
        self.assertAlmostEqual(sent[-1]["matrix"][3], 3.0)       # evaluated world transform (row-major)
        self.proxy.set_input(self.mod, item.identifier, None)   # clearing the picker
        self.ng.update_tag(); self.obj.update_tag()
        self.pump()
        self.assertEqual([r[key] for r in self.refs if key in r][-1]["name"], "")

    def test_placement_section_draws_object_picker_and_add_input(self):
        from tests.test_blender_rust_placements import TRANSFORM
        from tests.test_blender_link_sync import RecordingLayout
        import minihou_link
        holder = bpy.data.objects.new("Holder", bpy.data.meshes.new("Holder"))
        bpy.context.collection.objects.link(holder)
        gn = holder.modifiers.new("GN", "NODES")
        tree = bpy.data.node_groups.new("GN", "GeometryNodeTree")
        tree.interface.new_socket("Geometry", in_out="INPUT", socket_type="NodeSocketGeometry")
        tree.interface.new_socket("Geometry", in_out="OUTPUT", socket_type="NodeSocketGeometry")
        gin, gout = tree.nodes.new("NodeGroupInput"), tree.nodes.new("NodeGroupOutput")
        gn.node_group = tree
        node = self.g.add_group(tree, str(TRANSFORM), port=self.engine.port)
        tree.links.new(gin.outputs[0], node.inputs["Geometry"])
        tree.links.new(node.outputs[0], gout.inputs[0])
        bpy.context.view_layer.objects.active = holder
        self.assertEqual(bpy.ops.minihou.add_input(tree=node.node_tree.name, input_name="Target", kind="object"), {"FINISHED"})
        self.assertEqual(node.inputs["Target"].bl_idname, "NodeSocketObject")
        self.assertIsNone(node.inputs["Target"].default_value)
        node.inputs["Target"].default_value = self.target
        section = self.g.placement_controls(gn)[0]
        layout = RecordingLayout()
        minihou_link.draw_placement(layout, gn, section)
        props = [(a[0], a[1]) for name, a, kw in layout.calls if name == "prop"]
        self.assertIn((node.inputs["Target"], "default_value"), props)
        rna = node.inputs["Target"].bl_rna.properties["default_value"]
        self.assertEqual((rna.type, rna.fixed_type.identifier), ("POINTER", "Object"))   # ID field + eyedropper
        self.assertIn(("operator", ("minihou.add_input",)), [(n, a) for n, a, kw in layout.calls])
        layout = RecordingLayout()
        self.addon.input_define_ui.draw(layout, self.ng, modifier=self.mod.name)
        self.assertEqual(layout.calls[1][:2], ("operator", ("minihou.add_input",)))

    def test_operator_defines_each_dialog_type_with_suffixes(self):
        want = {"float": "NodeSocketFloat", "int": "NodeSocketInt", "bool": "NodeSocketBool",
                "vector": "NodeSocketVector", "color": "NodeSocketColor", "string": "NodeSocketString",
                "menu": "NodeSocketMenu", "object": "NodeSocketObject", "collection": "NodeSocketCollection",
                "material": "NodeSocketMaterial", "image": "NodeSocketImage"}
        for kind, socket in want.items():
            self.add(kind, "Value", float_value=0.25, options="Low, High")
            name = self.d.network_inputs(self.ng)[-1]["name"]
            self.assertEqual(self.item(name).socket_type, socket, kind)
        names = [e["name"] for e in self.d.network_inputs(self.ng)]
        self.assertEqual(names[:3], ["Value", "Value 2", "Value 3"])
        self.assertAlmostEqual(self.item("Value").default_value, 0.25, places=6)
        self.assertEqual(self.item(names[want and list(want).index("menu")]).socket_type, "NodeSocketMenu")
        # Rename keeps the socket identifier (modifier values/drivers survive); remove drops it.
        ident = self.item("Value").identifier
        entry = self.d.network_inputs(self.ng)[0]
        bpy.ops.minihou.rename_input(modifier=self.mod.name, ident=entry["id"], new_name="Speed")
        self.assertEqual(self.item("Speed").identifier, ident)
        bpy.ops.minihou.remove_input(modifier=self.mod.name, ident=entry["id"])
        self.assertFalse(any(i.name == "Speed" for i in self.ng.interface.items_tree))
        self.assertNotIn("Speed", self.embed.text_of(self.ng).as_string())

    def test_object_input_survives_save_and_reopen(self):
        self.add("object", "Target")
        self.proxy.set_input(self.mod, self.item("Target").identifier, self.target)
        path = os.path.join(tempfile.mkdtemp(), "inputs.blend")
        bpy.ops.wm.save_as_mainfile(filepath=path)
        bpy.ops.wm.open_mainfile(filepath=path)
        obj = bpy.data.objects[self.obj.name] if False else bpy.data.objects["Plane"]
        mod = obj.modifiers["MiniHou"]
        item = next(i for i in mod.node_group.interface.items_tree if getattr(i, "name", "") == "Target")
        self.assertEqual(item.socket_type, "NodeSocketObject")
        self.assertEqual(self.proxy.get_input(mod, item.identifier).name, "Cube")
        stored = json.loads(self.embed.text_of(mod.node_group).as_string())
        self.assertEqual(stored["nodes"]["input"]["modifier_inputs"][0]["kind"], "object")

    def test_bound_app_defines_undoably_and_offline_definitions_reach_the_app(self):
        _, route = self.d.define(self.ng, self.d.request(input={"name": "Offline", "kind": "float", "default": 1.0}),
                                 mod=self.mod)
        self.assertEqual(route, "offline")                       # connected, but not bound in the app
        self.m.edit_live(self.obj, self.mod)                    # app binds the embedded graph (with Offline)
        entry, route = self.d.define(self.ng, self.d.request(input={"name": "Offline", "kind": "object"}), mod=self.mod)
        self.assertEqual((route, entry["name"]), ("app", "Offline 2"))   # the app saw the offline input
        self.assertEqual(self.item("Offline 2").socket_type, "NodeSocketObject")
        # Undo in the app removes the app-side edit; the live update brings it back to Blender.
        control = self.engine.control
        control.with_suffix(".tmp").write_text(json.dumps({"id": 10, "op": "undo"}), encoding="utf-8")
        control.with_suffix(".tmp").replace(control)
        deadline = time.time() + 10
        while time.time() < deadline and any(e["name"] == "Offline 2" for e in self.d.network_inputs(self.ng)):
            time.sleep(0.2)
            self.m.pump(port=self.engine.port, force=True)
        self.assertEqual([e["name"] for e in self.d.network_inputs(self.ng)], ["Offline"])


if __name__ == "__main__":
    result = unittest.main(argv=[sys.argv[0]], verbosity=2, exit=False).result
    sys.exit(0 if result.wasSuccessful() else 1)
