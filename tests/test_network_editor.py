"""Real HTTP routing and failure-safe switching, without bpy or a real browser launch."""
import json
from pathlib import Path
import tempfile
import threading
import unittest
from unittest.mock import patch

from minihou import Graph, GraphError
from minihou.link.session import Session
from minihou.web import WebServer
from tests.addon_loader import pure_module

network = pure_module("network_editor")


class NetworkRoutingTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.a, self.b = self.root / "A graph.json", self.root / "B graph.json"
        for path, rows in ((self.a, 2), (self.b, 3)):
            Graph.from_dict({"nodes": {"g": {"type": "grid", "params": {"rows": rows}}}, "display": "g"}).save(path)
        self.s = Session(Graph.load(self.a))
        self.s.watch_file(self.a, interval=60)
        self.addCleanup(self.s.stop)
        self.web = WebServer(self.s, "127.0.0.1", 0, root=self.root)
        self.web.start()
        self.addCleanup(self.web.stop)
        self.router = network.NetworkEditor(timeout=1)
        self.server = {"state": "ready", "editor_url": self.web.url}

    def route(self, path, browser=True):
        with patch.object(network.webbrowser, "open", return_value=browser) as opened:
            self.router.start(self.server, str(path))
            self.router._thread.join(3)
            self.assertFalse(self.router._thread.is_alive())
            return self.router.snapshot(), opened.call_count

    def test_two_templates_route_and_flush_current_edits(self):
        self.s.edit([{"op": "set", "id": "g", "params": {"rows": 7}}])
        state, calls = self.route(self.b)
        self.assertEqual((state["state"], calls), ("ready", 1))
        self.assertEqual(self.s.path, self.b)
        self.assertEqual(Graph.load(self.a).get_param("g", "rows"), 7)
        self.s.edit([{"op": "set", "id": "g", "params": {"rows": 9}}])
        state, calls = self.route(self.a)
        self.assertEqual((state["state"], calls), ("ready", 1))
        self.assertEqual(self.s.path, self.a)
        self.assertEqual(Graph.load(self.b).get_param("g", "rows"), 9)

    def test_same_template_preserves_dirty_graph_and_undo(self):
        self.s.edit([{"op": "set", "id": "g", "params": {"rows": 11}}])
        context = self.s.snapshot()["document_id"]
        state, _ = self.route(self.a)
        self.assertEqual(state["state"], "ready")
        self.assertEqual(self.s.graph.get_param("g", "rows"), 11)
        self.assertEqual(Graph.load(self.a).get_param("g", "rows"), 11)
        self.assertEqual(self.s.snapshot()["document_id"], context)
        self.assertTrue(self.s.undo())

    def test_save_failure_keeps_old_graph_path_edits_and_history(self):
        self.s.edit([{"op": "set", "id": "g", "params": {"rows": 13}}])
        before = self.s.snapshot()
        with patch("minihou.link.session._atomic_write", side_effect=PermissionError("test save denied")):
            state, calls = self.route(self.b)
        self.assertEqual((state["state"], calls), ("error", 0))
        self.assertIn("Cannot save current graph", state["detail"])
        self.assertEqual(self.s.snapshot(), before)
        self.assertTrue(self.s._dirty)
        self.assertEqual(self.route(self.b)[0]["state"], "ready")

    def test_missing_outside_root_bad_json_and_browser_failure(self):
        bad = self.root / "invalid.json"
        bad.write_text("{", encoding="utf-8")
        with tempfile.TemporaryDirectory() as outside:
            escaped = Path(outside) / "outside.json"
            escaped.write_text(self.a.read_text(encoding="utf-8"), encoding="utf-8")
            for path in (self.root / "missing.json", escaped, bad):
                with self.subTest(path=path):
                    state, calls = self.route(path)
                    self.assertEqual((state["state"], calls), ("error", 0))
                    self.assertEqual(self.s.path, self.a)
        state, calls = self.route(self.b, browser=False)
        self.assertEqual(state["state"], "error")
        self.assertIn("Graph template selected", state["detail"])
        self.assertEqual(self.s.path, self.b)

    def test_delayed_edit_and_undo_cannot_change_new_document(self):
        old = self.s.snapshot()["document_id"]
        self.s.open_file(self.b)
        before = self.s.snapshot()
        with self.assertRaisesRegex(GraphError, "previously open"):
            self.s.edit([{"op": "set", "id": "g", "params": {"rows": 99}}], document_id=old)
        with self.assertRaisesRegex(GraphError, "previously open"):
            self.s.undo(document_id=old)
        self.assertEqual(self.s.snapshot(), before)
        self.s.open_file(self.a)
        with self.assertRaisesRegex(GraphError, "previously open"):
            self.s.edit([], document_id=old)  # A -> B -> A is still a different document

    def test_request_is_background_and_overlapping_requests_are_rejected(self):
        entered, release = threading.Event(), threading.Event()
        def blocked(*args):
            entered.set()
            release.wait(2)
        with patch.object(self.router, "_open", side_effect=blocked):
            self.router.start(self.server, str(self.a))
            self.assertTrue(entered.wait(1))
            with self.assertRaisesRegex(ValueError, "in progress"):
                self.router.start(self.server, str(self.b))
            release.set()
            self.router._thread.join(2)

    def test_invalid_endpoint_snapshot_only_and_old_engine_are_rejected(self):
        for server, path in (({"state": "stopped"}, self.a),
                             ({"state": "ready", "editor_url": "http://example.com/"}, self.a),
                             ({"state": "ready", "editor_url": "http://127.0.0.1:bad/"}, self.a),
                             (self.server, "")):
            with self.assertRaises(ValueError):
                self.router.start(server, str(path))
        with patch.object(self.s, "snapshot", return_value={"path": str(self.a)}):
            state, calls = self.route(self.b)
        self.assertEqual((state["state"], calls), ("error", 0))
        self.assertIn("Update/restart", state["detail"])
        self.assertEqual(self.s.path, self.a)


if __name__ == "__main__":
    unittest.main()
