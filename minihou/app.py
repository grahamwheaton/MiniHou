"""The packed application: ``MiniHou`` (or ``python -m minihou app``) -- one program that starts the engine and opens the editor.

It keeps everything in one workspace folder (default ``~/MiniHou``): ``graphs/`` for your graphs (a welcome graph and the
examples are copied in the first time), ``nodes/`` for your own node files. The editor's top bar offers to install the
Blender add-on and to open Blender connected, so nothing has to be set up by hand.
"""
from __future__ import annotations

import argparse
import os
import shutil
import socket
import sys
from pathlib import Path
from typing import List, Optional

WELCOME = "graphs/welcome.json"


def default_workspace() -> Path:
    if os.environ.get("MINIHOU_WORKSPACE"):
        return Path(os.environ["MINIHOU_WORKSPACE"]).expanduser()
    docs = Path.home() / "Documents"
    return (docs if docs.is_dir() else Path.home()) / "MiniHou"


def examples_source() -> Optional[Path]:
    """The bundled ``examples/graphs`` folder (next to the package in a checkout, inside the app when frozen)."""
    here = Path(__file__).resolve().parent
    cands = []
    if getattr(sys, "frozen", False):
        base = Path(getattr(sys, "_MEIPASS", None) or Path(sys.executable).parent)
        cands += [base / "examples" / "graphs", Path(sys.executable).parent / "examples" / "graphs"]
    cands += [here.parent / "examples" / "graphs", here / "_examples"]
    for c in cands:
        if c.is_dir() and any(c.glob("*.json")):
            return c
    return None


def prepare_workspace(root: Path) -> Path:
    """Create the folders and copy the examples the first time; returns the graph to open. Never overwrites anything."""
    root.mkdir(parents=True, exist_ok=True)
    (root / "graphs").mkdir(exist_ok=True)
    (root / "nodes").mkdir(exist_ok=True)
    src = examples_source()
    if src is not None:
        dst = root / "examples"
        if not dst.exists():
            shutil.copytree(src, dst)
        welcome = root / WELCOME
        if not welcome.exists() and (src / "demo.json").is_file():
            shutil.copyfile(src / "demo.json", welcome)
    for cand in (root / WELCOME, *sorted((root / "graphs").glob("*.json"))):
        if cand.is_file():
            return cand
    starter = root / "graphs" / "untitled.json"
    starter.write_text('{"version": 1, "nodes": {}}\n', encoding="utf-8")
    return starter


def _port_free(host: str, port: int) -> bool:
    if port <= 0:
        return True
    s = socket.socket()
    try:
        if os.name == "nt":
            s.setsockopt(socket.SOL_SOCKET, socket.SO_EXCLUSIVEADDRUSE, 1)
        s.bind((host, port))
        return True
    except OSError:
        return False
    finally:
        s.close()


def main(argv: Optional[List[str]] = None) -> int:
    from .__main__ import _cmd_serve
    from .link.protocol import DEFAULT_PORT
    from .web.server import DEFAULT_HTTP_PORT
    ap = argparse.ArgumentParser(prog="minihou app", description=__doc__.split("\n\n")[0])
    ap.add_argument("graph", nargs="?", help="graph file to open (default: the welcome graph)")
    ap.add_argument("--workspace", default=None, help="folder for graphs and nodes (default: ~/MiniHou)")
    ap.add_argument("--host", default="127.0.0.1")
    ap.add_argument("--port", type=int, default=DEFAULT_PORT, help="live-link port Blender connects to (another is used if busy)")
    ap.add_argument("--http-port", type=int, default=DEFAULT_HTTP_PORT, help="editor port (another is used if busy)")
    ap.add_argument("--no-browser", action="store_true", help="do not open the editor in a browser")
    ap.add_argument("--allow-python", action="store_true", help="let python nodes run code stored in graphs (trusted graphs only)")
    ap.add_argument("--disk-cache", action="store_true", help="keep expensive results on disk (in the workspace) so reopening a big scene is quick")
    args = ap.parse_args(argv)

    root = Path(args.workspace).expanduser() if args.workspace else default_workspace()
    graph = Path(args.graph).expanduser() if args.graph else prepare_workspace(root)
    if args.graph:
        root.mkdir(parents=True, exist_ok=True)
        prepare_workspace(root)
    if not _port_free(args.host, args.port):
        print(f"port {args.port} is busy (another MiniHou?): using a free one; Blender is started with the right port by the editor's button.",
              flush=True)
        args.port = 0
    if not _port_free(args.host, args.http_port):
        args.http_port = 0
    ns = argparse.Namespace(graph=str(graph), host=args.host, port=args.port, http_port=args.http_port, root=str(root),
                            open=not args.no_browser, nodes=[str(root / "nodes")], cache_mb=2048.0, no_watch=False, no_save=False,
                            allow_python=args.allow_python, disk_cache="auto" if args.disk_cache else None, disk_cache_gb=4.0)
    print(f"MiniHou workspace: {root}", flush=True)
    return _cmd_serve(ns)


if __name__ == "__main__":
    sys.exit(main())
