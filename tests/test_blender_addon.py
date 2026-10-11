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
    def test_wrangle_attributes_reach_the_proxy_as_native_blender_data(self):
        """Engine wrangles -> link -> output mesh -> Geometry Nodes proxy: UV map, colors, materials, flat faces."""
        graph = {"version": 1, "nodes": {
            "g": {"type": "grid", "params": {"columns": 4, "rows": 4, "size_x": 2, "size_y": 2}},
            "uv": {"type": "wrangle", "inputs": {"geo": "g"}, "params": {
                "snippet": "u@uv = set(@P.x, @P.y) * 0.25", "run_over": "vertices"}},
            "faces": {"type": "wrangle", "inputs": {"geo": "uv"}, "params": {
                "snippet": "v@Cd = set(1, 0, 0) if @P.x > 0 else set(0, 0, 1); i@material_index = @P.x > 0; "
                           "i@sharp_face = 1", "run_over": "primitives"}},
            "o": {"type": "output", "inputs": {"geo": "faces"}}}, "display": "o"}
        self.graph_path.write_text(json.dumps(graph), encoding="utf-8")
        proxy_obj = lambda: bpy.data.objects.get(self.proxy.PROXY_OBJECT)  # noqa: E731

        def evaluated():
            return proxy_obj().evaluated_get(bpy.context.evaluated_depsgraph_get()).data

        self.pump(lambda: proxy_obj() is not None and "Color" in evaluated().color_attributes, timeout=60)
        me = evaluated()
        self.assertIn("UVMap", me.uv_layers)
        uv = np.empty(len(me.loops) * 2, np.float32); me.uv_layers["UVMap"].data.foreach_get("uv", uv)
        self.assertAlmostEqual(float(np.abs(uv).max()), 0.25, places=4)      # grid size 2 -> P in [-1, 1]
        mats = np.empty(len(me.polygons), np.int32); me.polygons.foreach_get("material_index", mats)
        self.assertEqual(sorted(set(mats.tolist())), [0, 1])
        self.assertFalse(any(p.use_smooth for p in me.polygons))
        col = np.empty(len(me.loops) * 4, np.float32); me.color_attributes["Color"].data.foreach_get("color", col)
        self.assertEqual(sorted({tuple(c) for c in col.reshape(-1, 4)[:, :3].round(3).tolist()}), [(0, 0, 1), (1, 0, 0)])

    def test_engine_curves_reach_the_proxy_as_edges(self):
        graph = {"version": 1, "nodes": {
            "h": {"type": "helix", "params": {"turns": 1, "points": 30}},
            "line": {"type": "line", "params": {"origin": [3, 0, 0], "points": 4}},
            "box": {"type": "box", "params": {"center": [-3, 0, 0]}},
            "m": {"type": "merge", "inputs": {"geos": ["h", "line", "box"]}},
            "o": {"type": "output", "inputs": {"geo": "m"}}}, "display": "o"}
        self.graph_path.write_text(json.dumps(graph), encoding="utf-8")
        proxy_obj = lambda: bpy.data.objects.get(self.proxy.PROXY_OBJECT)  # noqa: E731

        def evaluated():
            return proxy_obj().evaluated_get(bpy.context.evaluated_depsgraph_get()).data

        self.pump(lambda: proxy_obj() is not None and len(evaluated().vertices) == 30 + 4 + 8, timeout=60)
        me = evaluated()
        self.assertEqual(len(me.polygons), 6)
        self.assertEqual(sum(e.is_loose for e in me.edges), 29 + 3)

    def test_detail_attributes_from_the_engine_reach_the_output_object(self):
        graph = {"version": 1, "nodes": {
            "g": {"type": "grid", "params": {"columns": 4, "rows": 4}},
            "m": {"type": "measure", "inputs": {"geo": "g"}},
            "o": {"type": "output", "inputs": {"geo": "m"}}}, "display": "o"}
        from minihou import Graph, Evaluator
        detail = Evaluator().cook(Graph.from_dict(graph)).detail
        self.assertTrue(detail, "the measure node should write detail attributes")
        self.graph_path.write_text(json.dumps(graph), encoding="utf-8")
        self.pump(lambda: bpy.data.objects.get(self.proxy.OUTPUT_OBJECT) is not None
                  and bpy.data.objects[self.proxy.OUTPUT_OBJECT].get("mh_grid_columns") == 4, timeout=60)
        out = bpy.data.objects[self.proxy.OUTPUT_OBJECT]
        for name in detail:
            if isinstance(detail[name], (int, float)):
                self.assertAlmostEqual(float(out["mh_" + name]), float(detail[name]), places=4)

    def test_named_outputs_become_their_own_objects_and_clean_up(self):
        graph = {"version": 1, "nodes": {
            "g": {"type": "grid", "params": {"columns": 5, "rows": 5}},
            "b": {"type": "box"},
            "t": {"type": "tube"},
            "main": {"type": "output", "inputs": {"geo": "g"}},
            "trees": {"type": "output", "params": {"object": "Trees"}, "inputs": {"geo": "b"}},
            "rocks": {"type": "output", "params": {"object": "Rocks"}, "inputs": {"geo": "t"}}}, "display": "main"}
        user_mesh = bpy.data.meshes.new("Mine")
        user_obj = bpy.data.objects.new("Rocks", user_mesh)          # not ours: must never be taken over
        bpy.context.scene.collection.objects.link(user_obj)
        self.graph_path.write_text(json.dumps(graph), encoding="utf-8")
        self.pump(lambda: bpy.data.objects.get("Trees") is not None and len(bpy.data.objects["Trees"].data.vertices) == 8,
                  timeout=60)
        trees = bpy.data.objects["Trees"]
        self.assertTrue(trees.get("minihou_extra"))
        self.assertEqual(len(trees.data.polygons), 6)
        self.assertIn(trees.name, [o.name for o in bpy.context.scene.collection.objects])
        self.assertEqual(len(user_obj.data.vertices), 0)             # the user's own "Rocks" object stays untouched
        self.assertFalse(user_obj.get("minihou_extra"))
        # edits to the extra output's branch update its mesh in place
        graph["nodes"]["b"]["params"] = {"size": [4, 4, 4]}
        self.graph_path.write_text(json.dumps(graph), encoding="utf-8")
        self.pump(lambda: max(v.co.x for v in bpy.data.objects["Trees"].data.vertices) > 1.9, timeout=60)
        # dropping the object name removes the object the add-on made
        graph["nodes"]["trees"]["params"] = {}
        self.graph_path.write_text(json.dumps(graph), encoding="utf-8")
        self.pump(lambda: bpy.data.objects.get("Trees") is None, timeout=60)
        self.assertIsNotNone(bpy.data.objects.get("Rocks"))          # the user's object is never deleted

    def test_instance_points_place_blender_objects(self):
        for name, size in (("Tree", 0.2), ("Rock", 0.1)):
            me = bpy.data.meshes.new(name)
            me.from_pydata([(0, 0, 0), (size, 0, 0), (0, size, 0), (0, 0, size)], [], [(0, 1, 2), (0, 1, 3), (0, 2, 3), (1, 2, 3)])
            ob = bpy.data.objects.new(name, me)
            bpy.context.scene.collection.objects.link(ob)
            ob.location = (50, 50, 50)                      # far away: instances use the object's own geometry
        graph = {"version": 1, "nodes": {
            "g": {"type": "grid", "params": {"columns": 5, "rows": 5, "size_x": 4, "size_y": 4}},
            "pts": {"type": "scatter", "inputs": {"geo": "g"}, "params": {"count": 12, "seed": 1}},
            "inst": {"type": "instance", "inputs": {"points": "pts"},
                     "params": {"objects": "Tree Rock", "pick": "cycle", "scale_min": 2, "scale_max": 2}},
            "m": {"type": "merge", "inputs": {"geos": ["g", "inst"]}},
            "o": {"type": "output", "inputs": {"geo": "m"}}}, "display": "o"}
        self.graph_path.write_text(json.dumps(graph), encoding="utf-8")

        def instances():
            dg = bpy.context.evaluated_depsgraph_get()
            return [(i.object.original.name if i.object else None, i.matrix_world.copy()) for i in dg.object_instances
                    if i.is_instance and i.parent and i.parent.original.name == self.proxy.PROXY_OBJECT]

        self.pump(lambda: len(instances()) == 12, timeout=60)
        got = instances()
        self.assertEqual(sorted(n for n, _ in got), ["Rock"] * 6 + ["Tree"] * 6)
        for _, mw in got:
            self.assertAlmostEqual(mw.to_scale()[0], 2.0, places=4)
            self.assertLess(abs(mw.translation.z), 1e-4)                 # on the grid, not at the object's location
        me = bpy.data.objects[self.proxy.PROXY_OBJECT].evaluated_get(bpy.context.evaluated_depsgraph_get()).data
        self.assertEqual(len(me.vertices), 25)                           # instance points removed from the mesh
        # renaming the list rebuilds the branch; a missing object is reported, not fatal
        graph["nodes"]["inst"]["params"]["objects"] = "Rock Nope"
        self.graph_path.write_text(json.dumps(graph), encoding="utf-8")
        self.pump(lambda: any("Nope" in line for line in self.rt.log), timeout=60)
        self.pump(lambda: {n for n, _ in instances()} == {"Rock"}, timeout=30)
        # no instance points: the branch goes away
        graph["nodes"]["o"]["inputs"]["geo"] = "g"
        self.graph_path.write_text(json.dumps(graph), encoding="utf-8")
        self.pump(lambda: not instances(), timeout=30)
        self.assertFalse([n for n in bpy.data.node_groups[self.proxy.GROUP].nodes if n.name.startswith("MH_inst_")])

    def test_timeline_drives_frame_dependent_nodes(self):
        graph = {"version": 1, "nodes": {
            "g": {"type": "grid", "params": {"columns": 3, "rows": 3}},
            "w": {"type": "wrangle", "inputs": {"geo": "g"}, "params": {"snippet": "@P.z = @Frame * 0.5"}},
            "o": {"type": "output", "inputs": {"geo": "w"}}}, "display": "o"}
        self.graph_path.write_text(json.dumps(graph), encoding="utf-8")
        proxy_obj = lambda: bpy.data.objects.get(self.proxy.PROXY_OBJECT)  # noqa: E731

        def z():
            o = proxy_obj()
            return float(evaluated_positions(o)[:, 2].max()) if o is not None and len(o.evaluated_get(
                bpy.context.evaluated_depsgraph_get()).data.vertices) == 9 else None

        bpy.context.scene.frame_set(10)
        self.pump(lambda: z() is not None and abs(z() - 5.0) < 1e-5, timeout=60)
        bpy.context.scene.frame_set(24)
        self.pump(lambda: abs(z() - 12.0) < 1e-5, timeout=30)

    def make_source_object(self, name="Src"):
        """A 2x2-face mesh with native data: UV map, point color, material indices, a vertex group, a float attribute."""
        verts = [(x, y, 0.0) for y in range(3) for x in range(3)]
        faces = [(0, 1, 4, 3), (1, 2, 5, 4), (3, 4, 7, 6), (4, 5, 8, 7)]
        me = bpy.data.meshes.new(name)
        me.from_pydata(verts, [], faces)
        uv = me.uv_layers.new(name="MyUVs")
        uv.data.foreach_set("uv", np.tile(np.array([0.5, 0.25], np.float32), len(me.loops)))
        col = me.color_attributes.new("Paint", "FLOAT_COLOR", "POINT")
        col.data.foreach_set("color", np.tile(np.array([0.5, 0, 0, 1], np.float32), len(me.vertices)))
        me.color_attributes.render_color_index = me.color_attributes.find("Paint")
        me.attributes.new("material_index", "INT", "FACE").data.foreach_set("value", np.array([0, 1, 2, 3], np.int32))
        me.attributes.new("heat", "FLOAT", "POINT").data.foreach_set("value", np.arange(9, dtype=np.float32))
        obj = bpy.data.objects.new(name, me)
        bpy.context.scene.collection.objects.link(obj)
        obj.location = (100.0, 0.0, 0.0)
        g = obj.vertex_groups.new(name="lift")
        g.add([4], 1.0, "REPLACE")                       # the centre point
        bpy.context.view_layer.update()
        return obj

    def test_blender_objects_feed_graphs_with_their_native_attributes(self):
        """Blender mesh -> blender_object node -> wrangles reading native data -> proxy; edits in Blender recook."""
        src = self.make_source_object()
        graph = {"version": 1, "nodes": {
            "src": {"type": "blender_object", "params": {"object": "Src", "space": "world"}},
            "pts": {"type": "wrangle", "inputs": {"geo": "src"}, "params": {
                "snippet": "@P.z = @lift * 2 + @Cd.x + @heat * 0.0", "run_over": "points"}},
            "prims": {"type": "wrangle", "inputs": {"geo": "pts"}, "params": {
                "snippet": "i@material_index = @material_index + 10", "run_over": "primitives"}},
            "o": {"type": "output", "inputs": {"geo": "prims"}}}, "display": "o"}
        self.graph_path.write_text(json.dumps(graph), encoding="utf-8")
        proxy_obj = lambda: bpy.data.objects.get(self.proxy.PROXY_OBJECT)  # noqa: E731

        def proxy_P():
            o = proxy_obj()
            return evaluated_positions(o) if o is not None else np.zeros((0, 3))

        def settled(pred, timeout=60):
            end = time.monotonic() + timeout
            while time.monotonic() < end:
                bpy.context.view_layer.update()
                self.rt.tick()
                if pred():
                    return
                time.sleep(0.01)
            self.fail(f"timed out; last_error={self.rt.last_error!r} info={self.rt.object_info}")

        settled(lambda: len(proxy_P()) == 9 and np.isclose(proxy_P()[:, 2].max(), 2.5, atol=1e-5))
        self.assertEqual(self.rt.watched, ["Src"])
        P = proxy_P()
        self.assertAlmostEqual(float(P[:, 0].min()), 100.0, places=4)          # world space: the object's location
        np.testing.assert_allclose(sorted(P[:, 2]), [0.5] * 8 + [2.5], atol=1e-5)  # vertex group + render color
        me = proxy_obj().evaluated_get(bpy.context.evaluated_depsgraph_get()).data
        mats = np.empty(len(me.polygons), np.int32); me.polygons.foreach_get("material_index", mats)
        self.assertEqual(sorted(mats.tolist()), [10, 11, 12, 13])
        self.assertIn("UVMap", me.uv_layers)                                     # render UV map -> uv -> UV map
        uv = np.empty(len(me.loops) * 2, np.float32); me.uv_layers["UVMap"].data.foreach_get("uv", uv)
        np.testing.assert_allclose(uv.reshape(-1, 2)[0], [0.5, 0.25], atol=1e-6)

        sent = self.rt.objects_sent
        src.select_set(True)                                                     # no data change: nothing is resent
        self.settle(20)
        self.assertEqual(self.rt.objects_sent, sent)
        src.data.vertices[0].co.x = -5.0                                         # an edit in Blender recooks
        src.data.update()
        settled(lambda: np.isclose(proxy_P()[:, 0].min(), 95.0, atol=1e-4))
        self.assertGreater(self.rt.objects_sent, sent)
        src.location.x = 200.0                                                   # so does moving the object
        settled(lambda: np.isclose(proxy_P()[:, 0].min(), 195.0, atol=1e-4))

        bpy.data.objects.remove(src)                                             # deleting it is reported
        settled(lambda: "Src" in self.rt.object_info and "no object" in self.rt.object_info["Src"])

    def test_connect_creates_proxy_and_mesh_matches_engine(self):
        self.pump(lambda: self.rt.applied_count >= 1)
        out = bpy.data.objects[self.proxy.OUTPUT_OBJECT]
        proxy_obj = bpy.data.objects[self.proxy.PROXY_OBJECT]
        self.assertTrue(out.hide_get())
        self.assertEqual(len(out.data.vertices), 900)
        self.assertEqual(len(out.data.polygons), 29 * 29)
        np.testing.assert_allclose(evaluated_positions(proxy_obj), self.expected_P(), atol=self.TOL)
        self.assertEqual(len(proxy_obj.evaluated_get(bpy.context.evaluated_depsgraph_get()).data.polygons), 29 * 29)
        self.assertIn("UVMap", out.data.uv_layers)              # the grid's uv arrives as a real UV map

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

    def native_geo(self, cd_domain="point", uv_domain="point", with_n=True):
        from minihou import Geometry
        P = np.array([[0, 0, 0], [1, 0, 0], [1, 1, 0], [0, 1, 0], [2, 0, 0], [2, 1, 0]], np.float32)
        g = Geometry.from_arrays(P, [0, 1, 2, 3, 1, 4, 5, 2], [0, 4, 8])
        uv = P[:, :2] * 0.5
        if uv_domain == "point":
            g.point["uv"] = uv
        else:
            g.vertex["uv"] = uv[g.topology.vertex_point]
        if cd_domain == "point":
            g.point["Cd"] = np.tile(np.array([1, 0.5, 0.25], np.float32), (6, 1))
        else:
            g.prim["Cd"] = np.array([[1, 0, 0], [0, 1, 0]], np.float32)
        if with_n:
            n = np.tile(np.array([0.6, 0, 0.8], np.float32), (6, 1))
            g.point["N"] = n
        g.prim["material_index"] = np.array([0, 2], np.int32)
        g.prim["sharp_face"] = np.array([1, 0], np.int32)
        return g.freeze()

    def test_native_attributes_uv_color_normals_materials_shading(self):
        from minihou.link.diff import DiffState
        d = DiffState()
        g = self.native_geo()
        t = self.apply(g, d)
        m = self.mesh
        self.assertEqual(t["warnings"], [])
        # uv -> UV map on corners, active for render
        self.assertEqual(list(m.uv_layers.keys()), ["UVMap"])
        self.assertTrue(m.uv_layers.active.name == "UVMap" and m.uv_layers["UVMap"].active_render)
        uv = np.empty(16, np.float32); m.uv_layers["UVMap"].data.foreach_get("uv", uv)
        np.testing.assert_allclose(uv.reshape(-1, 2), (g.P[:, :2] * 0.5)[g.topology.vertex_point])
        # Cd -> color attribute 'Color', alpha 1, active
        self.assertEqual(m.color_attributes.active_color.name, "Color")
        c = np.empty(24, np.float32); m.color_attributes["Color"].data.foreach_get("color", c)
        np.testing.assert_allclose(c.reshape(-1, 4)[0], [1, 0.5, 0.25, 1])
        # N -> custom normals
        self.assertTrue(m.has_custom_normals)
        cn = np.empty(24, np.float32); m.corner_normals.foreach_get("vector", cn)
        np.testing.assert_allclose(cn.reshape(-1, 3), np.tile([0.6, 0, 0.8], (8, 1)), atol=1e-4)
        # material_index and sharp_face are Blender's own
        self.assertEqual([p.material_index for p in m.polygons], [0, 2])
        self.assertEqual([p.use_smooth for p in m.polygons], [False, True])
        self.assertNotIn("uv", m.attributes)
        self.assertNotIn("Cd", m.attributes)
        self.assertNotIn("N", m.attributes)

    def test_detail_attributes_become_custom_properties_on_the_output_object(self):
        obj = bpy.data.objects.new("out", self.mesh)
        detail = {"iteration": 3, "ratio": 0.25, "label": "wall", "flag": True, "size": [1.5, 2.5, 3.5],
                  "ids": [1, 2, 3], "instance_objects": ["Cube"], "weird": {"a": 1}, "nothing": None, "mixed": [1, "a"]}
        warnings = self.w.write_detail(obj, detail)
        self.assertEqual(obj["mh_iteration"], 3)
        self.assertEqual(obj["mh_ratio"], 0.25)
        self.assertEqual(obj["mh_label"], "wall")
        self.assertEqual(obj["mh_flag"], 1)
        self.assertEqual(obj["mh_size"].to_list(), [1.5, 2.5, 3.5])
        self.assertEqual(obj["mh_ids"].to_list(), [1, 2, 3])
        for name in ("instance_objects", "weird", "nothing", "mixed"):
            self.assertNotIn("mh_" + name, obj.keys())
        self.assertEqual(sorted(w.split("'")[1] for w in warnings), ["mixed", "nothing", "weird"])
        # unchanged values are not rewritten; changed ones are; attributes the engine stopped producing go away
        before = obj.get("mh_ratio")
        self.w.write_detail(obj, {"iteration": 4, "ratio": 0.25})
        self.assertEqual((obj["mh_iteration"], obj["mh_ratio"], before), (4, 0.25, 0.25))
        for gone in ("mh_label", "mh_flag", "mh_size", "mh_ids"):
            self.assertNotIn(gone, obj.keys())
        self.w.write_detail(obj, {})
        self.assertEqual([k for k in obj.keys() if k.startswith("mh_")], [])
        obj["mine"] = 1                                            # properties we did not write stay untouched
        self.w.write_detail(obj, {"x": 1})
        self.assertEqual(obj["mine"], 1)
        self.assertEqual(list(obj[self.w.DETAIL_KEYS]), ["mh_x"])
        self.w.write_detail(obj, {"a" * 70: 1})                      # custom property names are limited to 63 bytes
        self.assertNotIn("mh_" + "a" * 70, obj.keys())

    def test_extra_output_objects_are_created_marked_and_removed(self):
        a = self.w.ensure_extra_object("Extra")
        self.assertTrue(a.get(self.w.EXTRA_MARK))
        self.assertIs(self.w.ensure_extra_object("Extra"), a)           # idempotent
        mine = bpy.data.objects.new("Mine", bpy.data.meshes.new("MineMesh"))
        bpy.context.scene.collection.objects.link(mine)
        self.assertIsNone(self.w.ensure_extra_object("Mine"))            # someone else's object: not ours
        self.w.ensure_extra_object("Other")
        self.assertEqual(self.w.remove_extra_objects(["Other"]), ["Extra"])
        self.assertIsNone(bpy.data.objects.get("Extra"))
        self.assertIsNotNone(bpy.data.objects.get("Other"))
        self.assertIsNotNone(bpy.data.objects.get("Mine"))
        self.assertEqual(self.w.remove_extra_objects([]), ["Other"])
        self.assertEqual([m.name for m in bpy.data.meshes if m.name in ("Extra", "Other")], [])    # no orphan meshes

    def test_curves_become_loose_edges(self):
        from minihou import Geometry
        from minihou.link.diff import DiffState
        P = np.array([[0, 0, 0], [1, 0, 0], [1, 1, 0], [0, 1, 0], [3, 0, 0], [4, 0, 0], [5, 1, 0]], np.float32)
        g = Geometry.from_arrays(P, [0, 1, 2, 3, 4, 5, 6], [0, 4, 7], closed=[True, False])
        g.prim["id"] = np.array([10, 20], np.int32)
        g.vertex["k"] = np.arange(7, dtype=np.float32)
        g.point["w"] = np.arange(7, dtype=np.float32)
        d = DiffState()
        t = self.apply(g.freeze(), d)
        m = self.mesh
        self.assertEqual((len(m.vertices), len(m.polygons), len(m.loops), t["loose_edges"]), (7, 1, 4, 2))
        loose = sorted(tuple(sorted(e.vertices)) for e in m.edges if e.is_loose)
        self.assertEqual(loose, [(4, 5), (5, 6)])
        self.assertEqual([d_.value for d_ in m.attributes["id"].data], [10])          # polygon's attribute only
        self.assertEqual([d_.value for d_ in m.attributes["k"].data], [0, 1, 2, 3])
        self.assertEqual(len(m.attributes["w"].data), 7)
        # the curve turns into a polygon: loose edges go away
        g2 = Geometry.from_arrays(P[:4], [0, 1, 2, 3], [0, 4])
        self.apply(g2.freeze(), d)
        self.assertFalse(any(e.is_loose for e in m.edges))
        self.assertEqual(len(m.edges), 4)

    def test_reader_turns_loose_edges_into_curves(self):
        import bpy as _bpy
        from minihou_link import mesh_reader
        from minihou.link.server import _decode_object
        me = _bpy.data.meshes.new("wire")
        me.from_pydata([(0, 0, 0), (1, 0, 0), (1, 1, 0), (0, 1, 0), (3, 0, 0), (4, 0, 0)], [(4, 5)], [(0, 1, 2, 3)])
        me.attributes.new("h", "FLOAT", "FACE").data.foreach_set("value", np.array([7.0], np.float32))
        obj = _bpy.data.objects.new("wire", me)
        _bpy.context.scene.collection.objects.link(obj)
        hdr, arrays = mesh_reader.read_object(obj, _bpy.context.evaluated_depsgraph_get())
        self.assertEqual(hdr["info"]["loose_edges"], 1)
        geo = _decode_object(hdr, arrays)
        self.assertEqual(geo.topology.closed_mask().tolist(), [True, False])
        self.assertEqual(geo.prim["h"].tolist(), [7.0, 0.0])
        self.assertEqual(geo.topology.polys()[1], [4, 5])

    def test_groups_become_boolean_attributes(self):
        from minihou.link.diff import DiffState
        g = self.native_geo().copy()
        g.point["group_top"] = np.array([0, 0, 1, 1, 0, 1], np.int32)
        g.prim["group_left"] = np.array([1, 0], np.int32)
        self.apply(g.freeze(), DiffState())
        a = self.mesh.attributes["group_top"]
        self.assertEqual((a.domain, a.data_type), ("POINT", "BOOLEAN"))
        self.assertEqual([d.value for d in a.data], [False, False, True, True, False, True])
        f = self.mesh.attributes["group_left"]
        self.assertEqual((f.domain, f.data_type, [d.value for d in f.data]), ("FACE", "BOOLEAN", [True, False]))

    def test_native_attributes_from_other_domains_and_removal(self):
        from minihou.link.diff import DiffState
        d = DiffState()
        self.apply(self.native_geo(cd_domain="prim", uv_domain="vertex"), d)
        m = self.mesh
        ca = m.color_attributes["Color"]
        self.assertEqual(ca.domain, "CORNER")                 # face colours expand to corners
        c = np.empty(32, np.float32); ca.data.foreach_get("color", c)
        np.testing.assert_allclose(c.reshape(-1, 4)[:, :3], [[1, 0, 0]] * 4 + [[0, 1, 0]] * 4)
        g2 = self.native_geo(with_n=False).copy()
        del g2.point["Cd"]
        self.apply(g2.freeze(), d)
        self.assertNotIn("Color", m.attributes)               # engine stopped producing Cd / N: gone in Blender
        self.assertFalse(m.has_custom_normals)
        self.assertIn("UVMap", m.uv_layers)

    def test_reserved_blender_names_warn(self):
        from minihou.link.diff import DiffState
        g = self.native_geo().copy()
        g.point["Color"] = np.zeros((6, 3), np.float32)
        t = self.apply(g.freeze(), DiffState())
        self.assertTrue(any("'Color'" in w for w in t["warnings"]), t["warnings"])

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
