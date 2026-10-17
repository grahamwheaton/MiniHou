"""Blender Object node pickers and object-name search (add-on 0.5.0) in REAL Blender.

    $env:CARGO_TARGET_DIR="rust/target_objpicker"; cargo build --manifest-path rust/Cargo.toml -p minihou-app --example link_selftest
    $env:MINIHOU_LINK_SELFTEST="rust/target_objpicker/debug/examples/link_selftest.exe"
    blender --background --factory-startup --python-exit-code 1 --python tests/test_blender_object_picker.py

Run with plain python it skips (no bpy). The engine is the Rust link_selftest example (blender_link.rs).
Every ``blender_object`` node in a bound graph gets an Object picker (NodeSocketObject, named after the
node, keyed ``input:@picker:<stable id>``); the pick feeds the node; nothing picked = waiting (empty geometry,
no error). The search timings (1k/10k/100k empty objects) print as ``OBJECT_NAMES_TIMING``.
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

GRAPH = {"version": 1, "display": "output", "nodes": {
    "input": {"type": "modifier_input"},
    "blender_object1": {"type": "blender_object", "params": {"object": ""}},
    "merge": {"type": "merge", "inputs": {"geos": ["input", "blender_object1"]}},
    "output": {"type": "output", "inputs": {"geo": "merge"}}}}


def renamed(graph, old, new):
    g = json.loads(json.dumps(graph))
    node = g["nodes"].pop(old)
    node.setdefault("picker_id", old)          # what the app's rename does (graph_edit::rename)
    g["nodes"][new] = node
    g["nodes"]["merge"]["inputs"]["geos"] = ["input", new]
    return g


@unittest.skipIf(bpy is None, "requires actual Blender")
class BlenderObjectPickerTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        from tests.test_blender_rust_placements import RustEngine
        cls.tmp = tempfile.TemporaryDirectory()
        cls.graph_path = Path(cls.tmp.name) / "picker.json"
        cls.graph_path.write_text(json.dumps(GRAPH), encoding="utf-8")
        cls.engine = RustEngine(cls.graph_path)
        import minihou_link
        cls.addon = minihou_link
        try:
            minihou_link.register()
        except ValueError:
            pass

    @classmethod
    def tearDownClass(cls):
        cls.engine.stop()
        cls.tmp.cleanup()

    def setUp(self):
        from minihou_link import geonodes, modifiers, proxy, embed
        self.g, self.m, self.proxy, self.embed = geonodes, modifiers, proxy, embed
        self.m.disconnect()
        bpy.ops.wm.read_factory_settings(use_empty=True)
        bpy.context.window_manager.minihou_port = self.engine.port
        self.addCleanup(self.m.disconnect)
        bpy.ops.mesh.primitive_plane_add()
        self.obj = bpy.context.object
        bpy.ops.mesh.primitive_cube_add(location=(3, 0, 0))
        self.target = bpy.context.object
        bpy.context.view_layer.objects.active = self.obj
        self.mod = self.m.add_modifier(self.obj, str(self.graph_path), port=self.engine.port)
        self.ng = self.mod.node_group
        self.pump()

    def pump(self):
        for _ in range(4):
            self.m.pump(port=self.engine.port, force=True)
            if "did not reply within" not in self.m.last_error():
                break
        self.assertEqual(self.m.last_error(), "")

    def item(self, name):
        return next((i for i in self.ng.interface.items_tree if i.item_type == "SOCKET" and i.name == name), None)

    def verts(self, obj=None):
        obj = obj or self.obj
        return len(obj.evaluated_get(bpy.context.evaluated_depsgraph_get()).data.vertices)

    def touch(self):
        self.ng.update_tag(); self.obj.update_tag()
        bpy.context.view_layer.update()

    def test_picker_is_exposed_feeds_the_node_and_clearing_waits(self):
        item = self.item("blender_object1")
        self.assertIsNotNone(item, [i.name for i in self.ng.interface.items_tree])
        self.assertEqual(item.socket_type, "NodeSocketObject")
        self.assertEqual(self.proxy.plumbing_node(self.ng).inputs["blender_object1"].bl_idname, "NodeSocketObject")
        owner, prop = self.proxy.input_owner(self.mod, item.identifier)
        if prop == "value":
            rna = owner.bl_rna.properties["value"]
            self.assertEqual((rna.type, rna.fixed_type.identifier), ("POINTER", "Object"))   # eyedropper field
        self.assertEqual(self.verts(), 4, "nothing picked: waiting, empty geometry, no error")
        self.proxy.set_input(self.mod, item.identifier, self.target)
        self.touch(); self.pump()
        self.assertEqual(self.verts(), 12, "plane + picked cube's evaluated mesh")
        self.proxy.set_input(self.mod, item.identifier, None)
        self.touch(); self.pump()
        self.assertEqual(self.verts(), 4)
        self.assertEqual(self.m.last_error(), "")

    def test_rename_keeps_socket_and_value_remove_drops_it(self):
        item = self.item("blender_object1")
        ident = item.identifier
        self.proxy.set_input(self.mod, ident, self.target)
        self.touch(); self.pump()
        graph = renamed(json.loads(self.ng[self.m.GRAPH]), "blender_object1", "rock")
        self.ng[self.m.GRAPH] = json.dumps(graph)
        self.embed.write(self.ng)
        self.touch(); self.pump()
        self.touch(); self.pump()      # the first cook returns the new interface; the next sends the renamed socket
        moved = self.item("rock")
        self.assertIsNotNone(moved, [i.name for i in self.ng.interface.items_tree])
        self.assertEqual(moved.identifier, ident, "same socket: value, links and drivers survive")
        self.assertEqual(self.proxy.get_input(self.mod, ident), self.target)
        self.assertEqual(self.verts(), 12)
        graph["nodes"].pop("rock")
        graph["nodes"]["merge"]["inputs"]["geos"] = ["input"]
        self.ng[self.m.GRAPH] = json.dumps(graph)
        self.embed.write(self.ng)
        self.touch(); self.pump()
        self.assertIsNone(self.item("rock"))
        self.assertEqual(self.verts(), 4)

    def test_pick_survives_save_and_reopen(self):
        self.proxy.set_input(self.mod, self.item("blender_object1").identifier, self.target)
        self.touch(); self.pump()
        path = os.path.join(tempfile.mkdtemp(), "picker.blend")
        bpy.ops.wm.save_as_mainfile(filepath=path)
        bpy.ops.wm.open_mainfile(filepath=path)
        mod = bpy.data.objects["Plane"].modifiers["MiniHou"]
        item = next(i for i in mod.node_group.interface.items_tree if getattr(i, "name", "") == "blender_object1")
        self.assertEqual(item.socket_type, "NodeSocketObject")
        self.assertEqual(self.proxy.get_input(mod, item.identifier).name, "Cube")

    def test_placement_section_has_its_own_picker(self):
        from tests.test_blender_link_sync import RecordingLayout
        holder = bpy.data.objects.new("Holder", bpy.data.meshes.new("Holder"))
        bpy.context.collection.objects.link(holder)
        gn = holder.modifiers.new("GN", "NODES")
        tree = bpy.data.node_groups.new("GN", "GeometryNodeTree")
        tree.interface.new_socket("Geometry", in_out="INPUT", socket_type="NodeSocketGeometry")
        tree.interface.new_socket("Geometry", in_out="OUTPUT", socket_type="NodeSocketGeometry")
        gin, gout = tree.nodes.new("NodeGroupInput"), tree.nodes.new("NodeGroupOutput")
        gn.node_group = tree
        node = self.g.add_group(tree, str(self.graph_path), port=self.engine.port)
        tree.links.new(gin.outputs[0], node.inputs["Geometry"])
        tree.links.new(node.outputs[0], gout.inputs[0])
        self.assertEqual(node.inputs["blender_object1"].bl_idname, "NodeSocketObject")
        node.inputs["blender_object1"].default_value = self.target
        section = self.g.placement_controls(gn)[0]
        layout = RecordingLayout()
        self.addon.draw_placement(layout, gn, section)
        props = [(a[0], a[1]) for name, a, kw in layout.calls if name == "prop"]
        self.assertIn((node.inputs["blender_object1"], "default_value"), props)
        if bpy.app.version < (5, 1, 0):
            return                     # placement cooks need Object.evaluated_geometry (5.1+)
        holder.update_tag(); bpy.context.view_layer.update()
        self.pump()
        self.assertEqual(self.verts(holder), 8, "placement cooks its own pick")


@unittest.skipIf(bpy is None, "requires actual Blender")
class ObjectNameSearchTimingTests(unittest.TestCase):
    """Budgeted search over 1k / 10k / 100k empties; generous ceilings, real numbers printed."""

    def test_search_is_ranked_time_sliced_and_bounded(self):
        from minihou_link import object_names as on
        bpy.ops.wm.read_factory_settings(use_empty=True)
        made = 0
        report = []
        for size in (1_000, 10_000, 100_000):
            t0 = time.perf_counter()
            for i in range(made, size):
                bpy.data.objects.new(f"Empty_{i:06d}", None)
            made = size
            build_s = time.perf_counter() - t0
            objects = bpy.data.objects
            cold, partials, ticks, max_slice = on.run_blocking("y_000123", objects)
            searcher = on.Searcher()
            out = []
            searcher.start({"request_id": 1, "query": "x"}, objects)
            while searcher.step(out.append):
                pass
            searcher.start({"request_id": 2, "query": "y_0001"}, objects)        # warm: cached names
            job = searcher.job
            warm_ticks = 1
            while searcher.step(out.append):
                warm_ticks += 1
            warm = out[-1]
            empty, _, empty_ticks, _ = on.run_blocking("", objects)
            report.append(f"{size}: cold {cold['ms']:.1f} ms/{ticks} ticks (max slice {max_slice:.2f} ms), "
                          f"warm {warm['ms']:.1f} ms/{warm_ticks} ticks (max slice {job.max_slice_ms:.2f} ms), "
                          f"empty query {empty['ms']:.2f} ms; created in {build_s:.1f} s")
            self.assertEqual(cold["count"], size)
            self.assertFalse(cold["partial"])
            self.assertLessEqual(len(cold["matches"]), on.LIMIT)
            self.assertEqual(cold["matches"][0]["name"], "Empty_000123", cold["matches"][:3])
            self.assertEqual((cold["matches"][0]["type"], cold["matches"][0]["mesh"]), ("EMPTY", False))
            self.assertTrue(all(m["name"].startswith("Empty_0001") for m in warm["matches"][:10]), warm["matches"][:3])
            self.assertEqual(len(empty["matches"]), on.LIMIT)
            self.assertEqual(empty_ticks, 1, "empty query reads only the first names")
            # Budget: each tick stays near BUDGET_MS (one slice may overrun by its own length); generous ceiling.
            self.assertLess(max_slice, 50.0)
            self.assertLess(job.max_slice_ms, 50.0)
            self.assertLess(cold["ms"], 20_000.0)
        print("OBJECT_NAMES_TIMING " + " | ".join(report))
        # A newer query replaces the running job; its answer carries the newer request id.
        searcher = on.Searcher()
        out = []
        searcher.start({"request_id": 5, "query": "a"}, bpy.data.objects)
        searcher.step(out.append, 0.5)
        searcher.start({"request_id": 6, "query": "b"}, bpy.data.objects)
        while searcher.step(out.append):
            pass
        self.assertEqual(out[-1]["request_id"], 6)
        self.assertNotIn(5, [o["request_id"] for o in out if not o["partial"]])


if __name__ == "__main__":
    result = unittest.main(argv=[sys.argv[0]], verbosity=2, exit=False).result
    sys.exit(0 if result.wasSuccessful() else 1)
