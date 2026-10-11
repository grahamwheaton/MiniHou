"""Addon tests that need Blender (``bpy``). Skipped when bpy is not importable.

Run inside Blender:      blender --background --factory-startup --python tools/blender_selftest.py
or with the bpy module:  python -m unittest tests.test_blender_addon

The engine runs as a *separate process* (``python -m minihou serve``); the addon talks to it over a real
loopback socket, and results are checked on Blender's evaluated depsgraph (i.e. through the Geometry Nodes
proxy), not just on the raw output mesh.
"""
import json
import os
import re
import socket
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from pathlib import Path

import numpy as np

try:
    import bpy
except ImportError:                                    # pragma: no cover
    bpy = None

ROOT = Path(__file__).resolve().parent.parent
sys.path[:0] = [str(ROOT), str(ROOT / "blender_addon")]

GRAPH = {
    "version": 1,
    "nodes": {
        "g": {"type": "grid", "params": {"columns": 30, "rows": 30, "size_x": 6, "size_y": 6}},
        "n": {"type": "noise", "params": {"amplitude": [0, 0, 1], "frequency": 0.4, "seed": 3}, "inputs": {"geo": "g"}},
        "o": {"type": "output", "inputs": {"geo": "n"}},
    },
    "display": "o",
    "exposed": [
        {"name": "Height", "node": "n", "param": "amplitude"},
        {"name": "Frequency", "node": "n", "param": "frequency"},
        {"name": "Columns", "node": "g", "param": "columns"},
        {"name": "Seed", "node": "n", "param": "seed"},
    ],
}


def free_port():
    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    p = s.getsockname()[1]
    s.close()
    return p


class Engine:
    """``python -m minihou serve`` in a subprocess."""

    def __init__(self, graph_path, port=0):
        self.proc = subprocess.Popen([os.environ.get("MINIHOU_ENGINE_PYTHON", sys.executable), "-u", "-m", "minihou", "serve", str(graph_path), "--port", str(port), "--http-port", "-1"],
                                     cwd=str(ROOT), stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True,
                                     encoding="utf-8", errors="replace")
        seen = []
        m = None
        for _ in range(50):                    # startup may print other lines (e.g. compute backend) first
            line = self.proc.stdout.readline()
            if not line:
                break
            seen.append(line)
            m = re.search(r"on 127\.0\.0\.1:(\d+)", line)
            if m:
                break
        if not m:
            self.proc.kill()
            raise RuntimeError("engine failed to start:\n" + "".join(seen))
        self.port = int(m.group(1))

    def stop(self):
        if self.proc.poll() is None:
            self.proc.terminate()
            try:
                self.proc.wait(5)
            except subprocess.TimeoutExpired:
                self.proc.kill()
        if self.proc.stdout:
            self.proc.stdout.close()


def evaluated_positions(obj):
    dg = bpy.context.evaluated_depsgraph_get()
    me = obj.evaluated_get(dg).data
    a = np.empty(len(me.vertices) * 3, np.float32)
    me.vertices.foreach_get("co", a)
    return a.reshape(-1, 3)


