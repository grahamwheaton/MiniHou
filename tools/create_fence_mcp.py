"""Create the sample fence graph through the running Rust app's MCP tool layer."""
import json
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
from minihou.mcp import EngineClient, McpServer

OUT = ROOT / "rust/crates/minihou-app/src/test_data/fence.minihou"
engine = EngineClient("http://127.0.0.1:8766")
state = engine.get("/api/state")
if state.get("engine", {}).get("engine") != "rust":
    raise SystemExit("8766 is not the Rust app")
mcp = McpServer(engine, workspace=ROOT)
ops = [
    {"op": "replace", "graph": {"version": 1, "nodes": {}, "display": None}},
    {"op": "add", "id": "sample_path", "type": "curve", "params": {"points": "0,0,0  2,0,0  4,0.4,0  6,0.4,0"}},
    {"op": "add", "id": "blender_path", "type": "object_input", "comment": "Pick a Blender curve, then connect this node to garden_fence.path"},
    {"op": "add", "id": "garden_fence", "type": "fence", "params": {"bay_width": 2.0, "height": 1.8,
        "rail_height": 0.85, "slat_width": 0.10, "slat_gap": 0.05,
        "batten_thickness": 0.07, "post_size": 0.12}},
    {"op": "connect", "from": "sample_path", "to": "garden_fence", "port": "path"},
    {"op": "add", "id": "output1", "type": "output"},
    {"op": "connect", "from": "garden_fence", "to": "output1", "port": "geo"},
    {"op": "display", "id": "output1"},
]
result = mcp.call_tool("edit_graph", {"ops": ops})
if result["isError"]:
    raise SystemExit(result["content"][0]["text"])
snapshot = engine.get("/api/graph")
if snapshot["graph"]["nodes"]["garden_fence"]["type"] != "fence":
    raise SystemExit("MCP graph was not accepted")
status = engine.get("/api/status?wait=1&timeout=30")
if status.get("ok") is not True:
    raise SystemExit(f"fence cook failed: {status}")
OUT.write_text(json.dumps(snapshot["graph"], indent=2) + "\n", encoding="utf-8")
print(json.dumps({"path": str(OUT), "rev": snapshot["rev"], "status": status}))
