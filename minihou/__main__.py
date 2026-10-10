"""Command line: ``python -m minihou <serve|cook|nodes> ...``"""
from __future__ import annotations

import argparse
import logging
import sys
import time

from . import REGISTRY, Evaluator, Graph, GraphError
from .link.protocol import DEFAULT_PORT


def _cmd_nodes(args) -> int:
    for nt in REGISTRY:
        ins = ", ".join(i.name + ("*" if i.multi else "") for i in nt.inputs) or "-"
        print(f"{nt.name:16s} in: {ins}")
        for p in nt.params:
            rng = ""
            if p.min is not None or p.max is not None:
                rng = f" [{'' if p.min is None else p.min}..{'' if p.max is None else p.max}]"
            print(f"    {p.name:14s} {p.kind:6s} default={p.to_json_value(p.default)!r}{rng}")
    return 0


def _cmd_cook(args) -> int:
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
    try:
        g = Graph.load(args.graph)
    except GraphError as e:
        print("graph error:\n  " + "\n  ".join(e.errors), file=sys.stderr)
        return 2
    from . import accel
    print(f"compute backend: {accel.status()}", flush=True)
    accel.warmup_async()          # compile/load kernels in the background; a cook that arrives early just waits
    session = Session(g, cache_bytes=int(args.cache_mb * 1024 * 1024))
    server = LinkServer(session, args.host, args.port)
    try:
        port = server.start()
    except OSError as e:
        print(f"cannot start server: {e}", file=sys.stderr)
        return 3
    session.start()
    if not args.no_watch:
        session.watch_file(args.graph)
    print(f"minihou serving {args.graph} on {args.host}:{port}  (Ctrl+C to quit)", flush=True)
    try:
        while True:
            time.sleep(0.5)
    except KeyboardInterrupt:
        print("shutting down")
    finally:
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
    s.add_argument("--cache-mb", type=float, default=2048)
    s.set_defaults(fn=_cmd_serve)
    c = sub.add_parser("cook", help="cook a graph once and print timings")
    c.add_argument("graph")
    c.add_argument("--node", default=None)
    c.add_argument("--repeat", type=int, default=2, help="cook N times to show cache behaviour")
    c.set_defaults(fn=_cmd_cook)
    n = sub.add_parser("nodes", help="list node types and parameters")
    n.set_defaults(fn=_cmd_nodes)
    args = ap.parse_args(argv)
    return args.fn(args)


if __name__ == "__main__":
    sys.exit(main())
