"""Rust TCP interoperability using the shipped bpy-free Blender client modules.

Run: python -m unittest discover -s tests -p test_rust_link.py -v
The debug-only `link_selftest` Cargo example never opens a window, starts Blender,
or launches minihou-app.exe. Each test owns an ephemeral listener and temp graph.
"""
from __future__ import annotations
import copy
import json
import os
from pathlib import Path
import queue
import shutil
import subprocess
import sys
import tempfile
import threading
import time
import types
import unittest

import numpy as np

ROOT = Path(__file__).resolve().parents[1]
# Load the real add-on files without executing its bpy-dependent __init__.py.
PKG = "_rust_link_addon"
package = types.ModuleType(PKG)
package.__path__ = [str(ROOT / "blender_addon/minihou_link")]
sys.modules[PKG] = package
from importlib import import_module
LinkClient = import_module(PKG + ".client").LinkClient
InstanceClient = import_module(PKG + ".instance_client").InstanceClient
protocol = import_module(PKG + ".protocol")
Inbox = import_module(PKG + ".state").Inbox
scene_plan = import_module(PKG + ".scene_plan")

PROXY = {"nodes": {"box": {"type": "box"}}, "display": "box", "unknown": {"keep": True}}
MODIFIER = {"nodes": {
    "input": {"type": "modifier_input"},
    "move": {"type": "transform", "inputs": {"geo": "input"}},
    "out": {"type": "output", "inputs": {"geo": "move"}}}, "display": "out",
    "exposed": [{"name": "Move", "node": "move", "param": "translate"},
                {"name": "Scale", "node": "move", "param": "uniform_scale"}]}


def mesh(x=1):
    return {"point/P": np.array([[x, 0, 0], [x, 1, 0], [x, 0, 1]], np.float32),
            "topo/vertex_point": np.array([0, 1, 2], np.int32),
            "topo/prim_offsets": np.array([0, 3], np.int32),
            "point/id": np.array([10, 11, 12], np.int32),
            "vertex/uv": np.array([[0, 0], [1, 0], [0, 1]], np.float32)}


class Client:
    def __init__(self, port):
        self.events = queue.Queue()
        self.inbox = Inbox()
        def receive(h, arrays):
            self.inbox.push(h, arrays)
            self.events.put((h, arrays))
        self.link = LinkClient(port=port, on_message=receive, reconnect=False)
        self.link.start()
        self.welcome = self.wait("welcome")[0]

    def wait(self, kind, predicate=lambda h: True, timeout=8):
        deadline = time.monotonic() + timeout
        seen = []
        while time.monotonic() < deadline:
            try:
                h, arrays = self.events.get(timeout=max(.01, deadline - time.monotonic()))
            except queue.Empty:
                break
            seen.append(h)
            if h.get("type") == kind and predicate(h):
                return h, arrays
        raise AssertionError(f"missing {kind}; seen {seen}; client error {self.link.last_error}")

    def geo(self, name=None, **kw):
        return self.wait("geo", lambda h: h.get("output") == name, **kw)

    def close(self):
        self.link.stop()