@unittest.skipIf(bpy is None, "needs Blender / the bpy module")
class AddonIntegrationTests(unittest.TestCase):
    def setUp(self):
        bpy.ops.wm.read_factory_settings(use_empty=True)
        import minihou_link
        from minihou_link import proxy, runtime
        minihou_link.register()
        self.minihou_link, self.proxy, self.runtime = minihou_link, proxy, runtime
        self.tmp = tempfile.TemporaryDirectory()
        self.graph_path = Path(self.tmp.name) / "g.json"
        self.graph_path.write_text(json.dumps(GRAPH), encoding="utf-8")
        self.engine = Engine(self.graph_path)
        self.rt = runtime.get_runtime()
        self.rt.connect("127.0.0.1", self.engine.port)

    def tearDown(self):
        self.minihou_link.unregister()
        self.engine.stop()
        self.tmp.cleanup()

    # -- helpers -----------------------------------------------------------
    def pump(self, pred, timeout=15.0):
        end = time.monotonic() + timeout
        while time.monotonic() < end:
            self.rt.tick()
            if pred():
                return True
            time.sleep(0.005)
        self.fail(f"timed out; last_error={self.rt.last_error!r} state={self.rt.state}")

    def settle(self, cycles=30):
        """Tick + re-evaluate the depsgraph repeatedly, like a live session would."""
        for _ in range(cycles):
            bpy.context.view_layer.update()
            self.rt.tick()
            time.sleep(0.01)

    # The engine may run numba while this process (Blender's Python) computes references with numpy: the two backends
    # agree to ~2e-6, so compare with TOL. And a cold numba engine compiles its kernels on first use (seconds on a fresh
    # Windows runner), so never assume a result arrives within a fixed number of ticks: wait for convergence.
    TOL = 2e-5

    def expected_P(self, **exposed):
        """Reference cook of GRAPH with exposed-parameter overrides (use the values actually forwarded)."""
        from minihou import Evaluator, Graph
        g = Graph.from_dict(GRAPH)
        for name, v in exposed.items():
            g.set_exposed(name, list(v) if isinstance(v, tuple) else v)
        return Evaluator().cook(g).P

    def mesh_matches(self, want):
        got = evaluated_positions(bpy.data.objects[self.proxy.PROXY_OBJECT])
        return got.shape == want.shape and np.allclose(got, want, atol=self.TOL)

    def pump_until_mesh(self, want, timeout=90.0):
        self.pump(lambda: self.mesh_matches(want), timeout)

    def modifier(self):
        p = bpy.data.objects[self.proxy.PROXY_OBJECT]
        return p.modifiers[self.proxy.MODIFIER]

    def set_modifier_value(self, name, value):
        mod = self.modifier()
        self.proxy.set_input(mod, self.proxy.mapping(mod.node_group)[name], value)
        bpy.context.view_layer.update()              # fires depsgraph_update_post, like a UI edit

    # -- tests -------------------------------------------------------------
    def test_connect_creates_proxy_and_mesh_matches_engine(self):
        self.pump(lambda: self.rt.applied_count >= 1)
        out = bpy.data.objects[self.proxy.OUTPUT_OBJECT]
        proxy_obj = bpy.data.objects[self.proxy.PROXY_OBJECT]
        self.assertTrue(out.hide_get())
        self.assertEqual(len(out.data.vertices), 900)
        self.assertEqual(len(out.data.polygons), 29 * 29)
        np.testing.assert_allclose(evaluated_positions(proxy_obj), self.expected_P(), atol=self.TOL)
        self.assertEqual(len(proxy_obj.evaluated_get(bpy.context.evaluated_depsgraph_get()).data.polygons), 29 * 29)
        self.assertIn("uv", out.data.attributes)

    def test_ensure_objects_in_steady_state_does_not_dirty_the_depsgraph(self):
        """Regression: it runs on every geometry update; any bpy write there forces a re-evaluation."""
        self.pump(lambda: self.rt.applied_count >= 1)
        self.settle(5)
        seen = []

        def spy(scene, depsgraph):
            seen.extend(u.id.name for u in depsgraph.updates)

        bpy.app.handlers.depsgraph_update_post.append(spy)
        try:
            bpy.context.view_layer.update()
            seen.clear()
            self.proxy.ensure_objects()
            bpy.context.view_layer.update()
        finally:
            bpy.app.handlers.depsgraph_update_post.remove(spy)
        self.assertEqual(seen, [])

    def test_timer_is_registered_and_stops_itself_when_disconnected(self):
        self.assertTrue(bpy.app.timers.is_registered(self.runtime.timer_entry))
        self.assertIsInstance(self.runtime.timer_entry(), float)         # next delay
        self.rt.disconnect()
        self.assertIsNone(self.runtime.timer_entry())                    # None = Blender unregisters it

    def test_node_group_inputs_mirror_exposed_parameters(self):
        self.pump(lambda: self.rt.applied_count >= 1)
        mod = self.modifier()
        m = self.proxy.mapping(mod.node_group)
        self.assertEqual(list(m), ["Height", "Frequency", "Columns", "Seed"])
        ng = mod.node_group
        sockets = {i.name: i for i in ng.interface.items_tree if i.item_type == "SOCKET" and i.in_out == "INPUT"}
        self.assertEqual(sockets["Columns"].min_value, 2)
        self.assertAlmostEqual(sockets["Frequency"].default_value, 0.4, places=6)
        self.assertEqual(tuple(self.proxy.get_input(mod, m["Height"])), (0.0, 0.0, 1.0))
        self.assertEqual(self.proxy.get_input(mod, m["Columns"]), 30)

    def test_editing_a_modifier_input_updates_the_engine_and_the_mesh(self):
        self.pump(lambda: self.rt.applied_count >= 1)
        self.set_modifier_value("Height", (0.0, 0.0, 2.5))
        self.pump(lambda: self.rt.sent_count >= 1)
        self.pump_until_mesh(self.expected_P(Height=self.rt.forwarder.known["Height"]))
        self.settle(10)
        self.assertEqual(self.rt.sent_count, 1)

    def test_position_only_update_does_not_rebuild_topology(self):
        self.pump(lambda: self.rt.applied_count >= 1)
        self.assertTrue(self.rt.last_timings["rebuilt"])
        before = self.rt.applied_count
        self.set_modifier_value("Frequency", 0.9)
        self.pump(lambda: self.rt.applied_count > before)
        self.assertFalse(self.rt.last_timings["rebuilt"])
        before = self.rt.applied_count
        self.set_modifier_value("Columns", 12)
        self.pump(lambda: self.rt.applied_count > before)
        self.assertTrue(self.rt.last_timings["rebuilt"])
        self.assertEqual(len(bpy.data.objects[self.proxy.OUTPUT_OBJECT].data.vertices), 12 * 30)

    def test_no_depsgraph_feedback_loop(self):
        self.pump(lambda: self.rt.applied_count >= 1)
        self.set_modifier_value("Seed", 11)
        self.pump(lambda: self.rt.sent_count >= 1)
        self.pump_until_mesh(self.expected_P(Seed=11))      # the edit's result has landed
        self.settle(10)
        applied, sent = self.rt.applied_count, self.rt.sent_count
        self.settle(80)                                # many depsgraph updates caused by our own mesh writes
        self.assertEqual(self.rt.sent_count, sent)
        self.assertEqual(self.rt.applied_count, applied)
        self.assertFalse(self.rt.forwarder.tripped)

    def test_rapid_edits_are_coalesced(self):
        self.pump(lambda: self.rt.applied_count >= 1)
        for i in range(40):
            self.set_modifier_value("Frequency", 0.1 + i * 0.02)
            self.rt.tick()
        self.settle(10)
        self.pump(lambda: abs(self.rt.forwarder.known["Frequency"] - (0.1 + 39 * 0.02)) < 1e-6, 30)
        self.pump_until_mesh(self.expected_P(Frequency=self.rt.forwarder.known["Frequency"]))
        self.assertLess(self.rt.sent_count, 40)

    def test_invalid_value_is_reported_without_looping(self):
        self.pump(lambda: self.rt.applied_count >= 1)
        # Bypass the socket's own min (2) by editing the engine graph limits: send raw out-of-range value.
        self.rt.client.set_param("Columns", 1)
        self.pump(lambda: "minimum" in self.rt.last_error)
        sent = self.rt.sent_count
        self.settle(20)
        self.assertEqual(self.rt.sent_count, sent)

    def test_hot_reload_rebuilds_the_interface(self):
        self.pump(lambda: self.rt.applied_count >= 1)
        G = json.loads(json.dumps(GRAPH))
        G["exposed"].append({"name": "Size", "node": "g", "param": "size_x"})
        G["nodes"]["n"]["params"]["frequency"] = 0.8
        time.sleep(0.1)
        self.graph_path.write_text(json.dumps(G), encoding="utf-8")
        self.pump(lambda: "Size" in self.proxy.mapping(self.modifier().node_group))
        mod = self.modifier()
        m = self.proxy.mapping(mod.node_group)
        self.assertAlmostEqual(self.proxy.get_input(mod, m["Frequency"]), 0.8, places=6)       # engine value pushed into the modifier
        self.assertEqual(self.proxy.get_input(mod, m["Size"]), 6.0)
        self.pump(lambda: abs(self.rt.forwarder.known.get("Frequency", 0) - 0.8) < 1e-9)
        sent = self.rt.sent_count
        self.settle(20)
        self.assertEqual(self.rt.sent_count, sent)                       # no echo of the engine's own values
        np.testing.assert_allclose(evaluated_positions(bpy.data.objects[self.proxy.PROXY_OBJECT]).shape, (900, 3))

    def test_reconnects_when_the_engine_restarts(self):
        self.pump(lambda: self.rt.applied_count >= 1)
        port = self.engine.port
        self.engine.stop()
        self.pump(lambda: self.rt.state != "connected", timeout=10)
        n = len(bpy.data.objects[self.proxy.OUTPUT_OBJECT].data.vertices)
        self.assertEqual(n, 900)                                        # last mesh stays while disconnected
        self.engine = Engine(self.graph_path, port=port)
        applied = self.rt.applied_count
        self.pump(lambda: self.rt.applied_count > applied and self.rt.state == "connected", timeout=20)
        np.testing.assert_allclose(evaluated_positions(bpy.data.objects[self.proxy.PROXY_OBJECT]), self.expected_P(), atol=1e-6)

    def test_bpy_use_off_the_main_thread_is_refused(self):
        from minihou_link import mesh_writer
        errors = []

        def worker():
            try:
                mesh_writer.assert_main_thread()
            except RuntimeError as e:
                errors.append(e)

        t = threading.Thread(target=worker)
        t.start()
        t.join()
        self.assertEqual(len(errors), 1)
        mesh = bpy.data.meshes.new("x")
        from minihou_link.protocol import GeoUpdate, Mirror
        errs = []

        def worker2():
            try:
                mesh_writer.apply_update(mesh, Mirror(), GeoUpdate())
            except RuntimeError as e:
                errs.append(e)

        t = threading.Thread(target=worker2)
        t.start()
        t.join()
        self.assertEqual(len(errs), 1)

    def test_socket_thread_never_calls_bpy(self):
        """The inbox callback runs on the client thread; make sure it never touches bpy."""
        calls = []
        orig = self.rt.inbox.push

        def spy(header, arrays):
            calls.append(threading.current_thread() is threading.main_thread())
            return orig(header, arrays)

        self.rt.client.on_message = spy
        self.rt.client.resync()
        self.pump(lambda: len(calls) >= 1)
        self.assertFalse(any(calls))                                    # all on the socket thread


