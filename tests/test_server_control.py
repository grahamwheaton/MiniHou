"""Launcher tests run without Blender and never start or stop an unrelated process."""
import importlib.util
from pathlib import Path
import socket
import sys
import tempfile
import threading
import time
import types
import unittest
from unittest.mock import Mock, patch

ROOT = Path(__file__).resolve().parents[1]
PACKAGE = "_launcher_test_addon"
package = types.ModuleType(PACKAGE)
package.__path__ = [str(ROOT / "blender_addon" / "minihou_link")]
sys.modules.setdefault(PACKAGE, package)
spec = importlib.util.spec_from_file_location(PACKAGE + ".server_control", package.__path__[0] + "/server_control.py")
launcher = importlib.util.module_from_spec(spec)
spec.loader.exec_module(launcher)
ServerController = launcher.ServerController


def finish(controller):
    controller._thread.join(5)
    assert not controller._thread.is_alive(), "launcher did not finish"
    return controller.snapshot()


class ServerControllerTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        self.graph = self.root / "graph.json"
        self.graph.write_text("{}", encoding="utf-8")
        self.controller = ServerController(startup_timeout=0.1)
        self.header = {"type": "welcome", "server": "minihou", "protocol": launcher.PROTOCOL_VERSION,
                       "capabilities": ["instances_v1"], "graph_path": str(self.graph),
                       "editor_url": "http://127.0.0.1:8766/"}

    def tearDown(self):
        self.controller.stop(wait=True)
        self.temp.cleanup()

    def test_reuse_needs_no_runtime_and_stop_never_terminates_external_server(self):
        with patch.object(self.controller, "_probe", return_value=self.header), patch.object(launcher.subprocess, "Popen") as popen:
            self.controller.start("", "", str(self.graph))
            state = finish(self.controller)
            self.assertEqual(state["state"], "ready")
            self.assertFalse(state["owned"])
            self.controller.stop(wait=True)
            popen.assert_not_called()

    def test_incompatible_graph_or_editor_never_launches_or_stops(self):
        for field, value in [("graph_path", str(self.root / "other.json")), ("editor_url", "http://127.0.0.1:9999/")]:
            with self.subTest(field=field), patch.object(self.controller, "_probe", return_value={**self.header, field: value}), patch.object(launcher.subprocess, "Popen") as popen:
                self.controller.start("", "", str(self.graph))
                self.assertEqual(finish(self.controller)["state"], "error")
                popen.assert_not_called()

    def test_explicit_runtime_required_and_blender_executable_rejected(self):
        blender = self.root / "blender.exe"
        blender.touch()
        for executable in ["", "python", str(blender)]:
            with self.subTest(executable=executable), patch.object(self.controller, "_probe", return_value=None), patch.object(launcher.subprocess, "Popen") as popen:
                self.controller.start(executable, str(ROOT), str(self.graph))
                self.assertEqual(finish(self.controller)["state"], "error")
                popen.assert_not_called()

    def test_launch_readiness_and_owned_stop(self):
        process = Mock()
        process.poll.return_value = None
        check = types.SimpleNamespace(returncode=0, stdout="MINIHOU_PYTHON_READY", stderr="")
        with patch.object(self.controller, "_probe", side_effect=[None, self.header]), patch.object(launcher.subprocess, "run", return_value=check), patch.object(launcher.subprocess, "Popen", return_value=process) as popen:
            self.controller.start(sys.executable, str(ROOT), str(self.graph))
            state = finish(self.controller)
            self.assertEqual(state["state"], "ready")
            self.assertTrue(state["owned"])
            command = popen.call_args.args[0]
            self.assertEqual(command[:4], [sys.executable, "-m", "minihou", "serve"])
            self.assertNotIn("--no-save", command)
            self.assertNotIn("PYTHONHOME", popen.call_args.kwargs["env"])
            with patch.object(launcher.webbrowser, "open", return_value=True) as browser:
                self.assertTrue(self.controller.open_editor())
                browser.assert_called_once_with(self.header["editor_url"])
            self.controller.stop(wait=True)
            process.terminate.assert_called_once()

    def test_import_failure_reports_error_without_launch(self):
        check = types.SimpleNamespace(returncode=1, stdout="", stderr="No module named numpy")
        with patch.object(self.controller, "_probe", return_value=None), patch.object(launcher.subprocess, "run", return_value=check), patch.object(launcher.subprocess, "Popen") as popen:
            self.controller.start(sys.executable, str(ROOT), str(self.graph))
            state = finish(self.controller)
            self.assertIn("No module named numpy", state["detail"])
            popen.assert_not_called()

    def test_startup_timeout_cleans_up_owned_process(self):
        process = Mock()
        process.poll.return_value = None
        check = types.SimpleNamespace(returncode=0, stdout="MINIHOU_PYTHON_READY", stderr="")
        with patch.object(self.controller, "_probe", return_value=None), patch.object(launcher.subprocess, "run", return_value=check), patch.object(launcher.subprocess, "Popen", return_value=process):
            self.controller.start(sys.executable, str(ROOT), str(self.graph))
            self.assertEqual(finish(self.controller)["state"], "error")
            process.terminate.assert_called_once()
            self.assertFalse(self.controller.snapshot()["owned"])

    def test_probe_checks_wire_handshake(self):
        listener = socket.socket()
        listener.bind(("127.0.0.1", 0))
        listener.listen()
        received = []
        def serve():
            with listener.accept()[0] as connection:
                received.append(launcher.recv_message(connection)[0])
                launcher.send_message(connection, self.header)
        thread = threading.Thread(target=serve)
        thread.start()
        try:
            result = self.controller._probe("127.0.0.1", listener.getsockname()[1])
            self.assertEqual(result["graph_path"], str(self.graph))
            self.assertEqual(received[0]["mode"], "instances")
        finally:
            thread.join(2)
            listener.close()

    @unittest.skipIf(sys.version_info < (3, 11), "External engine requires Python 3.11+")
    def test_real_external_engine_launch_reuse_and_owned_cleanup(self):
        import shutil
        from urllib.request import urlopen
        shutil.copyfile(ROOT / "examples" / "graphs" / "demo.json", self.graph)
        sockets = [socket.socket(), socket.socket()]
        for item in sockets:
            item.bind(("127.0.0.1", 0))
        port, http_port = [item.getsockname()[1] for item in sockets]
        for item in sockets:
            item.close()
        self.controller.startup_timeout = 30
        other = ServerController()
        try:
            self.controller.start(sys.executable, str(ROOT), str(self.graph), host="localhost", port=port, http_port=http_port)
            self.controller._thread.join(40)
            state = self.controller.snapshot()
            self.assertEqual(state["state"], "ready", state["detail"])
            with urlopen(state["editor_url"], timeout=3) as response:
                self.assertEqual(response.status, 200)
            other.start("", "", str(self.graph), host="localhost", port=port, http_port=http_port)
            self.assertEqual(finish(other)["state"], "ready")
            self.assertFalse(other.snapshot()["owned"])
            other.stop(wait=True)
            process = self.controller._process
            self.assertIsNone(process.poll())
            self.controller.stop(wait=True)
            self.assertIsNotNone(process.poll())
        finally:
            other.stop(wait=True)

    def test_reused_server_loss_updates_status(self):
        with patch.object(self.controller, "_probe", side_effect=[self.header, None]):
            self.controller.start("", "", str(self.graph))
            self.assertEqual(finish(self.controller)["state"], "ready")
            self.controller._monitor.join(3)
            state = self.controller.snapshot()
            self.assertEqual(state["state"], "error")
            self.assertIn("no longer reachable", state["detail"])

    def test_shutdown_without_new_threads_terminates_only_owned_process(self):
        process = Mock()
        process.poll.return_value = None
        self.controller._process = process
        self.controller._shutdown()
        process.terminate.assert_called_once()
        self.assertIsNone(self.controller._process)

    def test_editor_endpoint_accepts_loopback_aliases_only(self):
        for host in ("localhost", "127.0.0.1", "::1"):
            for endpoint in ("localhost", "127.0.0.1", "[::1]"):
                header = {**self.header, "editor_url": f"http://{endpoint}:8766/"}
                self.assertTrue(self.controller._compatible(header, self.graph, host, 8766))
        for url in ("http://example.com:8766/", "http://localhost:9999/", "https://localhost:8766/",
                    "http://localhost:8766/other", "http://localhost:8766/?x=1",
                    "http://user@localhost:8766/", "http://localhost:8766/#fragment"):
            with self.subTest(url=url), self.assertRaises(ValueError):
                self.controller._compatible({**self.header, "editor_url": url}, self.graph, "localhost", 8766)

    def test_open_editor_requires_ready(self):
        with self.assertRaisesRegex(ValueError, "ready"):
            self.controller.open_editor()


if __name__ == "__main__":
    unittest.main()
