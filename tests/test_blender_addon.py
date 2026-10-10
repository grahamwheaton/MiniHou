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
        self.proc = subprocess.Popen([os.environ.get("MINIHOU_ENGINE_PYTHON", sys.executable), "-u", "-m", "minihou", "serve", str(graph_path), "--port", str(port)],
                                     cwd=str(ROOT), stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
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
        self.graph_path.write_text(json.dumps(GRAPH))
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

    def expected_P(self):
        from minihou import Evaluator, Graph
        g = Graph.from_dict(GRAPH)
        return Evaluator().cook(g).P

    def modifier(self):
        p = bpy.data.objects[self.proxy.PROXY_OBJECT]
        return p.modifiers[self.proxy.MODIFIER]

    def set_modifier_value(self, name, value):
        mod = self.modifier()
        mod[self.proxy.mapping(mod.node_group)[name]] = value
        bpy.context.view_layer.update()              # fires depsgraph_update_post, like a UI edit

    # -- tests -------------------------------------------------------------
    def test_connect_creates_proxy_and_mesh_matches_engine(self):
        self.pump(lambda: self.rt.applied_count >= 1)
        out = bpy.data.objects[self.proxy.OUTPUT_OBJECT]
        proxy_obj = bpy.data.objects[self.proxy.PROXY_OBJECT]
        self.assertTrue(out.hide_get())
        self.assertEqual(len(out.data.vertices), 900)
        self.assertEqual(len(out.data.polygons), 29 * 29)
        np.testing.assert_allclose(evaluated_positions(proxy_obj), self.expected_P(), atol=1e-6)
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
        self.assertEqual(tuple(mod[m["Height"]]), (0.0, 0.0, 1.0))
        self.assertEqual(mod[m["Columns"]], 30)

    def test_editing_a_modifier_input_updates_the_engine_and_the_mesh(self):
        self.pump(lambda: self.rt.applied_count >= 1)
        before = self.rt.applied_count
        self.set_modifier_value("Height", (0.0, 0.0, 2.5))
        self.pump(lambda: self.rt.applied_count > before)
        GRAPH2 = json.loads(json.dumps(GRAPH))
        GRAPH2["nodes"]["n"]["params"]["amplitude"] = [0, 0, 2.5]
        from minihou import Evaluator, Graph
        want = Evaluator().cook(Graph.from_dict(GRAPH2)).P
        np.testing.assert_allclose(evaluated_positions(bpy.data.objects[self.proxy.PROXY_OBJECT]), want, atol=1e-6)
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
        self.settle(60)
        want = 0.1 + 39 * 0.02
        from minihou import Evaluator, Graph
        G = json.loads(json.dumps(GRAPH))
        G["nodes"]["n"]["params"]["frequency"] = want
        np.testing.assert_allclose(evaluated_positions(bpy.data.objects[self.proxy.PROXY_OBJECT]),
                                   Evaluator().cook(Graph.from_dict(G)).P, atol=1e-5)
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
        self.graph_path.write_text(json.dumps(G))
        self.pump(lambda: "Size" in self.proxy.mapping(self.modifier().node_group))
        mod = self.modifier()
        m = self.proxy.mapping(mod.node_group)
        self.assertAlmostEqual(mod[m["Frequency"]], 0.8, places=6)       # engine value pushed into the modifier
        self.assertEqual(mod[m["Size"]], 6.0)
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
