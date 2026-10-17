"""Opt-in live contract check for the native Rust HTTP service and Python MCP tools.

Run with ``python -m unittest tests.test_mcp_live`` after building the debug app.
An existing Rust editor is used only when its document is clean and file-backed.
"""
import json
from pathlib import Path
import subprocess
import time
import unittest
import uuid

from minihou.mcp import EngineClient, McpServer

ROOT = Path(__file__).resolve().parents[1]
BINARY = ROOT / "rust" / "target_fence" / "debug" / "minihou-app.exe"
URL = "http://127.0.0.1:8766"


class LiveMcp(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.engine = EngineClient(URL)
        cls.process = None
        cls.untitled_existed = (ROOT / "untitled.json").exists()
        try:
            state = cls.engine.get("/api/state")
        except Exception:
            if not BINARY.is_file():
                raise unittest.SkipTest(f"debug binary missing: {BINARY}")
            cls.process = subprocess.Popen([str(BINARY)], cwd=ROOT,
                                           stdout=subprocess.DEVNULL, stderr=subprocess.PIPE,
                                           env={**__import__("os").environ, "MINIHOU_NO_DIALOG": "1"})
            for _ in range(100):
                if cls.process.poll() is not None:
                    raise RuntimeError(f"native app exited {cls.process.returncode}: {cls.process.stderr.read().decode(errors='replace')[:500]}")
                try:
                    state = cls.engine.get("/api/state")
                    break
                except Exception:
                    time.sleep(0.2)
            else:
                cls.process.terminate()
                raise RuntimeError("native app did not serve 8766")
        if state.get("engine", {}).get("engine") != "rust":
            raise unittest.SkipTest("8766 is not the Rust app")
        cls.original = state
        if cls.process is None and (state.get("dirty") or state.get("live_modifier") or not Path(state.get("path") or "").is_file()):
            raise unittest.SkipTest("existing native document is not safe to replace and restore")
        cls.mcp = McpServer(cls.engine, workspace=Path(state.get("root", ROOT)))

    @classmethod
    def tearDownClass(cls):
        if getattr(cls, "process", None):
            cls.process.terminate()
            try:
                cls.process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                cls.process.kill()
                cls.process.wait(timeout=5)
            if not cls.untitled_existed:
                (ROOT / "untitled.json").unlink(missing_ok=True)
        elif getattr(cls, "original", None) and getattr(cls, "mcp", None):
            cls.engine.post("/api/open", {"path": cls.original["path"]})

    def tool(self, name, **args):
        result = self.mcp.call_tool(name, args)
        self.assertIsInstance(result.get("content"), list)
        self.assertFalse(result["isError"], f"{name}: {result['content']}")
        text = result["content"][0]["text"]
        self.assertIsInstance(text, str)
        return text

    def test_live_contract(self):
        state = self.engine.get("/api/state")
        types = self.engine.get("/api/nodes")
        self.assertIn("graph", state)
        self.assertIsInstance(types, list)
        self.assertTrue(all({"name", "inputs", "params"} <= t.keys() for t in types))
        self.assertIn("Engine running", self.tool("engine_status"))
        self.assertIn("fence", self.tool("list_node_types", names=["box", "curve", "fence", "output"]))
        self.assertIn("nodes", self.tool("get_graph"))

        graph = {"version": 1, "nodes": {
            "path": {"type": "curve", "params": {"points": "0,0,0  3,0,0  6,0,0"}},
            "fence1": {"type": "fence", "inputs": {"path": "path"}, "params": {"bay_width": 2.0}},
            "output1": {"type": "output", "inputs": {"geo": "fence1"}}}, "display": "output1"}
        ops = [{"op": "replace", "graph": graph},
               {"op": "set", "id": "fence1", "params": {"slat_gap": 0.05}}]
        self.assertIn("Applied", self.tool("edit_graph", ops=ops))
        snap = self.engine.get("/api/graph")
        self.assertEqual(snap["graph"]["nodes"]["fence1"]["type"], "fence")
        self.assertEqual(len(self.engine.post("/api/edit", {"ops": [{"op": "add", "type": "box", "id": "test_box"},
            {"op": "connect", "from": "test_box", "to": "output1"},
            {"op": "set", "id": "test_box", "params": {"size": [1, 2, 3]}}], "wait": True})["results"]), 3)
        inspected = json.loads(self.tool("inspect_node", node="output1"))
        self.assertTrue({"node", "points", "prims", "vertices", "bounds"} <= inspected.keys())
        self.assertIn("Undid 1", self.tool("undo"))
        self.assertNotIn("test_box", self.engine.get("/api/graph")["graph"]["nodes"])
        self.assertIn("Redid 1", self.tool("undo", redo=True))
        self.tool("undo")
        export = ROOT / "rust" / "crates" / "minihou-app" / "src" / "test_data" / ("mcp_live_" + uuid.uuid4().hex + ".usda")
        try:
            result = self.tool("export_usd", path=str(export), overwrite=False)
            self.assertIn("Wrote", result)
            self.assertTrue(export.is_file())
        finally:
            export.unlink(missing_ok=True)