class Host:
    def __init__(self, graph):
        self.temp = tempfile.TemporaryDirectory(prefix="minihou-rust-link-")
        self.path = Path(self.temp.name)
        self.graph_path = self.path / "graph.json"
        self.graph_path.write_text(json.dumps(graph), encoding="utf-8")
        self.control = self.path / "control.json"
        self.command_id = 0
        self.clients = []
        cargo = shutil.which("cargo") or str(Path.home() / ".cargo/bin/cargo.exe")
        if not Path(cargo).is_file():
            raise unittest.SkipTest("Cargo is required for the Rust TCP integration host")
        env = dict(os.environ)
        env.setdefault("CARGO_TARGET_DIR", str(ROOT / "rust/target_inst"))
        self.log = (self.path / "cargo.log").open("w", encoding="utf-8")
        # cargo run only builds/runs the headless example; never the desktop binary.
        self.process = subprocess.Popen([cargo, "run", "--quiet", "-p", "minihou-app", "--example", "link_selftest", "--",
                                         str(self.graph_path), "127.0.0.1:0", "180", str(self.control)],
                                        cwd=ROOT / "rust", env=env, stdout=subprocess.PIPE, stderr=self.log,
                                        text=True, creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0))
        ready = queue.Queue()
        def read():
            for line in self.process.stdout:
                if line.startswith("LINK_READY "):
                    ready.put(int(line.strip().rsplit(":", 1)[1]))
            ready.put(None)
        self.reader = threading.Thread(target=read, daemon=True)
        self.reader.start()
        try:
            self.port = ready.get(timeout=180)
            if self.port is None:
                self.log.flush()
                raise AssertionError((self.path / "cargo.log").read_text(encoding="utf-8")[-12000:])
        except Exception:
            self.close()
            raise

    def command(self, op, **values):
        self.command_id += 1
        temporary = self.control.with_suffix(".tmp")
        temporary.write_text(json.dumps(dict(id=self.command_id, op=op, **values)), encoding="utf-8")
        temporary.replace(self.control)

    def client(self):
        client = Client(self.port)
        self.clients.append(client)
        return client

    def instances(self):
        client = InstanceClient(port=self.port, timeout=10)
        self.clients.append(client)
        return client

    def close(self):
        for client in self.clients:
            client.close()
        if hasattr(self, "process"):
            self.command("stop")
            try:
                self.process.wait(timeout=15)
            except subprocess.TimeoutExpired:
                # Only our cargo/example test process; never find or kill the desktop app.
                self.process.terminate()
                self.process.wait(timeout=5)
            self.process.stdout.close()
            if hasattr(self, "reader"):
                self.reader.join(timeout=1)
        self.log.close()
        self.temp.cleanup()


