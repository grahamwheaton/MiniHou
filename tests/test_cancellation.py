import threading
import json
import tempfile
from urllib.request import Request, urlopen
import unittest
from unittest.mock import patch
from minihou import Graph
from minihou.link.session import Session
from minihou.nodes import fracture
from minihou.web import WebServer


class CancellationTests(unittest.TestCase):
    def test_cancelled_preview_returns_a_json_error_not_http_500(self):
        from minihou.cancellation import CookCancelled
        session = Session(Graph.from_dict({"nodes": {"box": {"type": "box"}}, "display": "box"}))
        self.addCleanup(session.stop)
        with tempfile.TemporaryDirectory() as root:
            server = WebServer(session, port=0, root=root)
            server.start()
            try:
                with patch.object(session.evaluator, "cook", side_effect=CookCancelled):
                    request = Request(server.url + "api/preview", b'{"node":"box"}', {"Content-Type": "application/json"})
                    with urlopen(request, timeout=3) as response:
                        result = json.load(response)
                        self.assertEqual(response.status, 200)
                self.assertFalse(result["ok"])
                self.assertEqual(result["node"], "box")
                self.assertEqual(result["rev"], session.snapshot()["rev"])
                self.assertIn("cancelled", result["error"]["message"])
            finally:
                server.stop()

    def test_cancel_endpoint_keeps_graph_edits(self):
        from pathlib import Path
        session = Session(Graph.from_dict({"nodes": {"box": {"type": "box"}}, "display": "box"}))
        before = session.snapshot()["graph"]
        with tempfile.TemporaryDirectory() as root:
            server = WebServer(session, port=0, root=Path(root))
            server.start()
            try:
                request = Request(server.url + "api/cancel", b"{}", {"Content-Type": "application/json"})
                with urlopen(request, timeout=3) as response:
                    self.assertTrue(json.load(response)["ok"])
                self.assertEqual(session.snapshot()["graph"], before)
                self.assertFalse(session.status()["busy"])
            finally:
                server.stop()

    def test_escape_interrupts_fracture_and_next_edit_cooks(self):
        graph = Graph.from_dict({"nodes": {
            "box": {"type": "box"},
            "points": {"type": "scatter", "inputs": {"geo": "box"}, "params": {"count": 100}},
            "fracture": {"type": "voronoi_fracture", "inputs": {"geo": "box", "points": "points"}},
        }, "display": "fracture"})
        session = Session(graph)
        entered = threading.Event()
        original = fracture.clip
        def clip(*args, **kwargs):
            entered.set()
            return original(*args, **kwargs)
        with patch.object(fracture, "clip", clip):
            worker = threading.Thread(target=session.cook_once, daemon=True)
            worker.start()
            self.assertTrue(entered.wait(5))
            session.cancel_cook()
            worker.join(3)
            self.assertFalse(worker.is_alive(), "fracture ignored cancellation")
        self.assertFalse(session.status()["busy"])
        self.assertIn("cancelled", session.status()["error"]["message"])
        graph.display = "box"
        session.replace_graph(graph)
        session.cook_once()
        self.assertTrue(session.status()["ok"])
        self.assertFalse(session.status()["busy"])


if __name__ == "__main__":
    unittest.main()
