"""Exercise runtime lifecycle methods without importing bpy or touching a Blender scene."""
import ast
from pathlib import Path
from types import SimpleNamespace
import unittest
from unittest.mock import Mock


class AutoConnectTests(unittest.TestCase):
    def setUp(self):
        path = Path(__file__).resolve().parents[1] / "blender_addon/minihou_link/runtime.py"
        tree = ast.parse(path.read_text(encoding="utf-8"))
        runtime = next(n for n in tree.body if isinstance(n, ast.ClassDef) and n.name == "Runtime")
        runtime.body = [n for n in runtime.body if isinstance(n, ast.FunctionDef)
                        and n.name in {"connect", "disconnect", "poll_auto_connect", "_on_state"}]
        self.client = Mock()
        self.handlers = SimpleNamespace(depsgraph_update_post=[], frame_change_post=[])
        namespace = dict(mesh_writer=SimpleNamespace(assert_main_thread=Mock()), LinkClient=Mock(return_value=self.client),
                         bpy=SimpleNamespace(app=SimpleNamespace(version_string="test", timers=Mock(),
                                                                  handlers=self.handlers)),
                         timer_entry=Mock(), _depsgraph_post=Mock(), _frame_change=Mock())
        exec(compile(ast.Module(body=[runtime], type_ignores=[]), str(path), "exec"), namespace)
        self.rt = namespace["Runtime"]()
        self.rt.client = None
        self.rt.state = "disconnected"
        self.rt.auto_connect_paused = False
        self.rt._auto_client = False
        self.rt._timer_on = False
        self.rt.inbox = Mock()
        self.rt._log = Mock()
        self.factory = namespace["LinkClient"]

    def test_enabled_poll_starts_one_client_on_configured_local_port(self):
        self.rt.poll_auto_connect(True, 9876)
        self.factory.assert_called_once()
        self.assertEqual(self.factory.call_args.args, ("127.0.0.1", 9876))
        self.assertEqual(self.factory.call_args.kwargs["retry_seconds"], 2.0)
        self.rt.poll_auto_connect(True, 9876)
        self.factory.assert_called_once()

    def test_manual_disconnect_stays_paused_until_explicit_connect(self):
        self.rt.poll_auto_connect(True, 8765)
        self.rt.disconnect()
        self.client.stop.assert_called_once()
        for enabled in [True, False, True]:
            self.rt.poll_auto_connect(enabled, 8765)
        self.factory.assert_called_once()
        self.assertIsNone(self.rt.client)
        self.rt.connect()
        self.assertFalse(self.rt.auto_connect_paused)
        self.assertEqual(self.factory.call_count, 2)

    def test_disabled_preference_stops_only_automatic_client(self):
        self.rt.poll_auto_connect(False, 8765)
        self.factory.assert_not_called()
        self.rt.poll_auto_connect(True, 8765)
        self.rt.poll_auto_connect(False, 8765)
        self.assertIsNone(self.rt.client)
        self.assertFalse(self.rt.auto_connect_paused)
        self.rt.connect()
        self.rt.poll_auto_connect(False, 8765)
        self.assertIs(self.rt.client, self.client)

    def test_network_disconnect_keeps_existing_retry_worker(self):
        self.rt.poll_auto_connect(True, 8765)
        self.rt.state = "disconnected"
        self.rt.poll_auto_connect(True, 8765)
        self.factory.assert_called_once()
        self.client.stop.assert_not_called()


if __name__ == "__main__":
    unittest.main()
