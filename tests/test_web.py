"""The engine's HTTP API (minihou.web): endpoints, live events, and the localhost hardening."""
import http.client
import json
import os
import shutil
import socket
import tempfile
import threading
import time
import unittest

from minihou import Graph
from minihou.link.server import LinkServer
from minihou.link.session import Session
from minihou.web import WebServer

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


class WebTests(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.mkdtemp(prefix="minihou_web_")
        self.addCleanup(shutil.rmtree, self.dir, True)
        self.path = os.path.join(self.dir, "demo.json")
        shutil.copyfile(os.path.join(ROOT, "examples", "graphs", "demo.json"), self.path)
        os.makedirs(os.path.join(self.dir, "sub"))
        with open(os.path.join(self.dir, "sub", "not_a_graph.json"), "w", encoding="utf-8") as f:
            f.write('{"hello": 1}')
        self.s = Session(Graph.load(self.path, check=False))
        self.s.start()
        self.s.watch_file(self.path, interval=0.05)
        self.link = LinkServer(self.s, "127.0.0.1", 0)
        self.link.start()
        self.web = WebServer(self.s, "127.0.0.1", 0, link=self.link, root=self.dir)
        self.port = self.web.start()
        self.addCleanup(self.s.stop)
        self.addCleanup(self.link.stop)
        self.addCleanup(self.web.stop)

    def req(self, method, path, body=None, headers=None, raw=None):
        c = http.client.HTTPConnection("127.0.0.1", self.port, timeout=10)
        h = {"Content-Type": "application/json"} if body is not None else {}
        h.update(headers or {})
        c.request(method, path, body=raw if raw is not None else (json.dumps(body) if body is not None else None), headers=h)
        r = c.getresponse()
        data = r.read()
        c.close()
        try:
            return r.status, json.loads(data)
        except ValueError:
            return r.status, data

    def test_state_has_everything_an_editor_needs(self):
        st, d = self.req("GET", "/api/state")
        self.assertEqual(st, 200)
        self.assertIn("hills", d["graph"]["nodes"])
        self.assertIn("wrangle", [n["name"] for n in d["nodes"]])
        self.assertEqual(d["files"], ["demo.json"])                       # non-graph json filtered out
        self.assertIn("add", d["ops"])
        self.assertEqual(d["link"]["port"], self.link.port)

    def test_edit_waits_for_the_cook_and_reports(self):
        st, d = self.req("POST", "/api/edit", {"ops": [{"op": "set", "id": "ground", "params": {"columns": 10}}], "wait": True})
        self.assertEqual(st, 200)
        self.assertTrue(d["changed"])
        self.assertTrue(d["status"]["ok"])
        self.assertEqual(d["status"]["cooked_rev"], d["rev"])
        self.assertEqual(d["graph"]["rev"], d["rev"])
        st, d = self.req("POST", "/api/edit", {"ops": [{"op": "add", "type": "nope"}]})
        self.assertEqual(st, 400)
        self.assertEqual(d["index"], 0)
        self.assertIn("unknown node type", d["error"])

    def test_inspect_undo_and_status(self):
        st, d = self.req("POST", "/api/inspect", {"node": "ground", "detail": True})
        self.assertEqual((st, d["points"]), (200, 3600))
        self.req("POST", "/api/edit", {"ops": [{"op": "remove", "id": "spike"}]})
        st, d = self.req("POST", "/api/undo", {})
        self.assertTrue(d["ok"])
        self.assertIn("spike", d["graph"]["graph"]["nodes"])
        st, d = self.req("GET", "/api/status?wait=1")
        self.assertTrue(d["ok"])
        self.assertEqual(d["clients"], [])

    def test_open_and_create_inside_root_only(self):
        st, d = self.req("POST", "/api/open", {"path": "graphs/new.json", "create": True})
        self.assertEqual(st, 200)
        self.assertTrue(os.path.exists(os.path.join(self.dir, "graphs", "new.json")))
        self.assertIn("grid1", d["graph"]["nodes"])
        for bad, code in [("../escape.json", 403), ("/etc/passwd", 400), ("x.txt", 400), ("missing.json", 404)]:
            with self.subTest(path=bad):
                st, d = self.req("POST", "/api/open", {"path": bad, "create": bad != "missing.json"})
                self.assertEqual(st, code, d)
        self.assertFalse(os.path.exists(os.path.join(os.path.dirname(self.dir), "escape.json")))

    def test_static_editor_is_served(self):
        st, body = self.req("GET", "/")
        self.assertEqual(st, 200)
        self.assertIn(b"MiniHou", body)
        self.assertEqual(self.req("GET", "/static/editor.js")[0], 200)
        self.assertEqual(self.req("GET", "/static/../server.py")[0], 404)
        self.assertEqual(self.req("GET", "/static/%2e%2e%2fserver.py")[0], 404)

    def test_hardening_against_web_pages(self):
        evil = {"ops": [{"op": "add", "type": "box"}]}
        # DNS rebinding: a page on evil.com that resolves to 127.0.0.1
        self.assertEqual(self.req("GET", "/api/state", headers={"Host": "evil.com:%d" % self.port})[0], 403)
        # cross-site POST from another origin
        self.assertEqual(self.req("POST", "/api/edit", evil, headers={"Origin": "http://evil.com"})[0], 403)
        self.assertEqual(self.req("POST", "/api/edit", evil, headers={"Origin": "http://127.0.0.1:1"})[0], 403)
        # "simple" cross-site requests can't set application/json
        self.assertEqual(self.req("POST", "/api/edit", raw=json.dumps(evil), headers={"Content-Type": "text/plain"})[0], 415)
        self.assertNotIn("box1", self.s.graph.nodes)
        # the editor's own origin is fine
        ok = self.req("POST", "/api/edit", evil, headers={"Origin": f"http://127.0.0.1:{self.port}"})
        self.assertEqual(ok[0], 200)
        self.assertEqual(self.req("POST", "/api/edit", raw="{not json", headers={"Content-Type": "application/json"})[0], 400)

    def test_events_stream_graph_and_status(self):
        sock = socket.create_connection(("127.0.0.1", self.port), timeout=5)
        sock.sendall(f"GET /api/events HTTP/1.1\r\nHost: 127.0.0.1:{self.port}\r\n\r\n".encode())
        buf = b""
        events = []
        stop = threading.Event()

        def reader():
            nonlocal buf
            while not stop.is_set():
                try:
                    chunk = sock.recv(65536)
                except OSError:
                    return
                if not chunk:
                    return
                buf += chunk
                while b"\n\n" in buf:
                    block, buf = buf.split(b"\n\n", 1)
                    lines = dict(l.split(": ", 1) for l in block.decode().split("\n") if ": " in l and not l.startswith(":"))
                    if "event" in lines:
                        events.append((lines["event"], json.loads(lines["data"])))

        t = threading.Thread(target=reader, daemon=True)
        t.start()
        self.addCleanup(lambda: (stop.set(), sock.close()))

        def wait(pred, timeout=5):
            end = time.monotonic() + timeout
            while time.monotonic() < end:
                if any(pred(e) for e in events):
                    return True
                time.sleep(0.02)
            return False

        self.assertTrue(wait(lambda e: e[0] == "graph"))
        self.s.edit([{"op": "add", "type": "box", "id": "bx"}], origin="mcp")
        self.assertTrue(wait(lambda e: e[0] == "graph" and "bx" in e[1]["graph"]["nodes"]))
        self.s.set_param("Hill Frequency", 0.77)                          # from Blender
        self.assertTrue(wait(lambda e: e[0] == "graph" and e[1]["graph"]["nodes"]["hills"]["params"]["frequency"] == 0.77))
        self.assertTrue(wait(lambda e: e[0] == "status" and e[1].get("cooked_rev") == self.s.rev))


class ServeCliTests(unittest.TestCase):
    def test_busy_http_port_is_a_readable_error(self):
        s = Session(Graph.from_dict({"nodes": {"g": {"type": "grid"}}, "display": "g"}))
        a = WebServer(s, "127.0.0.1", 0, bind_retry_seconds=0)
        port = a.start()
        self.addCleanup(a.stop)
        b = WebServer(s, "127.0.0.1", port, bind_retry_seconds=0)
        with self.assertRaises(OSError) as cm:
            b.start()
        self.assertIn("already in use", str(cm.exception))


if __name__ == "__main__":
    unittest.main()
