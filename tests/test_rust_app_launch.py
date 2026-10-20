"""Mocked Rust app launcher tests; no Blender or desktop process required."""
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import Mock, patch

from tests.test_server_control import ServerController, finish, launcher


class RustAppLaunchTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.graph = Path(self.temp.name) / "graph.json"
        self.graph.write_text("{}", encoding="utf-8")
        self.app = Path(self.temp.name) / ("minihou-app.exe" if os.name == "nt" else "minihou-app")
        self.app.touch()
        self.controller = ServerController(startup_timeout=0.2)
        self.addCleanup(lambda: self.controller.stop(wait=True))
        self.header = {"type": "welcome", "server": "minihou", "protocol": launcher.PROTOCOL_VERSION,
                       "capabilities": ["instances_v1"], "graph_path": str(self.graph),
                       "editor": "native", "editor_url": None}

    def test_launches_rust_app_with_graph(self):
        process = Mock()
        process.poll.return_value = None
        with patch.object(self.controller, "_probe", side_effect=[None, self.header]), \
             patch.object(launcher.subprocess, "Popen", return_value=process) as popen, \
             patch.object(launcher.subprocess, "run") as python_check:
            self.controller.start(str(self.app), str(self.graph))
            self.assertEqual(finish(self.controller)["state"], "ready")
            self.assertEqual(popen.call_args.args[0], [str(self.app), str(self.graph)])
            python_check.assert_not_called()
            self.assertTrue(self.controller.snapshot()["owned"])

    def test_reuses_running_rust_app_even_without_executable(self):
        with patch.object(self.controller, "_probe", return_value=self.header), \
             patch.object(launcher.subprocess, "Popen") as popen:
            self.controller.start(str(self.graph.parent / "missing-app"), str(self.graph))
            state = finish(self.controller)
            self.assertEqual(state["state"], "ready")
            self.assertFalse(state["owned"])
            popen.assert_not_called()

    def test_missing_executable_reports_clear_error(self):
        with patch.object(self.controller, "_probe", return_value=None), \
             patch.object(launcher.subprocess, "Popen") as popen:
            self.controller.start(str(self.graph.parent / "missing-app"), str(self.graph))
            state = finish(self.controller)
            self.assertEqual(state["state"], "error")
            self.assertIn("minihou-app) not found", state["detail"])
            self.assertIn("add-on preferences", state["detail"])
            popen.assert_not_called()

    def test_finds_built_app_in_worktree(self):
        root = Path(self.temp.name)
        built = root / "rust" / "target" / "debug" / self.app.name
        built.parent.mkdir(parents=True)
        built.touch()
        source = root / "blender_addon" / "minihou_link" / "server_control.py"
        with patch.object(launcher, "__file__", str(source)), patch.dict(os.environ, {"CARGO_TARGET_DIR": ""}):
            self.assertEqual(self.controller._app_executable(""), built.resolve())


if __name__ == "__main__":
    unittest.main()
