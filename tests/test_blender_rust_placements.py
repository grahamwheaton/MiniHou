"""Chained, independent wired GN placements against the RUST link server.

Runs inside headless Blender (5.1+; wired placements need Object.evaluated_geometry):

    blender --background --factory-startup --python tests/test_blender_rust_placements.py

The engine is the debug ``link_selftest`` Cargo example (production ``blender_link.rs``, no
window/GPU), started per test class on an ephemeral port. It never touches a running
minihou-app.exe. Build it first:

    cd rust; $env:CARGO_TARGET_DIR="..\\rust\\target_place"; cargo build -p minihou-app --example link_selftest

Override the executable with MINIHOU_LINK_SELFTEST. Set MINIHOU_RUST_FULL_GN=1 to also run
the whole tests/test_blender_geonodes.py placement suite against the Rust server.
"""
import json
import os
import queue
import subprocess
import sys
import tempfile
import threading
import unittest
from pathlib import Path

import numpy as np

try:
    import bpy
except ImportError:
    bpy = None

ROOT = Path(__file__).resolve().parent.parent
sys.path[:0] = [str(ROOT), str(ROOT / "blender_addon")]

TRANSFORM = ROOT / "examples" / "modifiers" / "transform.json"
EXTRUDE = {"version": 1, "nodes": {
    "input": {"type": "modifier_input"},
    "extrude": {"type": "polyextrude", "inputs": {"geo": "input"}},
    "transform": {"type": "transform", "inputs": {"geo": "extrude"}},
    "output": {"type": "output", "inputs": {"geo": "transform"}}},
    "display": "output",
    "exposed": [{"name": "Distance", "node": "extrude", "param": "distance"},
                {"name": "Scale", "node": "transform", "param": "uniform_scale"},
                {"name": "Translate", "node": "transform", "param": "translate"}]}
SOURCES = {"version": 1, "nodes": {
    "input": {"type": "modifier_input"},
    "asset": {"type": "object_input", "params": {"binding": "asset"}},
    "move": {"type": "transform", "inputs": {"geo": "asset"}},
    "merge": {"type": "merge", "inputs": {"geos": ["input", "move"]}},
    "output": {"type": "output", "inputs": {"geo": "merge"}}},
    "display": "output",
    "exposed": [{"name": "Offset", "node": "move", "param": "translate"}]}


def selftest_executable():
    explicit = os.environ.get("MINIHOU_LINK_SELFTEST")
    candidates = [Path(explicit)] if explicit else []
    for target in ("target_place", "target_inst"):
        candidates.append(ROOT / "rust" / target / "debug" / "examples" / "link_selftest.exe")
        candidates.append(ROOT / "rust" / target / "debug" / "examples" / "link_selftest")
    for path in candidates:
        if path.is_file():
            return path
    raise unittest.SkipTest("Build the Rust link_selftest example first (see module docstring)")


class RustEngine:
    """Same surface as tests.test_blender_addon.Engine (``port``/``stop``), Rust link server."""

    def __init__(self, graph_path, port=0, http_port=None):
        self.temp = tempfile.TemporaryDirectory(prefix="minihou-rust-gn-")
        self.control = Path(self.temp.name) / "control.json"
        self.log = open(Path(self.temp.name) / "server.log", "w", encoding="utf-8")
        self.proc = subprocess.Popen(
            [str(selftest_executable()), str(graph_path), f"127.0.0.1:{port}", "3600", str(self.control)],
            stdout=subprocess.PIPE, stderr=self.log, text=True,
            creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0))
        ready = queue.Queue()

        def read():
            for line in self.proc.stdout:
                if line.startswith("LINK_READY "):
                    ready.put(int(line.strip().rsplit(":", 1)[1]))
            ready.put(None)
        threading.Thread(target=read, daemon=True).start()
        self.port = ready.get(timeout=60)
        if self.port is None:
            self.stop()
            raise RuntimeError("Rust link_selftest failed to start")

    def stop(self):
        if self.proc.poll() is None:
            tmp = self.control.with_suffix(".tmp")
            tmp.write_text(json.dumps({"id": 1, "op": "stop"}), encoding="utf-8")
            tmp.replace(self.control)
            try:
                self.proc.wait(10)
            except subprocess.TimeoutExpired:
                self.proc.terminate()  # only our own example process
                self.proc.wait(5)
        if self.proc.stdout:
            self.proc.stdout.close()
        self.log.close()
        self.temp.cleanup()


def sorted_rows(points):
    points = np.asarray(points, np.float64).reshape(-1, 3)
    return points[np.lexsort(points.T[::-1])]


