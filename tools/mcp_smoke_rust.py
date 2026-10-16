"""Exercise MCP HTTP against an already-running Rust editor; starts no processes.

Default is read-only. --edit opens a new scratch graph and returns to the initial
file (Open resets history). --export additionally writes a scratch USDA file.
"""
import argparse
import json
from pathlib import Path
import sys
import uuid

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
from minihou.mcp import EngineClient, McpServer, ToolError


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--url", default="http://127.0.0.1:8766")
    parser.add_argument("--edit", action="store_true")
    parser.add_argument("--export", action="store_true", help="requires --edit and usd-core in the app's Python")
    args = parser.parse_args(argv)
    if args.export and not args.edit:
        parser.error("--export requires --edit")
    engine = EngineClient(args.url)
    state = engine.get("/api/state")
    if state.get("engine", {}).get("engine") != "rust":
        raise RuntimeError("Expected the Rust editor on this URL; refusing to change another engine")
    mcp = McpServer(engine, workspace=Path(state.get("root", ROOT)))

    def call(name, **kwargs):
        result = mcp.call_tool(name, kwargs)
        text = result["content"][0]["text"]
        print(f"{name}: {'FAIL' if result['isError'] else 'OK'}\n{text[:900]}\n")
        if result["isError"]:
            raise RuntimeError(text)
        return text

    call("engine_status")
    call("list_node_types", names=["box", "transform", "output"])
    call("get_graph")
    call("list_graphs")
    call("set_proxy")
    call("node_library")
    call("wrangle_reference")
    graph = state["graph"]
    display = graph.get("display")
    if isinstance(display, list):
        display = display[0] if display else None
    if display:
        call("inspect_node", node=display)
    if not args.edit:
        print("PASS: read-only Rust MCP smoke. No graph edits or processes started.")
        return 0
    if state.get("dirty"):
        raise RuntimeError("Save the current document before --edit (Open resets its history)")
    if state.get("live_modifier"):
        raise RuntimeError("Disconnect the live Blender modifier before --edit opens its scratch document")
    original_path = state.get("path")
    if not original_path or not Path(original_path).is_file():
        raise RuntimeError("Save the current document to a file before --edit")
    name = "mcp_smoke_" + uuid.uuid4().hex[:10]
    scratch = f"graphs/{name}.json"
    try:
        call("open_graph", path=scratch, create=True)
        call("edit_graph", ops=[
            {"op": "add", "id": "mcp_shape", "type": "box", "params": {"size": [2, 4, 6]}},
            {"op": "insert", "after": "mcp_shape", "type": "transform", "id": "mcp_move"},
            {"op": "connect", "from": "mcp_move", "to": "output1"},
            {"op": "expose", "id": "mcp_shape", "param": "size", "name": "Dimensions"},
            {"op": "meta", "id": "mcp_shape", "set": {"comment": "Created by unchanged Python MCP"}},
            {"op": "display", "id": "output1"},
            {"op": "box_add", "around": ["mcp_shape", "mcp_move", "output1"], "title": "MCP smoke"},
        ])
        inspected = engine.post("/api/inspect", {"node": "output1", "detail": True})
        assert inspected["ok"] and inspected["points"] == 8, inspected
        assert inspected["bounds"]["size"] == [2, 4, 6], inspected
        call("inspect_node", node="output1")
        before = engine.get("/api/graph")
        try:
            engine.post("/api/edit", {"ops": [
                {"op": "set", "id": "mcp_shape", "params": {"size": [99, 99, 99]}},
                {"op": "add", "type": "__unavailable_python_node__"}], "wait": True})
        except ToolError as exc:
            print("atomic rejection: OK", exc)
        else:
            raise AssertionError("invalid batch was accepted")
        after = engine.get("/api/graph")
        assert after["graph"] == before["graph"] and after["rev"] == before["rev"], "partial batch mutation"
        call("undo")
        assert "mcp_shape" not in engine.get("/api/graph")["graph"]["nodes"]
        call("undo", redo=True)
        assert "mcp_shape" in engine.get("/api/graph")["graph"]["nodes"]
        call("set_frame", frame=10, fps=24)
        call("set_proxy", level=0.5, auto=False, link_tris=10000)
        call("set_proxy", level=1, auto=False, link_tris=0)
        if args.export:
            call("export_usd", path=f"exports/{name}.usda", overwrite=False)
        print(f"PASS: mutations, rollback, inspection and undo/redo. Scratch file: {scratch}")
    finally:
        engine.post("/api/open", {"path": original_path})
        old_status = state["status"]
        engine.post("/api/frame", {"frame": old_status.get("frame", 1), "fps": old_status.get("fps", 24)})
        old_proxy = old_status.get("proxy") or {}
        engine.post("/api/proxy", {k: old_proxy[k] for k in ("level", "auto", "link_tris") if k in old_proxy})
        engine.get("/api/status?wait=1&timeout=30")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (ToolError, RuntimeError, AssertionError, OSError) as exc:
        print(f"FAIL: {exc}", file=sys.stderr)
        sys.exit(1)
