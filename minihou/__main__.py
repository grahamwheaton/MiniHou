"""Command line: ``python -m minihou <serve|cook|nodes> ...``"""
from __future__ import annotations

import argparse
import logging
import sys
import time

from . import REGISTRY, Evaluator, Graph, GraphError
from .link.protocol import DEFAULT_PORT
from .web.server import DEFAULT_HTTP_PORT


def _cmd_nodes(args) -> int:
    _library(args)
    for nt in REGISTRY:
        ins = ", ".join(i.name + ("*" if i.multi else "") for i in nt.inputs) or "-"
        print(f"{nt.name:16s} in: {ins}")
        for p in nt.params:
            rng = ""
            if p.min is not None or p.max is not None:
                rng = f" [{'' if p.min is None else p.min}..{'' if p.max is None else p.max}]"
            print(f"    {p.name:14s} {p.kind:6s} default={p.to_json_value(p.default)!r}{rng}")
    return 0


def _library(args):
    """Node libraries from --nodes, else <root or cwd>/nodes. Loaded before any graph that uses them."""
    import os
    from .library import NodeLibrary
    dirs = args.nodes or [os.path.join(getattr(args, "root", None) or os.getcwd(), "nodes")]
    lib = NodeLibrary(dirs)
    lib.refresh()
    for f in lib.status()["files"]:
        print(f"node library {f['file']}: " + (f"ERROR {f['error']}" if f["error"] else ", ".join(f["nodes"])), flush=True)
    return lib


def _cmd_new_node(args) -> int:
    import os
    import re
    from .library import TEMPLATE
    if not re.match(r"^[a-z_][a-z0-9_]*$", args.name):
        print("node names are lower_case_with_underscores", file=sys.stderr)
        return 2
    folder = args.dir or os.path.join(os.getcwd(), "nodes")
    os.makedirs(folder, exist_ok=True)
    path = os.path.join(folder, args.name + ".py")
    if os.path.exists(path):
        print(f"{path} already exists", file=sys.stderr)
        return 2
    with open(path, "w", encoding="utf-8", newline="\n") as f:
        f.write(TEMPLATE.format(name=args.name))
    print(f"wrote {path}; a running engine picks it up within a second (Tab menu: '{args.name}')")
    return 0


def _cmd_cook(args) -> int:
    _library(args)
    try:
        g = Graph.load(args.graph)
    except GraphError as e:
        print("graph error:\n  " + "\n  ".join(e.errors), file=sys.stderr)
        return 2
    ev = Evaluator()
    for i in range(args.repeat):
        geo = ev.cook(g, args.node)
        s = ev.last_stats
        print(f"cook {i + 1}: {s.total_ms:.1f} ms  cooked={s.cooked}  cached={s.cached}")
    print(geo)
    return 0


def _cmd_serve(args) -> int:
    from .link.server import LinkServer
    from .link.session import Session
    logging.basicConfig(level=logging.INFO, format="%(asctime)s %(levelname)s %(message)s", datefmt="%H:%M:%S")
    library = _library(args)                       # before the graph: it may use library node types
    try:
        g = Graph.load(args.graph, check=False)       # an unfinished graph (saved by the editor) still opens
    except (GraphError, OSError) as e:
        print("graph error:\n  " + "\n  ".join(getattr(e, "errors", [str(e)])), file=sys.stderr)
        return 2
    from . import accel
    print(f"compute backend: {accel.status()}", flush=True)
    accel.warmup_async()          # compile/load kernels in the background; a cook that arrives early just waits
    session = Session(g, cache_bytes=int(args.cache_mb * 1024 * 1024))
    session.attach_library(library)
    server = LinkServer(session, args.host, args.port)
    web = None
    try:
        port = server.start()
        if args.http_port >= 0:
            from .web import WebServer
            web = WebServer(session, args.host, args.http_port, link=server, root=args.root)
            web.start()
            server.editor_url = web.url
    except OSError as e:
        print(f"cannot start server: {e}", file=sys.stderr)
        server.stop()
        return 3
    session.start()
    if not args.no_watch:
        session.watch_file(args.graph, save=not args.no_save)
    if web:
        print(f"editor: {web.url}", flush=True)
    print(f"minihou serving {args.graph} on {args.host}:{port}  (Ctrl+C to quit)", flush=True)
    if web and args.open:
        import webbrowser
        webbrowser.open(web.url)

    def _graceful(signum, frame):              # SIGTERM / closing the console window: still save the graph
        raise KeyboardInterrupt
    import signal
    for name in ("SIGTERM", "SIGBREAK", "SIGHUP"):
        if hasattr(signal, name):
            try:
                signal.signal(getattr(signal, name), _graceful)
            except (OSError, ValueError):
                pass
    try:
        while True:
            time.sleep(0.5)
    except KeyboardInterrupt:
        print("shutting down")
    finally:
        if web:
            web.stop()
        server.stop()
        session.stop()
    return 0


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(prog="minihou")
    sub = ap.add_subparsers(dest="cmd", required=True)
    s = sub.add_parser("serve", help="serve a graph to Blender over the live link")
    s.add_argument("graph")
    s.add_argument("--host", default="127.0.0.1")
    s.add_argument("--port", type=int, default=DEFAULT_PORT)
    s.add_argument("--no-watch", action="store_true", help="do not reload the graph when the file changes")
    s.add_argument("--no-save", action="store_true", help="do not write edits back to the graph file")
    s.add_argument("--http-port", type=int, default=DEFAULT_HTTP_PORT,
                   help=f"port of the web editor and API (default {DEFAULT_HTTP_PORT}; 0 = any free port, -1 = off)")
    s.add_argument("--root", default=None, help="workspace folder the editor may open graphs from (default: cwd)")
    s.add_argument("--open", action="store_true", help="open the editor in a web browser")
    s.add_argument("--nodes", action="append", metavar="DIR",
                   help="node library folder, hot-reloaded (repeatable; default: <root>/nodes)")
    s.add_argument("--cache-mb", type=float, default=2048)
    s.set_defaults(fn=_cmd_serve)
    c = sub.add_parser("cook", help="cook a graph once and print timings")
    c.add_argument("graph")
    c.add_argument("--node", default=None)
    c.add_argument("--repeat", type=int, default=2, help="cook N times to show cache behaviour")
    c.add_argument("--nodes", action="append", metavar="DIR", help="node library folder (default: ./nodes)")
    c.set_defaults(fn=_cmd_cook)
    n = sub.add_parser("nodes", help="list node types and parameters")
    n.add_argument("--nodes", action="append", metavar="DIR", help="node library folder (default: ./nodes)")
    n.set_defaults(fn=_cmd_nodes)
    nn = sub.add_parser("new-node", help="write a starter node-library file (nodes/NAME.py)")
    nn.add_argument("name")
    nn.add_argument("--dir", help="folder (default: ./nodes)")
    nn.set_defaults(fn=_cmd_new_node)
    m = sub.add_parser("mcp", help="MCP server (stdio) so AI assistants can build graphs in the running engine",
                       add_help=False)
    m.set_defaults(fn=None)
    if argv is None:
        argv = sys.argv[1:]
    if argv and argv[0] == "mcp":
        from .mcp import main as mcp_main
        return mcp_main(argv[1:])
    args = ap.parse_args(argv)
    return args.fn(args)


if __name__ == "__main__":
    sys.exit(main())