@unittest.skipIf(bpy is None, "requires actual Blender")
class RustChainedPlacementTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.tmp = tempfile.TemporaryDirectory()
        cls.extrude = Path(cls.tmp.name) / "extrude_transform.json"
        cls.extrude.write_text(json.dumps(EXTRUDE), encoding="utf-8")
        cls.sources = Path(cls.tmp.name) / "two_sources.json"
        cls.sources.write_text(json.dumps(SOURCES), encoding="utf-8")
        cls.engine = RustEngine(TRANSFORM)

    @classmethod
    def tearDownClass(cls):
        cls.engine.stop()
        cls.tmp.cleanup()

    def setUp(self):
        from minihou_link import geonodes, modifiers, proxy
        from minihou_link.instance_client import InstanceClient
        self.g, self.m, self.proxy = geonodes, modifiers, proxy
        self.g.unregister()
        self.m.disconnect()
        bpy.ops.wm.read_factory_settings(use_empty=True)
        # Record every instance cook the add-on sends to the Rust server (id, params, input, output).
        self.cooks = []
        original = InstanceClient.cook
        recorder = self.cooks

        def cook(client, instance_id, graph_path, params, arrays, **kw):
            header, output = original(client, instance_id, graph_path, params, arrays, **kw)
            recorder.append({"id": instance_id, "params": dict(params),
                             "input": np.array(arrays["point/P"], np.float64),
                             "output": np.array(output["point/P"], np.float64)})
            return header, output
        InstanceClient.cook = cook
        self.addCleanup(setattr, InstanceClient, "cook", original)

    def tearDown(self):
        self.g.unregister()
        self.m.disconnect()

    # -- helpers ------------------------------------------------------------------------------
    def plane(self, name="Plane"):
        mesh = bpy.data.meshes.new(name)
        mesh.from_pydata([(-1, -1, 0), (1, -1, 0), (1, 1, 0), (-1, 1, 0)], [], [(0, 1, 2, 3)])
        obj = bpy.data.objects.new(name, mesh)
        bpy.context.scene.collection.objects.link(obj)
        return obj

    def point(self, name, x):
        mesh = bpy.data.meshes.new(name)
        mesh.from_pydata([(x, 0, 0)], [], [])
        obj = bpy.data.objects.new(name, mesh)
        bpy.context.scene.collection.objects.link(obj)
        return obj

    def tree(self, name="Chained MiniHou"):
        tree = bpy.data.node_groups.new(name, "GeometryNodeTree")
        tree.interface.new_socket("Geometry", in_out="INPUT", socket_type="NodeSocketGeometry")
        tree.interface.new_socket("Geometry", in_out="OUTPUT", socket_type="NodeSocketGeometry")
        return tree, tree.nodes.new("NodeGroupInput"), tree.nodes.new("NodeGroupOutput")

    def attach(self, obj, tree):
        mod = obj.modifiers.new("MiniHou GN", "NODES")
        mod.node_group = tree
        return mod

    def chain(self, tree, *nodes):
        for first, second in zip(nodes, nodes[1:]):
            tree.links.new(next(s for s in first.outputs if s.type == "GEOMETRY"),
                           next(s for s in second.inputs if s.type == "GEOMETRY"))

    def add(self, tree, path=TRANSFORM, name=None, **values):
        node = self.g.add_group(tree, str(path), port=self.engine.port)
        if name:
            node.name = name
        for key, value in values.items():
            node.inputs[key].default_value = value
        return node

    def cook(self, force=False):
        del self.cooks[:]
        count = self.g.pump(port=self.engine.port, force=force)
        self.assertEqual(self.g.last_error(), "")
        bpy.context.view_layer.update()
        return count

    def ident(self, node):
        return node.node_tree[self.g.KEY]

    def order(self):
        return [c["id"] for c in self.cooks]

    def evaluated(self, obj):
        me = obj.evaluated_get(bpy.context.evaluated_depsgraph_get()).data
        a = np.empty(len(me.vertices) * 3, np.float32)
        me.vertices.foreach_get("co", a)
        return a.reshape(-1, 3)

    def node_editor(self, tree):
        area = bpy.context.screen.areas[0]
        old = area.type
        area.type = "NODE_EDITOR"
        area.ui_type = "GeometryNodeTree"
        space = area.spaces.active
        space.pin = True
        space.node_tree = tree
        region = next(r for r in area.regions if r.type == "WINDOW")
        self.addCleanup(setattr, area, "type", old)
        return area, region, space

    # -- tests ----------------------------------------------------------------------------------
    def test_chained_example_placements_cook_in_order_with_independent_controls(self):
        obj = self.plane()
        tree, gin, gout = self.tree()
        a = self.add(tree, name="MiniHou A", Scale=2.0)
        b = self.add(tree, name="MiniHou B", Translate=(0, 0, 3))
        # The example graph's exposed params appear as per-placement controls on the group node.
        for node in (a, b):
            self.assertEqual([s.name for s in node.inputs if s.name != "Geometry"][:3],
                             ["Translate", "Scale", "Rotate"])
            self.assertTrue(self.proxy.is_nested(node.node_tree))
            self.assertEqual(sorted(n.bl_idname for n in node.node_tree.nodes),
                             ["GeometryNodeGroup", "NodeGroupInput", "NodeGroupOutput"])
        self.assertNotEqual(self.ident(a), self.ident(b))
        self.chain(tree, gin, a, b, gout)
        self.attach(obj, tree)
        self.assertEqual(self.cook(), 2)
        self.assertEqual(self.order(), [self.ident(a), self.ident(b)])
        first, second = self.cooks
        self.assertEqual(first["params"]["Scale"], 2.0)
        self.assertEqual(second["params"]["Scale"], 1.0)
        self.assertEqual(second["params"]["Translate"], [0, 0, 3])
        # B received A's cooked output (through Blender) as its wired input mesh.
        np.testing.assert_allclose(sorted_rows(second["input"]), sorted_rows(first["output"]), atol=1e-6)
        expected = np.array([[-2, -2, 3], [2, -2, 3], [2, 2, 3], [-2, 2, 3]], np.float64)
        np.testing.assert_allclose(sorted_rows(self.evaluated(obj)), sorted_rows(expected), atol=1e-5)
        self.assertEqual(self.cook(), 0)  # unchanged: nothing recooks

        a.inputs["Scale"].default_value = 3.0  # upstream change: A and then B recook
        self.assertEqual(self.cook(), 2)
        self.assertEqual(self.order(), [self.ident(a), self.ident(b)])
        self.assertEqual(b.inputs["Scale"].default_value, 1.0)
        self.assertEqual(self.evaluated(obj)[:, 0].max(), 3.0)

        b.inputs["Translate"].default_value = (0, 0, 5)  # downstream change: only B recooks
        self.assertEqual(self.cook(), 1)
        self.assertEqual(self.order(), [self.ident(b)])
        self.assertEqual(a.inputs["Scale"].default_value, 3.0)
        np.testing.assert_allclose(self.evaluated(obj)[:, 2], 5, atol=1e-6)
        # Each placement owns a different private cache object.
        self.assertIsNot(self.proxy.cache_info(a.node_tree).inputs["Object"].default_value,
                         self.proxy.cache_info(b.node_tree).inputs["Object"].default_value)

    def test_polyextrude_distance_is_independent_per_placement(self):
        obj = self.plane()
        tree, gin, gout = self.tree()
        a = self.add(tree, self.extrude, name="Extrude A", Distance=0.5)
        b = self.add(tree, self.extrude, name="Extrude B", Distance=2.0)
        self.assertEqual({s.name for s in a.inputs} - {"Geometry"}, {"Distance", "Scale", "Translate"})
        self.chain(tree, gin, a, b, gout)
        self.attach(obj, tree)
        self.assertEqual(self.cook(), 2)
        first, second = self.cooks
        self.assertEqual((first["params"]["Distance"], second["params"]["Distance"]), (0.5, 2.0))
        self.assertEqual(len(first["input"]), 4)
        self.assertEqual(len(first["output"]), 8)  # plane extruded once by A
        np.testing.assert_allclose(first["output"][:, 2].max(), 0.5, atol=1e-6)
        np.testing.assert_allclose(sorted_rows(second["input"]), sorted_rows(first["output"]), atol=1e-6)
        result = self.evaluated(obj)
        np.testing.assert_allclose(sorted_rows(result), sorted_rows(second["output"]), atol=1e-5)
        self.assertGreater(len(result), 8)  # B extruded A's result again
        top = result[:, 2].max()
        a.inputs["Distance"].default_value = 1.0
        self.assertEqual(self.cook(), 2)
        self.assertEqual(self.order(), [self.ident(a), self.ident(b)])
        self.assertEqual(self.cooks[1]["params"]["Distance"], 2.0)
        self.assertAlmostEqual(self.evaluated(obj)[:, 2].max() - top, .5, places=4)
        b.inputs["Distance"].default_value = 0.25
        self.assertEqual(self.cook(), 1)
        self.assertEqual(a.inputs["Distance"].default_value, 1.0)

    def test_source_object_pickers_are_independent_per_placement(self):
        obj = self.point("Owner", 1)
        tree, gin, gout = self.tree()
        a = self.add(tree, self.sources, name="Sources A")
        b = self.add(tree, self.sources, name="Sources B")
        a.inputs["Object: asset"].default_value = self.point("Asset A", 4)
        b.inputs["Object: asset"].default_value = self.point("Asset B", 9)
        b.inputs["Offset"].default_value = (0, 0, 1)
        self.chain(tree, gin, a, b, gout)
        self.attach(obj, tree)
        self.assertEqual(self.cook(), 2)
        np.testing.assert_allclose(sorted_rows(self.evaluated(obj)),
                                   sorted_rows([[1, 0, 0], [4, 0, 0], [9, 0, 1]]), atol=1e-6)
        a.inputs["Object: asset"].default_value = self.point("Asset C", 6)
        self.assertEqual(self.cook(), 2)
        np.testing.assert_allclose(sorted_rows(self.evaluated(obj)),
                                   sorted_rows([[1, 0, 0], [6, 0, 0], [9, 0, 1]]), atol=1e-6)
        self.assertEqual(b.inputs["Object: asset"].default_value.name, "Asset B")

    def test_shift_d_duplicate_is_an_independent_placement(self):
        obj = self.plane()
        tree, gin, gout = self.tree()
        a = self.add(tree, name="MiniHou A", Scale=2.0)
        self.chain(tree, gin, a, gout)
        self.attach(obj, tree)
        self.cook()
        area, region, _ = self.node_editor(tree)
        for node in tree.nodes:
            node.select = node == a
        tree.nodes.active = a
        with bpy.context.temp_override(area=area, region=region):
            self.assertEqual(bpy.ops.node.duplicate(), {"FINISHED"})
        copy = tree.nodes.active
        self.assertNotEqual(copy, a)
        self.assertTrue(self.g.is_group(copy))
        # Immediately independent: own group data-block, identity and plumbing, no cook needed.
        self.assertIsNot(copy.node_tree, a.node_tree)
        self.assertNotEqual(self.ident(copy), self.ident(a))
        self.assertIsNot(self.proxy.plumbing(copy.node_tree), self.proxy.plumbing(a.node_tree))
        copy.inputs["Scale"].default_value = 5.0
        copy.inputs["Translate"].default_value = (0, 0, 1)
        self.chain(tree, a, copy, gout)
        self.assertEqual(self.cook(), 1)  # A is unchanged; only the new placement cooks
        self.assertEqual(self.order(), [self.ident(copy)])
        self.assertEqual(self.evaluated(obj)[:, 0].max(), 10.0)
        self.assertEqual(a.inputs["Scale"].default_value, 2.0)
        caches = {self.proxy.cache_info(n.node_tree).inputs["Object"].default_value for n in (a, copy)}
        self.assertEqual(len(caches), 2)
        a.inputs["Scale"].default_value = 1.0
        self.assertEqual(self.cook(), 2)
        self.assertEqual(self.order(), [self.ident(a), self.ident(copy)])
        self.assertEqual(self.evaluated(obj)[:, 0].max(), 5.0)

    def test_plain_shared_group_reference_and_operator_duplicate(self):
        obj = self.plane()
        tree, gin, gout = self.tree()
        a = self.add(tree, name="MiniHou A", Scale=2.0)
        shared = tree.nodes.new("GeometryNodeGroup")  # e.g. Add > Group > the MiniHou group
        shared.node_tree = a.node_tree
        shared.inputs["Scale"].default_value = 3.0
        self.chain(tree, gin, a, shared, gout)
        self.attach(obj, tree)
        self.assertEqual(self.cook(), 2)
        self.assertNotEqual(self.ident(a), self.ident(shared))
        self.assertEqual(self.evaluated(obj)[:, 0].max(), 6.0)
        # Sidebar/context-menu operator: explicit independent duplicate.
        import minihou_link
        minihou_link.register()
        self.addCleanup(minihou_link.unregister)
        area, region, _ = self.node_editor(tree)
        tree.nodes.active = shared
        with bpy.context.temp_override(area=area, region=region):
            self.assertEqual(bpy.ops.minihou.duplicate_placement(), {"FINISHED"})
        third = tree.nodes.active
        self.assertNotIn(self.ident(third), {self.ident(a), self.ident(shared)})
        self.assertEqual(third.inputs["Scale"].default_value, 3.0)
        third.inputs["Scale"].default_value = 0.5
        self.chain(tree, shared, third, gout)
        self.assertEqual(self.cook(), 1)
        self.assertEqual(self.evaluated(obj)[:, 0].max(), 3.0)

    def test_copy_paste_between_trees_creates_independent_placement(self):
        obj, other = self.plane("First"), self.plane("Second")
        tree, gin, gout = self.tree("First network")
        a = self.add(tree, name="MiniHou A", Scale=2.0)
        self.chain(tree, gin, a, gout)
        self.attach(obj, tree)
        self.cook()
        target, tin, tout = self.tree("Second network")
        self.attach(other, target)
        area, region, space = self.node_editor(tree)
        for node in tree.nodes:
            node.select = node == a
        with bpy.context.temp_override(area=area, region=region):
            self.assertEqual(bpy.ops.node.clipboard_copy(), {"FINISHED"})
        space.node_tree = target
        with bpy.context.temp_override(area=area, region=region):
            self.assertEqual(bpy.ops.node.clipboard_paste(), {"FINISHED"})
        pasted = next(n for n in target.nodes if self.g.is_group(n))
        self.assertIsNot(pasted.node_tree, a.node_tree)
        self.assertNotEqual(self.ident(pasted), self.ident(a))
        self.assertEqual(pasted.inputs["Scale"].default_value, 2.0)
        pasted.inputs["Scale"].default_value = 4.0
        self.chain(target, tin, pasted, tout)
        self.assertEqual(self.cook(), 1)
        self.assertEqual(self.order(), [self.ident(pasted)])
        self.assertEqual(self.evaluated(other)[:, 0].max(), 4.0)
        self.assertEqual(self.evaluated(obj)[:, 0].max(), 2.0)
        self.assertEqual(a.inputs["Scale"].default_value, 2.0)

    def test_stack_modifier_coexists_and_modifier_group_becomes_placement(self):
        obj = self.plane()
        stack = self.m.add_modifier(obj, str(TRANSFORM), port=self.engine.port)
        self.proxy.write_values(stack, stack.node_group, self.m.parameters(stack),
                                {"Scale": 2, "Translate": [0, 0, 0], "Rotate": [0, 0, 0]})
        self.assertTrue(self.proxy.is_nested(stack.node_group))
        # A second stack modifier's group, dropped into a GN tree, becomes an independent placement.
        donor = self.m.add_modifier(obj, str(TRANSFORM), port=self.engine.port)
        wrapper = donor.node_group
        obj.modifiers.remove(donor)
        tree, gin, gout = self.tree()
        a = self.add(tree, name="MiniHou A", Scale=3.0)
        wrapped = tree.nodes.new("GeometryNodeGroup")
        wrapped.node_tree = wrapper
        wrapped.inputs["Translate"].default_value = (0, 0, 7)
        self.chain(tree, gin, a, wrapped, gout)
        self.attach(obj, tree)
        try:
            self.assertEqual(self.cook(), 3)
            self.assertEqual(self.order()[1:], [self.ident(a), self.ident(wrapped)])
            self.assertIn(self.g.KEY, wrapped.node_tree)
            self.assertTrue(self.proxy.is_nested(wrapped.node_tree))
            result = self.evaluated(obj)
            self.assertEqual(result[:, 0].max(), 6.0)  # stack x2, then A x3
            np.testing.assert_allclose(result[:, 2], 7, atol=1e-6)
            self.proxy.write_values(stack, stack.node_group, self.m.parameters(stack),
                                    {"Scale": 1, "Translate": [0, 0, 0], "Rotate": [0, 0, 0]})
            self.assertEqual(self.cook(), 3)
            self.assertEqual(self.evaluated(obj)[:, 0].max(), 3.0)
        finally:
            self.m.unregister()


if os.environ.get("MINIHOU_RUST_FULL_GN") and bpy is not None:
    import tests.test_blender_geonodes as _gn

    class RustGeometryNodesPlacementTests(_gn.GeometryNodesPlacementTests):
        """The full staged-GN placement suite with the Rust link server as engine."""

        @classmethod
        def setUpClass(cls):
            _gn.Engine = RustEngine
            super().setUpClass()

        def setUp(self):
            from minihou_link import modifiers
            modifiers.disconnect()
            super().setUp()

        @unittest.skip("uses the Python engine's HTTP /api/edit editor; Rust live edit is covered by test_rust_link.py")
        def test_live_group_edit_uses_its_wired_mesh_and_updates_only_it(self):
            pass


if __name__ == "__main__":
    unittest.main(argv=[sys.argv[0]], verbosity=2)
