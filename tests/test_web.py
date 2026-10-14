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

import numpy as np

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

    def test_python_preference_is_explicit_persistent_and_origin_checked(self):
        from unittest.mock import patch
        from minihou.nodes import python_node
        from minihou import user_preferences
        previous=python_node._ENABLED
        self.addCleanup(python_node.set_allowed,previous)
        with patch.dict(os.environ,{"MINIHOU_PREFERENCES_FILE":os.path.join(self.dir,"user-prefs.json")}):
            self.assertEqual(self.req("POST","/api/preferences",{"python_nodes":"yes"})[0],400)
            self.assertEqual(self.req("POST","/api/preferences",{"python_nodes":True},headers={"Origin":"https://example.com"})[0],403)
            for enabled in (True,False):
                status,data=self.req("POST","/api/preferences",{"python_nodes":enabled})
                self.assertEqual(status,200)
                self.assertEqual(data["python_nodes"],enabled)
                self.assertEqual(user_preferences.read()["python_nodes"],enabled)
                self.assertEqual(self.req("GET","/api/state")[1]["engine"]["python_nodes"],enabled)

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

    def test_preview_leaves_out_triangles_the_viewer_already_has(self):
        from minihou import preview

        def get(body):
            c = http.client.HTTPConnection("127.0.0.1", self.port, timeout=10)
            c.request("POST", "/api/preview", json.dumps(body), {"Content-Type": "application/json"})
            r = c.getresponse()
            data = r.read()
            c.close()
            return preview.unpack(data)
        head, arrays = get({"node": "ground"})
        token = head["topology"]
        self.assertTrue(token)
        self.assertIn("tri", arrays)
        head2, arrays2 = get({"node": "hills", "have": token})                # noise moves the points of the same grid
        self.assertTrue(head2.get("same_topology"))
        self.assertNotIn("tri", arrays2)
        self.assertNotIn("edge", arrays2)
        self.assertEqual(len(arrays2["P"]), len(arrays["P"]))
        self.assertFalse(np.array_equal(arrays2["P"], arrays["P"]))
        head3, arrays3 = get({"node": "hills", "have": "something else"})      # not what the viewer holds: everything
        self.assertNotIn("same_topology", head3)
        self.assertIn("tri", arrays3)

    def test_preview_returns_binary_geometry_or_a_json_error(self):
        from minihou import preview
        c = http.client.HTTPConnection("127.0.0.1", self.port, timeout=10)
        c.request("POST", "/api/preview", json.dumps({"node": "ground"}), {"Content-Type": "application/json"})
        r = c.getresponse()
        data = r.read()
        c.close()
        self.assertEqual((r.status, r.getheader("Content-Type")), (200, "application/octet-stream"))
        head, arrays = preview.unpack(data)
        self.assertEqual((head["node"], head["prims"]), ("ground", 3481))
        self.assertEqual(head["tris"], 2 * 3481)
        self.assertEqual(len(arrays["P"]), head["points"] * 3)
        st, d = self.req("POST", "/api/preview", {"node": "nope"})
        self.assertEqual((st, d["ok"]), (200, False))
        self.assertIn("no such node", d["error"]["message"])
        self.assertEqual(self.req("POST", "/api/preview", {"node": 5})[0], 400)
        self.assertEqual(self.req("POST", "/api/preview", {"node": "ground"}, headers={"Origin": "http://evil.com"})[0], 403)

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

    def test_frame_endpoint(self):
        st, d = self.req("POST", "/api/frame", {"frame": 12, "fps": 30})
        self.assertEqual((st, d["frame"], d["fps"]), (200, 12.0, 30.0))
        st, d = self.req("GET", "/api/status?wait=1")
        self.assertEqual(d["frame"], 12.0)
        st, _ = self.req("POST", "/api/frame", {"frame": "soon"})
        self.assertEqual(st, 400)

    def test_open_and_create_inside_root_only(self):
        st, d = self.req("POST", "/api/open", {"path": "graphs/new.json", "create": True})
        self.assertEqual(st, 200)
        self.assertTrue(os.path.exists(os.path.join(self.dir, "graphs", "new.json")))
        self.assertIn("box1", d["graph"]["nodes"])
        for bad, code in [("../escape.json", 403), ("/etc/passwd", 400), ("x.txt", 400), ("missing.json", 404)]:
            with self.subTest(path=bad):
                st, d = self.req("POST", "/api/open", {"path": bad, "create": bad != "missing.json"})
                self.assertEqual(st, code, d)
        self.assertFalse(os.path.exists(os.path.join(os.path.dirname(self.dir), "escape.json")))

    def test_blender_panel_lists_installs_serves_the_zip_and_only_runs_what_was_found(self):
        import io
        import zipfile
        from minihou import blender_setup
        st, d = self.req("GET", "/api/blender")
        self.assertEqual(st, 200)
        self.assertTrue(d["addon"]["available"])
        self.assertEqual(d["link_port"], self.link.port)
        st, zdata = self.req("GET", "/addon/minihou_link.zip")
        self.assertEqual(st, 200)
        self.assertIn("minihou_link/__init__.py", zipfile.ZipFile(io.BytesIO(zdata)).namelist())
        calls = []
        orig = (blender_setup.list_blenders, blender_setup.install_addon, blender_setup.launch_blender)
        blender_setup.list_blenders = lambda *a, **k: [{"path": "/opt/blender-5.1/blender", "version": "5.1", "supported": True}]
        blender_setup.install_addon = lambda exe, **k: (calls.append(("install", exe)), {"ok": True, "message": "done"})[1]
        blender_setup.launch_blender = lambda exe, port, host="127.0.0.1": calls.append(("launch", exe, port))
        self.addCleanup(lambda: (setattr(blender_setup, "list_blenders", orig[0]), setattr(blender_setup, "install_addon", orig[1]),
                                 setattr(blender_setup, "launch_blender", orig[2])))
        st, d = self.req("POST", "/api/blender/install", {"index": 0})
        self.assertEqual((st, d["ok"]), (200, True))
        st, d = self.req("POST", "/api/blender/launch", {"index": 0})
        self.assertEqual((st, d["ok"]), (200, True))
        self.assertEqual(calls, [("install", "/opt/blender-5.1/blender"), ("launch", "/opt/blender-5.1/blender", self.link.port)])
        for bad in ({"index": 5}, {"index": -1}, {"index": True}, {"index": "0"}, {"path": "/bin/sh"}, {}):
            self.assertEqual(self.req("POST", "/api/blender/install", bad)[0], 400, bad)         # a path from the request is never run
        self.assertEqual(len(calls), 2)
        st, _ = self.req("POST", "/api/blender/install", {"index": 0}, headers={"Origin": "http://evil.example"})
        self.assertEqual(st, 403)                                                                # a web page cannot start Blender
        self.assertEqual(len(calls), 2)

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
