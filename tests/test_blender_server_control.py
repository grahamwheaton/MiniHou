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

    def test_edit_network_routes_each_modifier_template_and_reports_missing_file(self):
        import json
        from urllib.request import urlopen
        from minihou_link import modifiers, network_ui
        (ROOT / "build").mkdir(exist_ok=True)
        with tempfile.TemporaryDirectory(dir=ROOT / "build", prefix="edit_network_") as folder:
            a, b = Path(folder) / "a.json", Path(folder) / "b.json"
            shutil.copyfile(ROOT / "examples/modifiers/transform.json", a)
            shutil.copyfile(ROOT / "examples/modifiers/repeat_transform.json", b)
            self.assertEqual(bpy.ops.minihou.start_server(), {"FINISHED"})
            ready = self.wait_state("ready")
            bpy.ops.mesh.primitive_cube_add()
            obj = bpy.context.object
            first = modifiers.add_modifier(obj, str(a), port=self.port)
            second = modifiers.add_modifier(obj, str(b), port=self.port)
            snapshots = [mod.node_group[modifiers.GRAPH] for mod in (first, second)]
            with patch("minihou_link.network_editor.webbrowser.open", return_value=True) as browser:
                for mod, path in ((first, a), (second, b), (first, a)):
                    self.assertEqual(bpy.ops.minihou.edit_network(modifier_name=mod.name), {"FINISHED"})
                    network_ui.router._thread.join(5)
                    self.assertEqual(network_ui.router.snapshot()["state"], "ready", network_ui.router.snapshot())
                    with urlopen(ready["editor_url"] + "api/graph", timeout=3) as response:
                        self.assertEqual(Path(json.load(response)["path"]), path)
                self.assertEqual(browser.call_count, 3)
                b.unlink()
                self.assertEqual(bpy.ops.minihou.edit_network(modifier_name=second.name), {"FINISHED"})
                network_ui.router._thread.join(5)
                self.assertEqual(network_ui.router.snapshot()["state"], "error")
                self.assertIn("no such", network_ui.router.snapshot()["detail"])
                self.assertEqual(browser.call_count, 3)
            self.assertEqual([mod.node_group[modifiers.GRAPH] for mod in (first, second)], snapshots)
            for mod in (first, second):
                obj.modifiers.remove(mod)


if __name__ == "__main__":
    unittest.main(argv=[__file__])
