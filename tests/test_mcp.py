"""MCP server (minihou.mcp): protocol, every tool against a real engine, the stdio process, starting an engine."""
import json
import os
import shutil
import signal
import socket
import subprocess
import sys
import tempfile
import time
import unittest
from pathlib import Path

from minihou import Graph
from minihou.link.server import LinkServer
from minihou.link.session import Session
from minihou.mcp import PROTOCOL_VERSIONS, TOOLS, EngineClient, McpServer
from minihou.web import WebServer

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


def free_port():
    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    port = s.getsockname()[1]
    s.close()
    return port


def call(server, name, **args):
    r = server.handle({"jsonrpc": "2.0", "id": 7, "method": "tools/call", "params": {"name": name, "arguments": args}})
    res = r["result"]
    return res["content"][0]["text"], res["isError"]


class ProtocolTests(unittest.TestCase):
    def setUp(self):
        self.m = McpServer(EngineClient(f"http://127.0.0.1:{free_port()}"))       # nothing listens there

    def test_initialize_negotiates_the_version(self):
        for asked, want in [("2025-06-18", "2025-06-18"), ("2024-11-05", "2024-11-05"), ("2099-01-01", PROTOCOL_VERSIONS[0])]:
            r = self.m.handle({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": asked}})
            self.assertEqual(r["result"]["protocolVersion"], want)
        self.assertIn("tools", r["result"]["capabilities"])
        self.assertIn("edit_graph", r["result"]["instructions"])

    def test_notifications_errors_and_ping(self):
        self.assertIsNone(self.m.handle({"jsonrpc": "2.0", "method": "notifications/initialized"}))
        self.assertIsNone(self.m.handle({"jsonrpc": "2.0", "method": "notifications/cancelled", "params": {}}))
        self.assertEqual(self.m.handle({"jsonrpc": "2.0", "id": 2, "method": "ping"})["result"], {})
        self.assertEqual(self.m.handle({"jsonrpc": "2.0", "id": 3, "method": "nope"})["error"]["code"], -32601)
        r = self.m.handle({"jsonrpc": "2.0", "id": 4, "method": "tools/call", "params": {"name": "rm_rf"}})
        self.assertEqual(r["error"]["code"], -32602)
        self.assertEqual(self.m.handle({"id": 5, "method": "ping"})["error"]["code"], -32600)

    def test_tool_list_is_well_formed(self):
        tools = self.m.handle({"jsonrpc": "2.0", "id": 1, "method": "tools/list"})["result"]["tools"]
        self.assertEqual(len(tools), len(TOOLS))
        for t in tools:
            self.assertTrue(t["name"].isidentifier())
            self.assertGreater(len(t["description"]), 20)
            self.assertEqual(t["inputSchema"]["type"], "object")
            self.assertTrue(callable(getattr(self.m, "tool_" + t["name"])))

    def test_engine_down_is_explained_and_docs_still_work(self):
        text, err = call(self.m, "engine_status")
        self.assertFalse(err)
        self.assertIn("not running", text)
        self.assertIn("open_graph", text)
        text, err = call(self.m, "edit_graph", ops=[{"op": "add", "type": "box"}])
        self.assertTrue(err)
        self.assertIn("not reachable", text)
        text, err = call(self.m, "list_node_types", names=["scatter"])        # falls back to the local registry
        self.assertFalse(err)
        self.assertIn("scatter [generate]", text)
        self.assertIn("count: int", text)
        text, err = call(self.m, "wrangle_reference")
        self.assertIn("rand(x, seed=0)", text)
        _, err = call(self.m, "inspect_node")                                 # missing argument
        self.assertTrue(err)


class ToolTests(unittest.TestCase):
    """Every tool against a real engine (Session + HTTP API), as an AI would use them."""

    def setUp(self):
        self.dir = tempfile.mkdtemp(prefix="minihou_mcp_")
        self.addCleanup(shutil.rmtree, self.dir, True)
        os.makedirs(os.path.join(self.dir, "graphs"))
        self.path = os.path.join(self.dir, "graphs", "start.json")
        with open(self.path, "w", encoding="utf-8") as f:
            json.dump({"nodes": {"grid1": {"type": "grid"}, "output1": {"type": "output", "inputs": {"geo": "grid1"}}},
                       "display": "output1"}, f)
        self.s = Session(Graph.load(self.path, check=False))
        self.s.start()
        self.s.watch_file(self.path, interval=0.05)
        from minihou.library import NodeLibrary
        self.lib = NodeLibrary([os.path.join(self.dir, "nodes")])
        self.addCleanup(self.lib.unload)
        self.s.attach_library(self.lib, interval=0.05)
        self.link = LinkServer(self.s, "127.0.0.1", 0)
        self.link.start()
        self.web = WebServer(self.s, "127.0.0.1", 0, link=self.link, root=self.dir)
        port = self.web.start()
        for x in (self.s.stop, self.link.stop, self.web.stop):
            self.addCleanup(x)
        self.m = McpServer(EngineClient(f"http://127.0.0.1:{port}"), Path(self.dir))

    def test_build_a_system_step_by_step(self):
        text, err = call(self.m, "edit_graph", ops=[
            {"op": "set", "id": "grid1", "params": {"columns": 30, "rows": 30, "size_x": 10, "size_y": 10}},
            {"op": "add", "type": "noise", "id": "terrain", "inputs": {"geo": "grid1"},
             "params": {"amplitude": [0, 0, 1.5]}, "comment": "rolling hills"},
            {"op": "add", "type": "scatter", "id": "rock_pts", "inputs": {"geo": "terrain"}, "params": {"count": 50}},
            {"op": "add", "type": "box", "id": "rock", "params": {"size": [0.3, 0.3, 0.3]}},
            {"op": "add", "type": "copy_to_points", "id": "rocks", "inputs": {"source": "rock", "points": "rock_pts"}},
            {"op": "add", "type": "merge", "id": "all", "inputs": {"geos": ["terrain", "rocks"]}},
            {"op": "connect", "from": "all", "to": "output1"},
            {"op": "expose", "id": "rock_pts", "param": "count", "name": "Rock Count"}])
        self.assertFalse(err, text)
        self.assertIn("Nodes: terrain, rock_pts, rock, rocks, all", text)
        self.assertIn("Exposed to Blender as: Rock Count", text)
        self.assertIn("Cook OK", text)
        self.assertNotIn("not feeding", text)
        self.assertEqual(self.s.status()["points"], 900 + 50 * 8)

        text, err = call(self.m, "inspect_node", node="rock_pts")
        info = json.loads(text)
        self.assertEqual(info["points"], 50)
        self.assertLessEqual(info["bounds"]["max"][0], 5.0 + 1e-4)

        text, _ = call(self.m, "get_graph")
        self.assertIn('"rolling hills"', text)
        self.assertNotIn('"pos"', text)                                    # layout stripped by default
        self.assertIn('"pos"', call(self.m, "get_graph", include_layout=True)[0])

    def test_cook_errors_name_the_node_and_undo_recovers(self):
        text, err = call(self.m, "edit_graph", ops=[
            {"op": "add", "type": "wrangle", "id": "w", "inputs": {"geo": "grid1"}, "params": {"snippet": "@P.z = bogus"}},
            {"op": "connect", "from": "w", "to": "output1"}])
        self.assertFalse(err)                                              # the edit itself is fine...
        self.assertIn("COOK ERROR at node 'w'", text)                      # ...but the cook isn't
        self.assertIn("bogus", text)
        text, err = call(self.m, "inspect_node", node="output1")
        self.assertTrue(err)
        self.assertIn("failing node: w", text)
        text, err = call(self.m, "undo")
        self.assertIn("Undid 1", text)
        self.assertIn("Cook OK", text)
        self.assertNotIn("w", self.s.graph.nodes)

    def test_rejected_batches_report_the_op(self):
        text, err = call(self.m, "edit_graph", ops=[{"op": "add", "type": "box"}, {"op": "connect", "from": "x", "to": "y"}])
        self.assertTrue(err)
        self.assertIn("op 1 (connect)", text)
        self.assertNotIn("box1", self.s.graph.nodes)
        self.assertTrue(call(self.m, "edit_graph", ops=[])[1])

    def test_status_files_and_open(self):
        text, err = call(self.m, "engine_status")
        self.assertIn("Engine running", text)
        self.assertIn("Blender: not connected", text)
        self.assertIn("* graphs/start.json", call(self.m, "list_graphs")[0])
        text, err = call(self.m, "open_graph", path="graphs/second", create=True)
        self.assertFalse(err, text)
        self.assertTrue(os.path.exists(os.path.join(self.dir, "graphs", "second.json")))
        self.assertIn("second.json", text)
        text, err = call(self.m, "open_graph", path="../outside.json", create=True)
        self.assertTrue(err)
        self.assertIn("outside the workspace", text)

    def test_ai_writes_a_node_library_file_and_uses_it(self):
        text, _ = call(self.m, "node_library")
        self.assertIn("(no node files yet)", text)
        self.assertIn("@node(", text)                                       # the authoring guide comes with it
        name = f"ai_lift_{os.getpid()}"
        os.makedirs(os.path.join(self.dir, "nodes"))
        code = (f"from minihou.sdk import Float, node\n\n@node({name!r}, inputs=['geo'], params=[Float('h', 2.0)])\n"
                f"def {name}(geo, h):\n    out = geo.copy()\n    P = geo.P.copy()\n    P[:, 2] += h\n"
                f"    out.point['P'] = P\n    return out\n")
        with open(os.path.join(self.dir, "nodes", "ai.py"), "w", encoding="utf-8") as f:   # the AI's own file tool
            f.write(code)
        deadline = time.monotonic() + 5
        while name not in call(self.m, "list_node_types")[0] and time.monotonic() < deadline:
            time.sleep(0.05)
        text, err = call(self.m, "list_node_types", names=[name])
        self.assertFalse(err, text)
        self.assertIn("ai.py", text)
        text, err = call(self.m, "edit_graph", ops=[{"op": "add", "type": name, "id": "up", "inputs": {"geo": "grid1"}},
                                                     {"op": "connect", "from": "up", "to": "output1"}])
        self.assertIn("Cook OK", text)
        self.assertAlmostEqual(json.loads(call(self.m, "inspect_node", node="up")[0])["bounds"]["max"][2], 2.0)
        with open(os.path.join(self.dir, "nodes", "ai.py"), "a", encoding="utf-8") as f:
            f.write("\ndef broken(:\n")
        deadline = time.monotonic() + 5
        while "ERROR in" not in call(self.m, "engine_status")[0] and time.monotonic() < deadline:
            time.sleep(0.05)
        status = call(self.m, "engine_status")[0]
        self.assertIn("SyntaxError", status)
        self.assertIn("ai.py", call(self.m, "node_library")[0])

    def test_ai_builds_a_system_and_uses_it(self):
        name = f"ai_pillars_{os.getpid()}"
        text, err = call(self.m, "open_graph", path=f"nodes/{name}.json", create=True)
        self.assertFalse(err, text)
        text, err = call(self.m, "edit_graph", ops=[
            {"op": "add", "type": "input", "id": "ground", "inputs": {"preview": "grid1"}, "params": {"name": "ground"}},
            {"op": "add", "type": "scatter", "id": "spots", "inputs": {"geo": "ground"}, "params": {"count": 5}},
            {"op": "add", "type": "box", "id": "pillar", "params": {"size": [0.2, 0.2, 2]}},
            {"op": "add", "type": "copy_to_points", "id": "pillars", "inputs": {"source": "pillar", "points": "spots"},
             "params": {"orient": False}},
            {"op": "connect", "from": "pillars", "to": "output1"},
            {"op": "expose", "id": "spots", "param": "count", "name": "Pillar Count"}])
        self.assertIn("Cook OK", text)
        # open_graph(create=True) already wrote a starter file, so the type appears at once without inputs or params;
        # wait for the edited graph (saved with a debounce) to be reloaded
        deadline = time.monotonic() + 10
        while time.monotonic() < deadline:
            text, err = call(self.m, "list_node_types", names=[name])
            if not err and "pillar_count" in text:
                break
            time.sleep(0.05)
        text, err = call(self.m, "list_node_types", names=[name])
        self.assertFalse(err, text)
        self.assertIn("pillar_count: int = 5", text)
        self.assertIn("inputs: ground", text)
        call(self.m, "open_graph", path="graphs/start.json")
        text, err = call(self.m, "edit_graph", ops=[
            {"op": "add", "type": name, "id": "colonnade", "inputs": {"ground": "grid1"}, "params": {"pillar_count": 12}},
            {"op": "connect", "from": "colonnade", "to": "output1"}])
        self.assertIn("Cook OK", text, text)
        info = json.loads(call(self.m, "inspect_node", node="colonnade")[0])
        self.assertEqual(info["prims"], 12 * 6)                               # 12 pillars of 6 faces

    def test_ai_edits_reach_blender_clients(self):
        """An expose + value change from the AI is pushed to link clients as an interface update."""
        got = []

        class L:
            def on_result(self, r): pass
            def on_interface(self, i): got.append(i)

        self.s.add_listener(L())
        call(self.m, "edit_graph", ops=[{"op": "expose", "id": "grid1", "param": "rows", "name": "Rows"}])
        self.assertTrue(got and got[-1]["params"][0]["name"] == "Rows")


class StdioTests(unittest.TestCase):
    def run_server(self, lines, url):
        env = dict(os.environ, PYTHONPATH=ROOT, PYTHONIOENCODING="cp1252")   # hostile console encoding
        p = subprocess.run([sys.executable, os.path.join(ROOT, "tools", "mcp_server.py"), "--url", url],
                           input="".join(l + "\n" for l in lines).encode("utf-8"), capture_output=True, timeout=60,
                           cwd=tempfile.gettempdir(), env=env)
        out = p.stdout.decode("utf-8")
        return [json.loads(l) for l in out.splitlines()], p

    def test_stdout_carries_only_protocol_messages(self):
        url = f"http://127.0.0.1:{free_port()}"
        replies, p = self.run_server([
            json.dumps({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-03-26"}}),
            json.dumps({"jsonrpc": "2.0", "method": "notifications/initialized"}),
            "{this is not json",
            json.dumps([{"jsonrpc": "2.0", "id": 2, "method": "ping"}, {"jsonrpc": "2.0", "id": 3, "method": "tools/list"}]),
            json.dumps({"jsonrpc": "2.0", "id": 4, "method": "tools/call",
                        "params": {"name": "list_node_types", "arguments": {"names": ["wrangle"]}}}),
        ], url)
        self.assertEqual(p.returncode, 0, p.stderr)
        self.assertEqual(replies[0]["result"]["protocolVersion"], "2025-03-26")
        self.assertEqual(replies[1]["error"]["code"], -32700)
        self.assertEqual([r["id"] for r in replies[2]], [2, 3])
        text = replies[3]["result"]["content"][0]["text"]
        self.assertIn("wrangle [modify]", text)
        self.assertIn("…" if "…" in text else "snippet", text)            # non-ASCII survives a cp1252 console


class StartEngineTests(unittest.TestCase):
    def test_engine_that_cannot_start_is_reported_quickly(self):
        ws = tempfile.mkdtemp(prefix="minihou_mcp_ws_")
        self.addCleanup(shutil.rmtree, ws, True)
        blocker = socket.socket()
        blocker.bind(("127.0.0.1", 0))
        blocker.listen(1)
        self.addCleanup(blocker.close)
        m = McpServer(EngineClient(f"http://127.0.0.1:{free_port()}"), Path(ws), link_port=blocker.getsockname()[1])
        t0 = time.monotonic()
        text, err = call(m, "open_graph", path="graphs/x.json", create=True)
        self.assertTrue(err)
        self.assertIn("exited at once", text)
        self.assertLess(time.monotonic() - t0, 25)
        m.engine_process.wait(5)

    def test_open_graph_starts_an_engine_when_none_runs(self):
        ws = tempfile.mkdtemp(prefix="minihou_mcp_ws_")
        self.addCleanup(shutil.rmtree, ws, True)
        port = free_port()
        m = McpServer(EngineClient(f"http://127.0.0.1:{port}"), Path(ws), link_port=free_port())
        self.assertTrue(call(m, "open_graph", path="graphs/missing.json")[1])      # no create, no engine: error
        text, err = call(m, "open_graph", path="graphs/fresh.json", create=True)
        try:
            self.assertFalse(err, text)
            self.assertIn("Started the MiniHou engine", text)
            self.assertIn("Engine running", text)
            self.assertIn("Cook OK", text)
            text, err = call(m, "edit_graph", ops=[{"op": "set", "id": "grid1", "params": {"rows": 3}}])
            self.assertFalse(err, text)
        finally:
            pid = EngineClient(f"http://127.0.0.1:{port}").get("/api/state")["engine"]["pid"]
            os.kill(pid, signal.SIGTERM)
            m.engine_process.wait(15)
        with open(os.path.join(ws, "graphs", "fresh.json"), encoding="utf-8") as f:
            self.assertEqual(json.load(f)["nodes"]["grid1"]["params"]["rows"], 3)


if __name__ == "__main__":
    unittest.main()