class RustLinkTests(unittest.TestCase):
    def host(self, graph=PROXY):
        host = Host(graph)
        self.addCleanup(host.close)
        return host

    def test_asset_chain_binds_target_mesh(self):
        graph = {"nodes": {"source": {"type": "blender_object", "params": {"object": "@target", "space": "object"}},
                           "out": {"type": "output", "inputs": {"geo": "source"}}}, "display": "out"}
        client = self.host().instances()
        client.set_sources("B", {"input:@picker:source": mesh(1)})
        header, arrays = client.cook("B", "", {}, mesh(8), graph=graph)
        self.assertEqual(header["type"], "instance_result")
        np.testing.assert_array_equal(arrays["point/P"], mesh(8)["point/P"])

    def test_literal_owner_migration_keeps_boolean_cutter(self):
        embed = import_module(PKG + ".embed")
        graph = {"nodes": {"source": {"type": "blender_object", "params": {"object": "Plane A"}},
                           "cutter": {"type": "blender_object", "params": {"object": "Cutter"}},
                           "boolean": {"type": "boolean", "inputs": {"geo": "source", "cutter": "cutter"}}}}
        class Group(dict):
            name = "Plane A asset"
        group = Group({embed.GRAPH: json.dumps(graph), embed.ASSET_OWNER: "Plane A"})
        fields = embed.repair_target(group)
        migrated = json.loads(group[embed.GRAPH])
        self.assertEqual(fields, ["nodes.source.params.object: 'Plane A' -> '@target'"])
        self.assertEqual(json.loads(group[embed.TARGET_CHANGES]), fields)
        self.assertEqual(migrated["nodes"]["source"]["params"]["object"], "@target")
        self.assertEqual(migrated["nodes"]["cutter"]["params"]["object"], "Cutter")
        self.assertEqual(graph["nodes"]["source"]["params"]["object"], "Plane A")

    def test_dynamic_modifier_inputs_roundtrip(self):
        definitions = [
            ("f", "Float", "float", 1.0), ("i", "Integer", "int", 2),
            ("b", "Enabled", "bool", False), ("v", "Offset", "vector", [0, 0, 0]),
            ("c", "Tint", "color", [1, 1, 1, 1]), ("s", "Label", "string", ""),
            ("o", "Asset", "object", ""),
        ]
        graph = {"nodes": {
            "input": {"type": "modifier_input", "modifier_inputs": [
                {"id": ident, "name": name, "kind": kind, "default": default}
                for ident, name, kind, default in definitions]},
            "asset": {"type": "object_input", "params": {"binding": "input:input:o"}},
            "move": {"type": "transform", "inputs": {"geo": "input"}, "params": {
                "translate": {"modifier_input": {"node": "input", "id": "v"}},
                "uniform_scale": {"modifier_input": {"node": "input", "id": "f"}}}},
            "merge": {"type": "merge", "inputs": {"geos": ["move", "asset"]}},
            "out": {"type": "output", "inputs": {"geo": "merge"}}}, "display": "out"}
        host = self.host()
        client = host.instances()
        client.set_sources("A", {"input:input:o": mesh(20)})
        values = {"Float": 2.0, "Integer": 7, "Enabled": True, "Offset": [3, 4, 5],
                  "Tint": [0.1, 0.2, 0.3, 0.4], "Label": "hello", "Asset": "Cube"}
        refs = {"input:input:o": {"kind": "object", "name": "Cube", "matrix":
                [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1]}}
        header, arrays = client.cook("A", "", values, mesh(), graph=graph, references=refs)
        self.assertEqual(header["type"], "instance_result")
        self.assertEqual(header["input_references"], refs)
        got = {p["name"]: p["value"] for p in header["interface"]}
        self.assertEqual(got, values)
        self.assertEqual(arrays["point/P"].shape[0], 6)
        self.assertAlmostEqual(arrays["point/P"][0, 0], 5)
        self.assertAlmostEqual(arrays["point/P"][3, 0], 20)
        client.set_sources("B", {"input:input:o": mesh(30)})
        other, other_arrays = client.cook("B", "", {"Float": 4, "Offset": [0, 0, 0], "Asset": "Sphere"}, mesh(), graph=graph)
        self.assertEqual(other["interface"][0]["value"], 4)
        self.assertEqual(other_arrays["point/P"][3, 0], 30)
        again, _ = client.cook("A", "", values, mesh(), graph=graph)
        self.assertEqual({p["name"]: p["value"] for p in again["interface"]}, values)

    def test_chained_placements_reuse_only_unchanged_cooks(self):
        host = self.host()
        client = host.instances()
        client.connect()
        source = mesh()

        def cook(a_move, b_scale):
            ah, a = client.cook("placement-A", "", {"Move": [a_move, 0, 0]}, source, graph=MODIFIER)
            bh, b = client.cook("placement-B", "", {"Scale": b_scale}, a, graph=MODIFIER)
            return ah, bh, a, b

        ah, bh, a, b = cook(2, 3)
        np.testing.assert_array_equal(a["point/P"][:, 0], [3, 3, 3])
        np.testing.assert_array_equal(b["point/P"][:, 0], [9, 9, 9])
        self.assertIn("move", ah["cook"]["cooked"])
        self.assertIn("move", bh["cook"]["cooked"])

        ah, bh, _, b = cook(2, 5)
        self.assertNotIn("move", ah["cook"]["cooked"])
        self.assertIn("move", bh["cook"]["cooked"])
        np.testing.assert_array_equal(b["point/P"][:, 0], [15, 15, 15])

        ah, bh, _, b = cook(4, 5)
        self.assertIn("move", ah["cook"]["cooked"])
        self.assertIn("move", bh["cook"]["cooked"])
        np.testing.assert_array_equal(b["point/P"][:, 0], [25, 25, 25])

    def test_exposure_interface_roundtrip_and_native_history(self):
        host = self.host()
        client = host.client()
        self.assertNotIn("shm_v1", client.welcome["capabilities"])
        self.assertIn("instances_v1", client.welcome["capabilities"])
        empty = client.wait("interface")[0]
        self.assertEqual(empty["params"], [])
        first, arrays = client.geo()
        self.assertTrue(first["full"])
        self.assertNotEqual(arrays["point/P"].size, 0)
        host.command("expose", node="box", param="size")
        exposed = client.wait("interface", lambda h: len(h["params"]) == 1)[0]
        p = exposed["params"][0]
        self.assertEqual((p["name"], p["node"], p["param"], p["kind"]), ("box_size", "box", "size", "vec3"))
        self.assertNotEqual(exposed["structure"], empty["structure"])
        client.geo()  # exposure cook, before the parameter cook
        self.assertTrue(client.link.set_param("box_size", [2, 3, 4]))
        updated = client.wait("interface", lambda h: h["params"][0]["value"] == [2, 3, 4])[0]
        self.assertEqual(updated["structure"], exposed["structure"])
        geo, arrays = client.geo()
        self.assertFalse(geo["topology_changed"])
        np.testing.assert_allclose(np.ptp(arrays["point/P"], axis=0), [2, 3, 4])
        self.assertGreaterEqual(geo["cook"]["ms"], 0)
        self.assertIsInstance(geo["cook"]["node_ms"], dict)
        host.command("undo")
        client.wait("interface", lambda h: h["params"][0]["value"] == p["value"])
        client.geo()
        host.command("redo")
        client.wait("interface", lambda h: h["params"][0]["value"] == [2, 3, 4])
        client.geo()
        host.command("expose", node="box", param="size")
        self.assertEqual(client.wait("interface", lambda h: not h["params"])[0]["structure"], empty["structure"])
        self.assertEqual(json.loads(host.graph_path.read_text()), PROXY)

    def test_per_subscriber_diffs_resync_and_inbox_conflation(self):
        host = self.host()
        a = host.client()
        a.wait("interface")
        first, arrays = a.geo()
        mirror = protocol.Mirror()
        mirror.apply(protocol.GeoUpdate.from_message(first, arrays))
        a.link.send({"type": "set_param", "node": "box", "param": "size", "value": [2, 2, 2]})
        h, arrays = a.geo()
        self.assertGreater(h["seq"], first["seq"])
        self.assertFalse(h["full"])
        self.assertFalse(h["topology_changed"])
        self.assertEqual(set(arrays), {"point/P"})
        mirror.apply(protocol.GeoUpdate.from_message(h, arrays))
        b = host.client()
        b.wait("interface")
        full, arrays = b.geo()
        self.assertTrue(full["full"])
        self.assertEqual(full["seq"], h["seq"])
        np.testing.assert_array_equal(arrays["point/P"], mirror.P)
        for n in range(3, 15):
            a.link.send({"type": "set_param", "node": "box", "param": "size", "value": [n, 2, 2]})
        a.wait("interface", lambda h: True)
        final = None
        while final is None:
            h, arrays = a.geo()
            mirror.apply(protocol.GeoUpdate.from_message(h, arrays))
            if np.ptp(mirror.P, axis=0)[0] == 14:
                final = h
        a.link.resync()
        h, arrays = a.geo()
        self.assertTrue(h["full"])
        self.assertGreaterEqual(h["seq"], final["seq"])
        self.assertIn("topo/vertex_point", arrays)
        _, merged = a.inbox.take()
        self.assertIsNotNone(merged)
        received = protocol.Mirror()
        received.apply(merged)
        np.testing.assert_array_equal(received.P, mirror.P)

    def test_instance_family_isolated_defaults_render_and_errors(self):
        host = self.host()
        c, other = host.instances(), host.instances()
        self.assertIn("live_modifier_v1", c.connect()["capabilities"])
        self.assertEqual(len(c.describe("", MODIFIER)["interface"]), 2)
        graph = copy.deepcopy(MODIFIER)
        graph["render"] = "input"
        h, a = c.cook("A", "", {"Move": [2, 0, 0]}, mesh(), graph=graph, materials=["Paint"])
        self.assertEqual(h["request_id"], h["seq"])
        self.assertEqual(h["detail"]["materials"], ["Paint"])
        np.testing.assert_array_equal(a["point/P"][:, 0], [3, 3, 3])
        np.testing.assert_array_equal(a["point/id"], mesh()["point/id"])
        _, rendered = c.cook("render", "", {"Move": [2, 0, 0]}, mesh(), graph=graph, render=True)
        np.testing.assert_array_equal(rendered["point/P"], mesh()["point/P"])
        _, b = c.cook("B", "", {"Scale": 3}, a, graph=graph)
        np.testing.assert_array_equal(b["point/P"][:, 0], [9, 9, 9])
        _, defaults = c.cook("B", "", {}, mesh(), graph=graph)
        np.testing.assert_array_equal(defaults["point/P"], mesh()["point/P"])
        _, isolated = other.cook("A", "", {}, mesh(7), graph=graph)
        self.assertEqual(isolated["point/P"][0, 0], 7)
        # Add-on 0.5.0: modifier graphs may hold blender_object nodes; they read only their own group
        # picker (input:@picker:<id>), never an object by name, so they are exposed instead of refused.
        picked = copy.deepcopy(graph)
        picked["nodes"]["source"] = {"type": "blender_object", "params": {"object": "Cube"}}
        self.assertIn("source", [p["name"] for p in c.describe("", picked)["interface"]])
        with self.assertRaisesRegex(RuntimeError, "exposed"):
            c.cook("bad", "", {"missing": 1}, mesh(), graph=graph)
        with self.assertRaisesRegex(RuntimeError, "fps"):
            c.cook("bad", "", {}, mesh(), graph=graph, fps=0)
        c.ack(1)
        c.remove("A")
        _, good = c.cook("A", "", {}, mesh(4), graph=graph)
        self.assertEqual(good["point/P"][0, 0], 4)
        proxy = host.client()
        proxy.wait("interface")
        self.assertEqual(proxy.geo()[0]["counts"]["point"], 8)

    def test_instance_sources_missing_replacement_and_scene_payload(self):
        host = self.host()
        c = host.instances()
        graph = copy.deepcopy(MODIFIER)
        graph["nodes"]["source"] = {"type": "object_input", "params": {"binding": "tree"}}
        graph["nodes"]["merge"] = {"type": "merge", "inputs": {"geos": ["move", "source"]}}
        graph["nodes"]["out"]["inputs"]["geo"] = "merge"
        graph["nodes"]["cloud"] = {"type": "scene_objects", "inputs": {"geo": "merge"}, "params": {"mode": "points"}}
        graph["nodes"]["bad_sink"] = {"type": "scene_objects", "inputs": {"geo": "input"}}  # no duplicate source
        c.set_sources("A", {"tree": mesh(8)})
        h, arrays = c.cook("A", "", {}, mesh(), graph=graph)
        self.assertEqual(len(arrays["point/P"]), 6)
        self.assertEqual(h["scene"][0]["mode"], "points")
        self.assertEqual(h["scene_errors"][0]["node"], "bad_sink")
        np.testing.assert_array_equal(arrays["scene:cloud:P"], arrays["point/P"])
        main, manifests = protocol.instance_payload(h, arrays)
        mirror = protocol.Mirror()
        mirror.apply(protocol.GeoUpdate.from_message(h, main))
        self.assertFalse(any(k.startswith("scene:") for k in mirror.arrays))
        np.testing.assert_array_equal(manifests[0]["_arrays"]["P"], mirror.P)
        c.set_sources("A", {})
        h, arrays = c.cook("A", "", {}, mesh(), graph=graph)       # nothing picked: waiting, not an error
        self.assertIn("Object input picker", h["cook"]["waiting"]["source"])
        c.set_sources("A", {"tree": mesh(9)})
        _, arrays = c.cook("A", "", {}, mesh(), graph=graph)
        self.assertEqual(arrays["point/P"][3, 0], 9)

    def test_live_modifier_native_edits_sync_switch_and_restore(self):
        host = self.host()
        proxy = host.client()
        proxy.wait("interface")
        proxy.geo()
        c = host.instances()
        h = c.edit_live("A", MODIFIER, {"Move": [2, 0, 0]}, mesh())
        self.assertTrue(h["live"])
        self.addCleanup(Path(h["path"]).unlink, missing_ok=True)
        self.assertTrue(Path(h["path"]).is_file())
        router = import_module(PKG + ".network_editor").NetworkEditor()
        router.start({"state": "ready", "native_live": h["editor"] == "native"}, h["path"], live=True)
        self.assertEqual(router.snapshot()["state"], "ready")
        self.assertNotEqual(Path(h["path"]), host.graph_path)
        host.command("param", name="Move", value=[5, 0, 0])
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline:
            h = c.sync_live("A", {"Move": [2, 0, 0]})
            if h["interface"][0]["value"] == [5, 0, 0]:
                break
            time.sleep(.02)
        self.assertEqual(h["interface"][0]["value"], [5, 0, 0])
        h = c.sync_live("A", {"Move": [7, 0, 0]})
        self.assertEqual(h["interface"][0]["value"], [7, 0, 0])
        host.command("param", name="Scale", value=2)
        c.set_sources("A", {})  # a feed update cannot overwrite an intervening native graph edit
        h, _ = c.cook("A", "", {"Move": [7, 0, 0]}, mesh(3), graph=h["graph"], frame=20)
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline:
            h = c.sync_live("A", {"Move": [7, 0, 0]})
            if h["interface"][1]["value"] == 2:
                break
            time.sleep(.02)
        self.assertEqual(h["interface"][1]["value"], 2)
        second = c.edit_live("B", MODIFIER, {}, mesh(3))
        self.addCleanup(Path(second["path"]).unlink, missing_ok=True)
        h = c.sync_live("A", {"Move": [7, 0, 0]})
        self.assertEqual(h["interface"][0]["value"], [7, 0, 0])
        h = c.edit_live("A", MODIFIER, {}, mesh())
        self.assertEqual(h["interface"][0]["value"], [7, 0, 0])
        proxy.link.set_param("Move", [99, 0, 0])
        proxy.wait("error", lambda h: h.get("where") == "set_param")
        c.remove("A")
        self.assertFalse(c.sync_live("A", {})["live"])
        interface = proxy.wait("interface")[0]
        self.assertEqual(interface["params"], [])
        self.assertEqual(proxy.geo()[0]["counts"]["point"], 8)
        self.assertEqual(json.loads(host.graph_path.read_text()), PROXY)

    def test_disconnect_restores_original_history_and_time(self):
        host = self.host()
        proxy = host.client()
        proxy.wait("interface")
        proxy.geo()
        host.command("expose", node="box", param="size")
        proxy.wait("interface", lambda h: len(h["params"]) == 1)
        proxy.geo()
        proxy.link.set_param("box_size", [3, 4, 5])
        proxy.wait("interface", lambda h: h["params"][0]["value"] == [3, 4, 5])
        proxy.geo()
        proxy.link.send({"type": "set_frame", "frame": 12, "fps": 30})
        proxy.wait("interface")
        proxy.geo()
        c = host.instances()
        h = c.edit_live("A", MODIFIER, {}, mesh(), frame=90, fps=60)
        self.addCleanup(Path(h["path"]).unlink, missing_ok=True)
        c.close()
        proxy.wait("interface", lambda h: len(h["params"]) == 1)
        restored, _ = proxy.geo()
        self.assertEqual((restored["cook"]["frame"], restored["cook"]["fps"]), (12, 30))
        host.command("undo")
        proxy.wait("interface", lambda h: h["params"][0]["value"] == [1, 1, 1])
        _, arrays = proxy.geo()
        np.testing.assert_allclose(np.ptp(arrays["point/P"], axis=0), [1, 1, 1])

    def test_input_recovery_curves_attribute_removal_and_last_good_resync(self):
        graph = {"nodes": {"source": {"type": "blender_object", "params": {"object": "Curve", "space": "object"}}}, "display": "source"}
        host = self.host(graph)
        c = host.client()
        c.wait("watch_objects")
        c.wait("interface")
        # Nothing received yet: the node is waiting (empty geometry), not a cook error (add-on 0.5.0).
        self.assertEqual(c.geo()[0]["counts"]["point"], 0)
        curve = mesh()
        curve["topo/prim_closed"] = np.array([0], np.uint8)
        c.link.send({"type": "object_geo", "object": "Curve", "materials": ["Paint"], "info": {"components": ["curves"]}}, curve)
        h, arrays = c.geo()
        mirror = protocol.Mirror()
        mirror.apply(protocol.GeoUpdate.from_message(h, arrays))
        self.assertTrue(h["topology_changed"])
        self.assertFalse(mirror.prim_closed[0])
        polygon = mesh()
        del polygon["point/id"]
        c.link.send({"type": "object_geo", "object": "Curve", "info": {"components": ["curves"]}}, polygon)
        h, arrays = c.geo()
        self.assertTrue(h["topology_changed"])
        self.assertEqual(arrays["topo/prim_closed"].size, 0)
        mirror.apply(protocol.GeoUpdate.from_message(h, arrays))
        self.assertIsNone(mirror.prim_closed)
        self.assertNotIn("point/id", mirror.arrays)
        good_seq = h["seq"]
        c.link.send({"type": "object_geo", "object": "Curve"}, {"point/P": np.zeros((1, 2), np.float32)})
        c.wait("error", lambda h: h.get("where") == "object_geo")
        error = c.wait("error", lambda h: h.get("where") == "cook")[0]
        self.assertGreater(error["seq"], good_seq)
        late = host.client()
        late.wait("interface")
        h, arrays = late.geo()
        self.assertEqual(h["seq"], good_seq)
        self.assertTrue(h["full"])
        np.testing.assert_array_equal(arrays["point/P"], mirror.P)
        late.wait("error", lambda h: h.get("seq") == error["seq"])
        c.link.resync()
        self.assertTrue(c.geo()[0]["full"])
        c.link.send({"type": "object_geo", "object": "Curve"}, mesh(8))
        h, arrays = c.geo()
        self.assertGreater(h["seq"], error["seq"])
        mirror.apply(protocol.GeoUpdate.from_message(h, arrays))
        self.assertEqual(mirror.P[0, 0], 8)

    def test_outputs_watch_objects_images_and_scene_modes(self):
        graph = copy.deepcopy(PROXY)
        graph["nodes"].update({
            "source": {"type": "blender_object", "params": {"object": "Cube", "space": "object"}},
            "extra": {"type": "output", "params": {"object": " Separate "}, "inputs": {"geo": "source"}},
            "points": {"type": "scene_objects", "params": {"mode": "points"}, "inputs": {"geo": "box"}},
            "curves": {"type": "scene_objects", "params": {"mode": "curves"}, "inputs": {"geo": "box"}},
            "pieces": {"type": "scene_objects", "params": {"mode": "pieces"}, "inputs": {"geo": "box"}},
            "duplicates": {"type": "scene_objects", "params": {"mode": "instances", "source": "Cube"}, "inputs": {"geo": "box"}},
            "pixels": {"type": "blender_image", "params": {"image": "Paint"}},
            "image": {"type": "image_out", "params": {"image": "Preview", "encoding": "linear"}, "inputs": {"geo": "pixels"}},
        })
        host = self.host(graph)
        c = host.client()
        watch = c.wait("watch_objects")[0]
        self.assertEqual(watch["objects"], ["Cube"])
        self.assertEqual(watch["images"], ["Paint"])
        c.wait("interface")
        c.geo()  # failed side outputs do not hide the valid main mesh
        c.link.send({"type": "object_geo", "object": "Cube", "materials": ["Paint"]}, mesh())
        c.wait("outputs", lambda h: h["names"] == ["Separate"])
        # Before the object arrived "Separate" cooked empty (waiting); take the geometry that carries it.
        h, arrays = c.wait("geo", lambda h: h.get("output") == "Separate" and h["counts"]["point"] > 0)
        self.assertEqual(h["detail"]["materials"], ["Paint"])
        np.testing.assert_array_equal(arrays["point/P"], mesh()["point/P"])
        pixels = np.array([[[.2, .4, .6, .8], [1, 0, 0, .5]], [[0, 1, 0, 1], [0, 0, 1, .25]]], np.float32)
        c.link.send({"type": "image_data", "image": "Paint", "width": 2, "height": 2, "is_float": True}, {"pixels": pixels})
        nodes = c.wait("scene_nodes", lambda h: "image" in h["nodes"])[0]["nodes"]
        self.assertEqual(set(nodes), {"points", "curves", "pieces", "duplicates", "image"})
        image, arrays = c.wait("scene_objects", lambda h: h["node"] == "image")
        self.assertEqual((image["mode"], image["image"], image["width"], image["height"]), ("image", "Preview", 2, 2))
        expected = pixels.copy()
        expected[..., 3] = 1  # image_out uses the chosen RGB source; matches Python pixels_from_lattice
        np.testing.assert_allclose(arrays["pixels"], expected)
        # Resync replays every manifest and each named output in full to this subscriber.
        c.link.resync()
        c.geo()
        h, arrays = c.geo("Separate")
        self.assertTrue(h["full"])
        c.wait("scene_nodes")
        manifests = {}
        for _ in range(5):
            h, arrays = c.wait("scene_objects")
            manifests[h["node"]] = (h, arrays)
        self.assertEqual(manifests["curves"][1]["cyclic"].dtype, np.uint8)
        self.assertIn("vp", manifests["pieces"][1])
        for name in ("points", "curves", "pieces", "duplicates"):
            self.assertTrue(scene_plan.items(manifests[name][0]))
        c.link.send({"type": "object_missing", "object": "Cube", "reason": "deleted"})
        c.wait("error", lambda h: "deleted" in h.get("message", ""))
        c.link.send({"type": "image_missing", "image": "Paint", "reason": "no image"})
        c.wait("error", lambda h: "no image" in h.get("message", ""))
        host.command("replace", graph=PROXY)
        c.wait("watch_objects", lambda h: not h["objects"] and not h["images"])
        c.wait("outputs", lambda h: not h["names"])
        c.wait("scene_nodes", lambda h: not h["nodes"])


if __name__ == "__main__":
    unittest.main()
