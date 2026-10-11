"""Register the MiniHou MCP server with AI assistants, using *this* Python interpreter.

    py tools\\setup_mcp.py            (Windows)
    python3 tools/setup_mcp.py        (macOS / Linux)

* Writes ``.mcp.json`` in the repository root, so Claude Code started in this folder offers the ``minihou`` tools
  (it asks you to approve the server the first time).
* Prints the snippet for Claude Desktop's ``claude_desktop_config.json`` (and where that file lives).
* Checks the server actually starts and answers the MCP handshake.

``.mcp.json`` holds absolute paths for this machine, so it is git-ignored.
"""
import json
import os
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SERVER = os.path.join(ROOT, "tools", "mcp_server.py")


def server_entry(python=sys.executable):
    return {"type": "stdio", "command": python, "args": [SERVER]}


def write_mcp_json(path=os.path.join(ROOT, ".mcp.json"), python=sys.executable):
    data = {}
    if os.path.exists(path):
        try:
            with open(path, encoding="utf-8") as f:
                data = json.load(f)
        except (OSError, ValueError):
            data = {}
    data.setdefault("mcpServers", {})["minihou"] = server_entry(python)
    with open(path, "w", encoding="utf-8", newline="\n") as f:
        json.dump(data, f, indent=2)
        f.write("\n")
    return path


def handshake(python=sys.executable, timeout=60):
    """Start the server like a client would and complete initialize + tools/list. Returns the tool names."""
    msgs = [{"jsonrpc": "2.0", "id": 1, "method": "initialize",
             "params": {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "setup", "version": "1"}}},
            {"jsonrpc": "2.0", "method": "notifications/initialized"},
            {"jsonrpc": "2.0", "id": 2, "method": "tools/list"}]
    data = "".join(json.dumps(m) + "\n" for m in msgs).encode("utf-8")
    p = subprocess.run([python, SERVER], input=data, capture_output=True, timeout=timeout, cwd=os.path.expanduser("~"))
    replies = [json.loads(line) for line in p.stdout.decode("utf-8").splitlines() if line.strip()]
    if len(replies) != 2 or "result" not in replies[1]:
        raise RuntimeError(f"unexpected reply: {p.stdout!r}\nstderr: {p.stderr.decode('utf-8', 'replace')}")
    return [t["name"] for t in replies[1]["result"]["tools"]]


def desktop_config_path():
    if os.name == "nt":
        return os.path.join(os.environ.get("APPDATA", "%APPDATA%"), "Claude", "claude_desktop_config.json")
    if sys.platform == "darwin":
        return os.path.expanduser("~/Library/Application Support/Claude/claude_desktop_config.json")
    return os.path.expanduser("~/.config/Claude/claude_desktop_config.json")


def main():
    path = write_mcp_json()
    print(f"wrote {path}")
    tools = handshake()
    print(f"server OK: {len(tools)} tools ({', '.join(tools)})")
    print("\nClaude Code: start `claude` in this folder and approve the 'minihou' server when asked"
          " (or check with /mcp).")
    print(f"\nClaude Desktop: add this to {desktop_config_path()} and restart the app:")
    print(json.dumps({"mcpServers": {"minihou": {"command": sys.executable, "args": [SERVER]}}}, indent=2))
    print("\nThen ask for a system, e.g. \"Build a terrain with scattered rocks and expose the rock size to Blender\".")
    print("Keep the engine running (py tools\\launch_blender.py --engine graphs\\x.json), or let the AI start it with"
          " open_graph. Watch it happen in the editor at http://127.0.0.1:8766/")
    return 0


if __name__ == "__main__":
    sys.exit(main())