@unittest.skipIf(bpy is None, "needs Blender / the bpy module")
class InterfaceSyncTests(unittest.TestCase):
    """proxy.sync_interface must change the node group incrementally (a full rebuild crashed Blender when a panel
    showing the modifier inputs was open -- see tools/gui_smoketest.py)."""

    def setUp(self):
        bpy.ops.wm.read_factory_settings(use_empty=True)
        from minihou_link import proxy
        self.proxy = proxy
        self.out, self.obj, self.mod = proxy.ensure_objects()
        self.ng = self.mod.node_group

    def p(self, name, kind="float", value=1.0, **kw):
        return dict(name=name, kind=kind, value=value, **kw)

    def inputs(self):
        return [(i.name, i.socket_type) for i in self.ng.interface.items_tree if i.item_type == "SOCKET" and i.in_out == "INPUT"]

    def test_kept_inputs_keep_identifier_and_value(self):
        self.proxy.sync_interface(self.ng, self.out, [self.p("A"), self.p("B", "int", 3)], "s1")
        m1 = self.proxy.mapping(self.ng)
        self.proxy.set_input(self.mod, m1["A"], 7.5)
        self.proxy.sync_interface(self.ng, self.out, [self.p("A"), self.p("C", "vec3", [0, 0, 1]), self.p("B", "int", 3)], "s2")
        m2 = self.proxy.mapping(self.ng)
        self.assertEqual((m2["A"], m2["B"]), (m1["A"], m1["B"]))
        self.assertEqual(self.proxy.get_input(self.mod, m2["A"]), 7.5)
        self.assertEqual(self.inputs(), [("Geometry", "NodeSocketGeometry"), ("A", "NodeSocketFloat"),
                                         ("C", "NodeSocketVector"), ("B", "NodeSocketInt")])
        self.assertTrue(self.proxy.has_input(self.mod, m2["C"]))

    def test_removed_and_retyped_inputs(self):
        self.proxy.sync_interface(self.ng, self.out, [self.p("A"), self.p("B"), self.p("C")], "s1")
        old = self.proxy.mapping(self.ng)
        self.proxy.sync_interface(self.ng, self.out, [self.p("C"), self.p("B", "bool", True)], "s2")
        m = self.proxy.mapping(self.ng)
        self.assertEqual(self.inputs(), [("Geometry", "NodeSocketGeometry"), ("C", "NodeSocketFloat"), ("B", "NodeSocketBool")])
        self.assertEqual(m["C"], old["C"])
        self.assertNotEqual(m["B"], old["B"])                       # type changed -> new socket
        self.assertFalse(self.proxy.has_input(self.mod, old["A"]))
        self.assertNotIn("A", m)
        self.assertEqual(len([n for n in self.ng.nodes if n.name.startswith(self.proxy.USE_NODE_PREFIX)]), 2)
        self.assertIsInstance(self.proxy.get_input(self.mod, m["B"]), bool)              # the property exists with the new type...
        sock = next(i for i in self.ng.interface.items_tree if getattr(i, "identifier", "") == m["B"])
        self.assertIs(sock.default_value, True)                     # ...values themselves are written by the runtime

    def test_keyframes_and_drivers_on_inputs_survive_a_reload(self):
        """Users animate modifier inputs; a reload that keeps an input must keep its animation working."""
        self.proxy.sync_interface(self.ng, self.out, [self.p("A"), self.p("B")], "s1")
        ident = self.proxy.mapping(self.ng)["A"]
        path = self.proxy.input_data_path(self.mod, ident)
        self.proxy.set_input(self.mod, ident, 1.0)
        self.obj.keyframe_insert(data_path=path, frame=1)
        self.proxy.set_input(self.mod, ident, 9.0)
        self.obj.keyframe_insert(data_path=path, frame=10)
        self.proxy.sync_interface(self.ng, self.out, [self.p("C"), self.p("A")], "s2")   # B gone, C added, reordered
        self.assertEqual(self.proxy.mapping(self.ng)["A"], ident)
        scene = bpy.context.scene
        scene.frame_set(10)
        self.assertAlmostEqual(self.proxy.get_input(self.mod, ident), 9.0, places=5)
        scene.frame_set(1)
        self.assertAlmostEqual(self.proxy.get_input(self.mod, ident), 1.0, places=5)

    def test_reordering_many_inputs(self):
        names = [f"P{i}" for i in range(8)]
        self.proxy.sync_interface(self.ng, self.out, [self.p(n) for n in names], "s1")
        rev = list(reversed(names))
        self.proxy.sync_interface(self.ng, self.out, [self.p(n) for n in rev], "s2")
        self.assertEqual([n for n, _ in self.inputs()[1:]], rev)

    def test_limits_and_defaults(self):
        self.proxy.sync_interface(self.ng, self.out, [self.p("N", "int", 5, min=2, max=50)], "s1")
        sock = next(i for i in self.ng.interface.items_tree if i.item_type == "SOCKET" and i.name == "N")
        self.assertEqual((sock.default_value, sock.min_value, sock.max_value), (5, 2, 50))

    def test_repeating_the_same_sync_writes_nothing(self):
        params = [self.p("A", min=0.0, max=10.0), self.p("V", "vec3", [1, 2, 3])]
        self.proxy.sync_interface(self.ng, self.out, params, "s1")
        bpy.context.view_layer.update()
        seen = []

        def spy(scene, depsgraph):
            seen.extend(u.id.name for u in depsgraph.updates)

        bpy.app.handlers.depsgraph_update_post.append(spy)
        try:
            self.proxy.sync_interface(self.ng, self.out, params, "s1")
            bpy.context.view_layer.update()
        finally:
            bpy.app.handlers.depsgraph_update_post.remove(spy)
        self.assertEqual(seen, [])

    def test_every_input_feeds_the_output_path_and_output_is_unchanged(self):
        params = [self.p("A"), self.p("V", "vec3", [0, 0, 1]), self.p("N", "int", 2), self.p("F", "bool", False)]
        self.proxy.sync_interface(self.ng, self.out, params, "s1")
        m = self.proxy.mapping(self.ng)
        gout = next(n for n in self.ng.nodes if n.bl_idname == "NodeGroupOutput")
        # walk back from the output: Object Info must be reached through one store node per input
        node, seen = gout.inputs["Geometry"].links[0].from_node, []
        while node.bl_idname == "GeometryNodeStoreNamedAttribute":
            seen.append(node.inputs["Value"].links[0].from_socket.identifier)
            node = node.inputs["Geometry"].links[0].from_node
        self.assertEqual(node.bl_idname, "GeometryNodeObjectInfo")
        self.assertEqual(sorted(seen), sorted(m.values()))
        sock = next(i for i in self.ng.interface.items_tree if getattr(i, "identifier", "") == m["A"])
        self.assertEqual(sock.structure_type, "SINGLE")
        # remove one input: its store node goes, chain stays intact
        self.proxy.sync_interface(self.ng, self.out, [params[0], params[2]], "s2")
        stores = [n for n in self.ng.nodes if n.name.startswith(self.proxy.USE_NODE_PREFIX)]
        self.assertEqual(len(stores), 2)
        # the proxy's evaluated mesh has no extra attributes
        import numpy as np
        me = self.out.data
        me.clear_geometry()
        me.vertices.add(3)
        me.attributes["position"].data.foreach_set("vector", np.arange(9, dtype=np.float32))
        me.update()
        ev = self.obj.evaluated_get(bpy.context.evaluated_depsgraph_get()).data
        self.assertEqual(len(ev.vertices), 3)
        self.assertEqual(sorted(a.name for a in ev.attributes if not a.name.startswith(".")), ["position"])

    def test_object_info_still_wired(self):
        self.proxy.sync_interface(self.ng, self.out, [self.p("A")], "s1")
        info = next(n for n in self.ng.nodes if n.bl_idname == "GeometryNodeObjectInfo")
        self.assertEqual(info.inputs["Object"].default_value.name, self.proxy.OUTPUT_OBJECT)
        self.assertEqual(sum(1 for l in self.ng.links if l.from_node == info), 1)
        self.proxy.sync_interface(self.ng, self.out, [], "s0")          # no inputs: Object Info straight to output
        gout = next(n for n in self.ng.nodes if n.bl_idname == "NodeGroupOutput")
        self.assertEqual(gout.inputs["Geometry"].links[0].from_node.bl_idname, "GeometryNodeObjectInfo")


