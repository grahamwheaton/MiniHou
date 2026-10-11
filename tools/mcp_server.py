"""Entry point for MCP clients: runs ``minihou.mcp`` from this checkout, whatever the current directory is.

    python tools/mcp_server.py [--url http://127.0.0.1:8766]

Register it with ``python tools/setup_mcp.py`` (writes .mcp.json for Claude Code and prints the Claude Desktop config).
"""
import os
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
if ROOT not in sys.path:
    sys.path.insert(0, ROOT)

from minihou.mcp import main  # noqa: E402

if __name__ == "__main__":
    sys.exit(main())
