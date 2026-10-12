"""Actual addon preference registration and external-server operators in Blender."""
import os
from pathlib import Path
import shutil
import socket
import sys
import tempfile
import time
import unittest
from unittest.mock import patch

try:
    import bpy
except ImportError:
    bpy = None

ROOT = Path(__file__).resolve().parents[1]
sys.path[:0] = [str(ROOT), str(ROOT / "blender_addon")]


@unittest.skipUnless(bpy is not None, "requires real Blender")
class BlenderServerOperatorTests(unittest.TestCase):
    def setUp(self):
        executable = os.environ.get("MINIHOU_ENGINE_PYTHON", "")
        if not executable or not Path(executable).is_file():
            self.skipTest("Set MINIHOU_ENGINE_PYTHON to prepared external Python 3.11+")
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.assertEqual(bpy.ops.preferences.addon_enable(module="minihou_link"), {"FINISHED"})
        import minihou_link
        self.addon = minihou_link
        self.addCleanup(self.disable_addon)
        registered = bpy.context.preferences.addons.get("minihou_link")
        self.assertIsNotNone(registered)
        self.prefs = registered.preferences
        self.prefs.external_python = executable
        self.prefs.project_root = str(ROOT)
        self.graph = Path(self.temp.name) / "launcher-demo.json"
        shutil.copyfile(ROOT / "examples" / "graphs" / "demo.json", self.graph)
        bpy.context.scene.minihou_graph_path = str(self.graph)
        reserved = [socket.socket(), socket.socket()]
        try:
            for item in reserved:
                item.bind(("127.0.0.1", 0))
            self.port, self.http_port = [item.getsockname()[1] for item in reserved]
        finally:
            for item in reserved:
                item.close()
        bpy.context.window_manager.minihou_host = "127.0.0.1"
        bpy.context.window_manager.minihou_port = self.port
        bpy.context.scene.minihou_http_port = self.http_port

    def disable_addon(self):
        self.addon._server.stop(wait=True)
        self.assertEqual(bpy.ops.preferences.addon_disable(module="minihou_link"), {"FINISHED"})
        self.assertNotIn("minihou_link", bpy.context.preferences.addons)
        self.assertFalse(hasattr(bpy.types.Scene, "minihou_graph_path"))

    def wait_state(self, expected, timeout=40):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            state = self.addon._server.snapshot()
            if state["state"] == expected:
                return state
            self.assertNotEqual(state["state"], "error", state["detail"])
            time.sleep(0.05)
        self.fail(f"Timed out waiting for {expected}: {self.addon._server.snapshot()}")

    def test_preferences_launch_open_external_editor_and_owned_stop(self):
        from urllib.request import urlopen
        self.assertEqual(self.prefs.external_python, os.environ["MINIHOU_ENGINE_PYTHON"])
        self.assertEqual(Path(self.prefs.project_root), ROOT)
        self.assertEqual(bpy.ops.minihou.start_server(), {"FINISHED"})
        ready = self.wait_state("ready")
        self.assertTrue(ready["owned"])
        self.assertEqual(ready["editor_url"], f"http://127.0.0.1:{self.http_port}/")
        with urlopen(ready["editor_url"], timeout=5) as response:
            self.assertEqual(response.status, 200)
        process = self.addon._server._process
        self.assertIsNone(process.poll())
        with patch("minihou_link.server_control.webbrowser.open", return_value=True) as external_browser:
            self.assertEqual(bpy.ops.minihou.open_editor(), {"FINISHED"})
            external_browser.assert_called_once_with(ready["editor_url"])
        self.assertEqual(bpy.ops.minihou.stop_server(), {"FINISHED"})
        self.wait_state("stopped")
        self.assertIsNotNone(process.poll())
        self.assertFalse(self.addon._server.snapshot()["owned"])

    def test_edit_network_binds_actual_mesh_and_browser_edits_recook_modifier(self):
        import json
        import numpy as np
        from urllib.request import urlopen, Request
        from minihou_link import modifiers, network_ui, proxy
        from tests.test_blender_addon import evaluated_positions
        self.assertEqual(bpy.ops.minihou.start_server(), {"FINISHED"})
        ready = self.wait_state("ready")
        self.assertTrue(self.prefs.auto_connect)
        self.assertFalse(self.prefs.show_sidebar)
        self.assertFalse(self.addon.MINIHOU_PT_main.poll(bpy.context))
        self.prefs.auto_connect = False
        self.prefs.manual_port = 43210
        self.assertEqual(modifiers._settings(), ("127.0.0.1", 43210))
        self.prefs.auto_connect = True
        from minihou_link.discovery import publish
        stale = publish("127.0.0.1", 1, "http://127.0.0.1:1/")
        self.addCleanup(lambda: stale.unlink(missing_ok=True))
        # A deliberately wrong manual port and newer stale record must not prevent discovery.
        bpy.context.window_manager.minihou_port = 1
        self.assertEqual(modifiers._connection().port, self.port)
        bpy.ops.mesh.primitive_cube_add()
        obj = bpy.context.object
        # A fresh modifier needs neither a template file nor a legacy proxy connection.
        mod = modifiers.add_modifier(obj, "", port=self.port)
        other = modifiers.add_modifier(obj, "", port=self.port)
        modifiers.pump(port=self.port, force=True)
        initial = evaluated_positions(obj).copy()
        with patch("minihou_link.network_editor.webbrowser.open", return_value=True), \
             patch.object(self.addon._server, "snapshot", return_value={"state": "stopped"}):
            self.assertEqual(bpy.ops.minihou.edit_network(modifier_name=mod.name), {"FINISHED"})
            network_ui.router._thread.join(5)
        self.assertEqual(network_ui.router.snapshot()["state"], "ready", network_ui.router.snapshot())
        with urlopen(ready["editor_url"] + "api/graph", timeout=3) as response:
            state = json.load(response)
        self.addCleanup(lambda: Path(state["path"]).unlink(missing_ok=True))
        self.assertTrue(state["live_modifier"]["connected"])
        self.assertIn(obj.name, state["live_modifier"]["label"])
        self.assertEqual(Path(state["path"]), Path(modifiers.graph_path(mod)))
        request = Request(ready["editor_url"] + "api/edit", data=json.dumps({
            "document_id": state["document_id"],
            "ops": [{"op": "set", "id": "transform", "params": {"translate": [3, 0, 0]}}]
        }).encode(), headers={"Content-Type": "application/json"})
        with urlopen(request, timeout=3) as response:
            self.assertEqual(response.status, 200)
        modifiers.pump(port=self.port, force=True)
        self.assertEqual(modifiers.last_error(), "")
        np.testing.assert_allclose(evaluated_positions(obj), initial + [3, 0, 0], atol=1e-5)
        self.assertEqual(list(proxy.read_values(other, other.node_group, modifiers.parameters(other))["Translate"]), [0, 0, 0])
        # Blender controls update the same browser graph, with no manual Reload Graph.
        proxy.write_values(mod, mod.node_group, modifiers.parameters(mod), {"Translate": [6, 0, 0]})
        modifiers.pump(port=self.port, force=True)
        with urlopen(ready["editor_url"] + "api/graph", timeout=3) as response:
            updated = json.load(response)
        self.assertEqual(updated["graph"]["nodes"]["transform"]["params"]["translate"], [6, 0, 0])
        # Source mesh changes are visible to the browser as real modifier input.
        for vertex in obj.data.vertices:
            vertex.co.y += 2
        obj.data.update()
        modifiers.pump(port=self.port, force=True)
        request = Request(ready["editor_url"] + "api/inspect", data=json.dumps({"node": "input"}).encode(),
                          headers={"Content-Type": "application/json"})
        with urlopen(request, timeout=3) as response:
            inspection = json.load(response)
        self.assertTrue(inspection["ok"], inspection)
        self.assertEqual(inspection["points"], 8)
        np.testing.assert_allclose(evaluated_positions(obj), initial + [6, 2, 0], atol=1e-5)
        # Stored snapshots are updated for saving with the blend file.
        saved = json.loads(mod.node_group[modifiers.GRAPH])
        self.assertEqual(saved["nodes"]["transform"]["params"]["translate"], [6, 0, 0])
        obj.modifiers.remove(other)
        obj.modifiers.remove(mod)


if __name__ == "__main__":
    unittest.main(argv=[__file__])