@unittest.skipIf(bpy is None, "needs Blender / the bpy module")
class MeshWriterTests(unittest.TestCase):
    """mesh_writer in isolation, with hand-built updates (no engine, no socket)."""

    def setUp(self):
        bpy.ops.wm.read_factory_settings(use_empty=True)
        from minihou_link import mesh_writer
        self.w = mesh_writer
        self.mesh = bpy.data.meshes.new("t")
        from minihou.link.protocol import Mirror
        self.mirror = Mirror()

    def apply(self, geo, differ):
        from minihou.link.protocol import GeoUpdate
        h, arrays = differ.build(geo, differ_seq(differ))
        u = GeoUpdate.from_message(h, arrays)
        self.mirror.apply(u)
        return self.w.apply_update(self.mesh, self.mirror, u)

    def make_geo(self):
        from minihou import Geometry
        P = np.array([[0, 0, 0], [1, 0, 0], [1, 1, 0], [0, 1, 0], [2, 0, 0], [2, 1, 0]], np.float32)
        g = Geometry.from_arrays(P, [0, 1, 2, 3, 1, 4, 5, 2], [0, 4, 8])
        g.point["f"] = np.arange(6, dtype=np.float32)
        g.point["i"] = np.arange(6, dtype=np.int32)
        g.point["v"] = np.arange(18, dtype=np.float32).reshape(6, 3)
        g.point["uv2"] = np.arange(12, dtype=np.float32).reshape(6, 2)
        g.point["c"] = np.arange(24, dtype=np.float32).reshape(6, 4)
        g.point["b"] = np.array([1, 0, 1, 0, 1, 1], bool)
        g.vertex["cv"] = np.arange(24, dtype=np.float32).reshape(8, 3)
        g.prim["pf"] = np.array([5.0, 7.0], np.float32)
        return g.freeze()

    def test_full_write_topology_and_all_attribute_types(self):
        from minihou.link.diff import DiffState
        d = DiffState()
        g = self.make_geo()
        t = self.apply(g, d)
        self.assertTrue(t["rebuilt"])
        m = self.mesh
        self.assertEqual((len(m.vertices), len(m.loops), len(m.polygons), len(m.edges)), (6, 8, 2, 7))
        self.assertEqual([p.loop_total for p in m.polygons], [4, 4])
        lv = np.empty(8, np.int32); m.loops.foreach_get("vertex_index", lv)
        np.testing.assert_array_equal(lv, [0, 1, 2, 3, 1, 4, 5, 2])
        co = np.empty(18, np.float32); m.vertices.foreach_get("co", co)
        np.testing.assert_array_equal(co.reshape(-1, 3), g.P)

        def read(name, prop, n, dtype=np.float32):
            a = np.empty(n, dtype)
            m.attributes[name].data.foreach_get(prop, a)
            return a

        np.testing.assert_array_equal(read("f", "value", 6), np.arange(6))
        np.testing.assert_array_equal(read("i", "value", 6, np.int32), np.arange(6))
        np.testing.assert_array_equal(read("v", "vector", 18), np.arange(18))
        np.testing.assert_array_equal(read("uv2", "vector", 12), np.arange(12))
        np.testing.assert_array_equal(read("c", "color", 24), np.arange(24))
        np.testing.assert_array_equal(read("b", "value", 6, np.bool_), [1, 0, 1, 0, 1, 1])
        self.assertEqual(m.attributes["cv"].domain, "CORNER")
        self.assertEqual(m.attributes["pf"].domain, "FACE")
        np.testing.assert_array_equal(read("pf", "value", 2), [5, 7])
        self.assertEqual(t["warnings"], [])

    def test_positions_only_update_touches_only_positions(self):
        from minihou.link.diff import DiffState
        d = DiffState()
        g = self.make_geo()
        self.apply(g, d)
        m = self.mesh
        m.attributes["f"].data.foreach_set("value", np.full(6, 99, np.float32))     # sentinel: must survive
        g2 = g.copy()
        g2.point["P"] = g.P + 10
        t = self.apply(g2.freeze(), d)
        self.assertFalse(t["rebuilt"])
        co = np.empty(18, np.float32); m.vertices.foreach_get("co", co)
        np.testing.assert_array_equal(co.reshape(-1, 3), g.P + 10)
        f = np.empty(6, np.float32); m.attributes["f"].data.foreach_get("value", f)
        np.testing.assert_array_equal(f, 99)                                         # untouched

    def test_fast_write_paths_are_used_and_keep_blender_state_valid(self):
        """Positions/corners are written through attribute data (memcpy speed); make sure Blender's derived
        data (normals, edges) and validity checks still agree."""
        from minihou import Geometry
        from minihou.link.diff import DiffState
        d = DiffState()
        flat = np.array([[0, 0, 0], [1, 0, 0], [1, 1, 0], [0, 1, 0]], np.float32)
        g = Geometry.from_arrays(flat, [0, 1, 2, 3], [0, 4]).freeze()
        t = self.apply(g, d)
        self.assertEqual((t["position_path"], t["corner_path"]), ("attribute", "attribute"))
        self.assertFalse(self.mesh.validate(verbose=True))                       # False = nothing needed fixing
        self.assertEqual(len(self.mesh.edges), 4)
        np.testing.assert_allclose(self.mesh.polygons[0].normal, (0, 0, 1), atol=1e-6)
        wall = Geometry.from_arrays(flat[:, [0, 2, 1]].copy(), [0, 1, 2, 3], [0, 4])      # same quad, now in the XZ plane
        wall.point["P"] = flat[:, [0, 2, 1]].copy()
        t = self.apply(wall.freeze(), d)
        self.assertFalse(t["rebuilt"])
        n = np.array(self.mesh.polygons[0].normal)
        np.testing.assert_allclose(np.abs(n), (0, 1, 0), atol=1e-6)               # normal followed the update

    def test_removed_attribute_is_removed_and_count_change_rebuilds(self):
        from minihou import Geometry
        from minihou.link.diff import DiffState
        d = DiffState()
        g = self.make_geo()
        self.apply(g, d)
        g2 = g.copy()
        del g2.point["f"]
        self.apply(g2.freeze(), d)
        self.assertNotIn("f", self.mesh.attributes)
        self.assertIn("i", self.mesh.attributes)
        pts = Geometry.points_only(np.random.default_rng(0).random((10, 3), dtype=np.float32)).freeze()
        t = self.apply(pts, d)
        self.assertTrue(t["rebuilt"])
        self.assertEqual((len(self.mesh.vertices), len(self.mesh.polygons)), (10, 0))
        self.assertNotIn("i", self.mesh.attributes)

    def test_unsupported_attribute_is_skipped_with_a_warning(self):
        from minihou.link.diff import DiffState
        g = self.make_geo().copy()
        g.point["bad"] = np.zeros((6, 5), np.float32)
        t = self.apply(g.freeze(), DiffState())
        self.assertEqual(len(t["warnings"]), 1)
        self.assertNotIn("bad", self.mesh.attributes)


def differ_seq(d):
    d._seq = getattr(d, "_seq", 0) + 1
    return d._seq


if __name__ == "__main__":
    unittest.main()
