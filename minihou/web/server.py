"""HTTP API and node editor for a live :class:`~minihou.link.session.Session` (stdlib only, loopback by default).

The browser editor and the MCP server (``python -m minihou mcp``) both use this API, so edits from either show up
live in the other and in Blender.

=====================  =======================================================================================
``GET  /``             the node editor
``GET  /api/state``    everything an editor needs at start: graph, status, node types, files, link clients
``GET  /api/nodes``    node type schemas (ports, params with kinds/defaults/ranges, docs)
``GET  /api/graph``    ``{rev, graph, path, can_undo, can_redo}``
``GET  /api/status``   newest cook: ok/error (with the failing node), counts, timings; ``?wait=1`` waits for cooks
``GET  /api/events``   Server-Sent Events: ``graph`` (snapshot) and ``status`` events, latest-wins
``GET  /api/files``    graph files under the workspace root
``POST /api/edit``     ``{ops: [...], wait?: bool}`` -> ``{rev, results, changed, status?}`` (see :mod:`minihou.edit`)
                       Optional ``gesture`` token merges a modal drag into one undo entry. Send ``document_id`` to fence file switches.
``POST /api/gesture_cancel`` ``{gesture, document_id?}`` restores the drag's original graph, removes its undo entry and restores prior redo; refuses intervening graph edits
``POST /api/gesture_finish`` ``{gesture, document_id?}`` closes the drag, retaining one undo entry; closed tokens cannot continue editing
``GET /api/preferences`` persistent Python, copy-memory-limit (MiB) and proxy settings, with copy limit range metadata
``POST /api/preferences`` partial ``{python_nodes?, copy_memory_limit_mib?}``; user settings never come from a graph
``POST /api/undo``     also ``/api/redo``
``POST /api/inspect``  ``{node, detail?}`` -> geometry summary of any node
``POST /api/select_group`` ``{node}`` -> ``{node: first_input, domain: point|prim|edge, ids}``; resolves the group on input geometry without graph edits (at most 100000 ids)
``POST /api/preview``  ``{node, full?, tris?, attr?, mask?, instances?}`` -> positions, triangles and edges of any node for the 3-D view (binary, see
                       :mod:`minihou.preview`; a big one is reduced to the display budget unless ``full``, or to ``tris`` triangles),
                       or JSON ``{ok: false, error}``
``POST /api/export_usd`` ``{path, node?, overwrite?, graph?}`` write the cooked geometry (always at full size, whatever the proxy level)
                       to a ``.usd`` / ``.usda`` / ``.usdc`` file under the root: the display output and each extra output, or just
                       ``node``; the graph is stored in the file unless ``graph`` is false (see :mod:`minihou.usd_io`)
``POST /api/node_action`` ``{node, action}`` press a button of a node (``usd_export``: ``save``); returns the handler's dict (``message``...)
``GET  /api/browse``   ``?path=&exts=.usd,.usda&places=1&fallback=1`` the folders and matching files of any folder (names only), for the file picker
``POST /api/pick_file`` ``{mode: open|save, exts?, title?, start?}`` the operating system's file dialog on the engine's machine ->
                       ``{native: true, path, cancelled}`` (``path`` relative to the workspace when inside it), or ``{native: false}`` when
                       there is none (the editor then shows its own browser)
``POST /api/upload``   ``?name=`` the raw bytes of a dropped file (header ``X-MiniHou-Upload: 1`` required, streamed to disk, up to 16 GB) into
                       ``imports/`` under the root -> ``{path, size}``; ``.usd``/``.usda``/``.usdc``/``.usdz``/``.png`` only
``POST /api/spreadsheet`` ``{node, domain: point|vertex|prim|edge|detail, start?, count?, ids?, sort?, filter?, origins?}`` a page (at most 500) of rows of any node's geometry:
                       ``{total, columns, rows: [[element, value...]]}``; ``ids`` restricts to those elements (the selection), ``filter`` keeps matching rows
                       (point/prim: group syntax such as ``@w > 0.5``; vertex/edge: ``column <op> number and ...``), ``sort`` is ``{column, comp?, desc?}``,
                       ``origins`` adds ``{column: node that first made it}``
``POST /api/disk_cache`` ``{enabled?, clear?}`` turn the disk cache of expensive results on (``.minihou_cache`` in the root) or off, or
                       clear it; returns ``{enabled, stats}``
``POST /api/proxy``    ``{level?, auto?, link_tris?}`` the proxy settings (cook level 0.001..1, auto refine, Blender triangle cap);
                       also ``GET`` for the current ones
``POST /api/open``     ``{path, create?}`` switch to (or create) a graph file inside the workspace root
``GET  /api/library``  every graph file with name, age, node count, doc and whether it is a library node (for the file browser)
``GET  /api/file``     ``?path=`` one graph file's JSON (to insert its nodes into the open graph)
``POST /api/save_as``  ``{path, overwrite?}`` copy the open graph to a new file and switch to it
``POST /api/file_op``  ``{op: rename|duplicate|delete, path, to?}`` on a graph file (never the open one, except duplicate)
``POST /api/make_system`` ``{name, nodes, replace?, doc?, overwrite?}`` save the selected nodes as a library node; replace swaps them for it
``GET  /api/blender``  installed Blenders (by index) and the add-on zip, for the editor's Blender panel
``GET  /addon/minihou_link.zip``  the Blender add-on, ready to drag into Blender or install from disk
``POST /api/blender/install``  ``{index}`` install + enable the add-on in that Blender (runs it in the background once)
``POST /api/blender/launch``   ``{index}`` open that Blender with the add-on connected to this engine
``POST /api/frame``    ``{frame, fps?}`` set the current frame for @Frame / @Time (Blender's timeline also sets it)
=====================  =======================================================================================

Hardening, since any web page can try to talk to localhost: the ``Host`` header must be a loopback name (blocks DNS
rebinding), POSTs must be ``application/json`` (cross-site forms can't send that without a CORS preflight, which is
never granted) and a present ``Origin`` must be this server. File access is limited to ``.json`` under the root (and ``.usd``,
``.usda``, ``.usdc`` for the USD export).
"""
from __future__ import annotations

import errno
import json
import logging
import os
import socket
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from typing import Any, List, Optional
from urllib.parse import parse_qs, urlparse

from .. import actions, blender_setup, filedialog, paths, usd_io
from ..edit import OP_NAMES, EditError
from ..evaluator import NodeError
from ..graph import GraphError
from ..link.protocol import PROTOCOL_VERSION
from ..sdk import ParamError

log = logging.getLogger("minihou.web")

DEFAULT_HTTP_PORT = 8766
STATIC_DIR = Path(__file__).with_name("static")
MAX_BODY = 16 * 1024 * 1024
MAX_UPLOAD = 16 * 2**30
UPLOAD_EXTENSIONS = (".usd", ".usda", ".usdc", ".usdz", ".png")
SKIP_DIRS = {".git", "__pycache__", "node_modules", ".venv", "venv", "gui_test_output", ".pytest_cache"}
CONTENT_TYPES = {".html": "text/html; charset=utf-8", ".js": "text/javascript; charset=utf-8",
                 ".css": "text/css; charset=utf-8", ".svg": "image/svg+xml", ".png": "image/png",
                 ".ico": "image/x-icon", ".json": "application/json"}
LOOPBACK = ("127.0.0.1", "localhost", "[::1]", "::1")


def _budget(v) -> int:
    """A triangle budget from a request (0: the default)."""
    if v is None or isinstance(v, bool) or not isinstance(v, (int, float)) or v != v or v < 1000:
        return 0
    return int(min(v, 16_000_000))


class ApiError(Exception):
    def __init__(self, status: int, message: str, **extra):
        super().__init__(message)
        self.status, self.message, self.extra = status, message, extra


class _EventStream:
    """One SSE client: a session listener with latest-wins flags."""

    def __init__(self):
        self.cv = threading.Condition()
        self.graph = True          # send the initial state straight away
        self.status = True
        self.types = False

    def on_graph(self, rev, origin):
        with self.cv:
            self.graph = True
            self.cv.notify_all()

    def on_result(self, res):
        with self.cv:
            self.status = True
            self.cv.notify_all()

    def on_status(self, rev):
        self.on_result(None)

    def on_types(self):
        with self.cv:
            self.types = True
            self.cv.notify_all()

    def on_interface(self, iface):
        pass


class _HTTPServer(ThreadingHTTPServer):
    daemon_threads = True
    allow_reuse_address = os.name != "nt"     # on Windows SO_REUSEADDR lets another process share the port

    def server_bind(self):
        if os.name == "nt":
            self.socket.setsockopt(socket.SOL_SOCKET, socket.SO_EXCLUSIVEADDRUSE, 1)
        super().server_bind()


class WebServer:
    def __init__(self, session, host: str = "127.0.0.1", port: int = DEFAULT_HTTP_PORT, *, link=None,
                 root=None, bind_retry_seconds: float = 3.0):
        self.session, self.host, self.port, self.link = session, host, port, link
        self.root = Path(root or os.getcwd()).resolve()
        session.editor_root = self.root
        paths.set_root(self.root)                 # relative file parameters of nodes start here
        self.pick_file = None                     # the desktop window's native dialog: fn(mode, exts, title, start) -> {"path": ...} | None
        self.bind_retry_seconds = bind_retry_seconds
        self._httpd: Optional[_HTTPServer] = None
        self._thread: Optional[threading.Thread] = None
        self.stopping = threading.Event()
        self._listing_cache: dict = {}

    @property
    def url(self) -> str:
        host = "127.0.0.1" if self.host in ("0.0.0.0", "") else self.host
        return f"http://{host}:{self.port}/"

    def start(self) -> int:
        deadline = time.monotonic() + self.bind_retry_seconds
        handler = type("Handler", (_Handler,), {"app": self})
        while True:
            try:
                self._httpd = _HTTPServer((self.host, self.port), handler)
                break
            except OSError as e:
                busy = e.errno in (errno.EADDRINUSE, errno.EACCES) or getattr(e, "winerror", 0) in (10048, 10013)
                if not busy:
                    raise
                if time.monotonic() >= deadline:
                    raise OSError(e.errno, f"port {self.port} on {self.host} is already in use "
                                           f"(is another minihou engine running? use --http-port)") from None
                time.sleep(0.25)
        self.port = self._httpd.server_address[1]
        self._thread = threading.Thread(target=self._httpd.serve_forever, kwargs={"poll_interval": 0.25},
                                        name="minihou-http", daemon=True)
        self._thread.start()
        return self.port

    def stop(self) -> None:
        self.stopping.set()
        if self._httpd:
            self._httpd.shutdown()
            self._httpd.server_close()
        if self._thread:
            self._thread.join(2.0)

    # -- helpers used by the handler -----------------------------------------
    def link_clients(self) -> List[str]:
        if self.link is None:
            return []
        return [c.name for c in self.link.clients]

    def select_group(self, node):
        """Resolve component ids against the group's input, never the modified output."""
        from .. import groups, edges, proxy, timectx
        import numpy as np
        s = self.session
        if not isinstance(node, str):
            raise ApiError(400, '"node" must be a node id')
        with s.lock:
            graph = s.graph.copy()
            frame, fps = s.frame, s.fps
            feeds = s._modifier_feeds(graph)
        if node not in graph.nodes:
            raise ApiError(400, f'no such node {node!r}')
        inst = graph.nodes[node]
        nt = s.evaluator.registry.get(inst.type)
        params = graph.effective_params(node)
        if 'group' not in params:
            raise ApiError(400, 'node has no group parameter')
        source = inst.inputs.get(nt.inputs[0].name) if nt.inputs else None
        if not isinstance(source, str) or source not in graph.nodes:
            raise ApiError(400, 'group selection requires one connected first input')
        raw_domain = next((params[k] for k in ('group_type', 'domain', 'run_over') if k in params), None)
        aliases = {'point': 'point', 'points': 'point', 'prim': 'prim', 'primitives': 'prim',
                   'edge': 'edge', 'edges': 'edge'}
        if raw_domain is not None:
            if raw_domain not in aliases:
                raise ApiError(400, f'component selection does not support domain {raw_domain!r}')
            domain = aliases[raw_domain]
        elif inst.type in ('edge_collapse', 'edge_flip', 'edge_cusp', 'dissolve_edges'):
            domain = 'edge'
        else:
            point_types = {'transform', 'peak', 'noise', 'ray', 'instance',
                           'fillet', 'attribute_transfer', 'attribute_blur', 'goal_surface', 'goal_pin'}
            domain = 'point' if inst.type in point_types or nt.category == 'deform' else 'prim'
        try:
            with s._cook_lock, timectx.at(frame, fps), proxy.using(s.view_level()):
                geo = s.evaluator.cook(graph, source, feeds=feeds)
                mask = edges.mask(geo, params['group']) if domain == 'edge' else groups.mask(geo, domain, params['group'])
                total = (geo.npoints if domain == 'point' else geo.nprims) if mask is None else int(np.count_nonzero(mask))
                if total > 100000:
                    raise ApiError(400, 'group selection exceeds the 100000 component limit; narrow the group')
                ids = list(range(total)) if mask is None else np.flatnonzero(mask).tolist()
        except ApiError:
            raise
        except (ValueError, KeyError, RuntimeError) as e:
            raise ApiError(400, str(e)) from None
        return {'node': source, 'domain': domain, 'ids': ids}

    def blender_info(self) -> dict:
        found = blender_setup.list_blenders()
        return {"blenders": found, "addon": {"available": blender_setup.addon_source() is not None, "url": "/addon/minihou_link.zip",
                                              "name": "minihou_link.zip"},
                "clients": self.link_clients(), "link_port": getattr(self.link, "port", None)}

    def state(self) -> dict:
        s = self.session
        out = s.snapshot()
        from .. import menu
        out.update(status=s.status(), nodes=[nt.describe() for nt in s.evaluator.registry], library=s.library_status(),
                   menu={"groups": menu.groups(), "extras": menu.pseudo()},
                   ops=OP_NAMES, root=str(self.root), files=self.files(),
                   link={"port": getattr(self.link, "port", None), "clients": self.link_clients()},
                   engine={"pid": os.getpid(), "version": _version(), "python_nodes": _python_allowed(), "protocol": PROTOCOL_VERSION,
                           "capabilities": ["instances_v1", "scene_objects_v1"]})
        return out

    def files(self) -> List[str]:
        """Graph files (``*.json`` with a top-level ``"nodes"``) under the root, relative, '/'-separated."""
        out = []
        for dirpath, dirnames, filenames in os.walk(self.root):
            dirnames[:] = sorted(d for d in dirnames if d not in SKIP_DIRS and not d.startswith("."))
            for fn in sorted(filenames):
                if not fn.endswith(".json") or fn.startswith("."):
                    continue
                p = Path(dirpath, fn)
                try:
                    with open(p, encoding="utf-8") as f:
                        head = f.read(4096)
                except (OSError, UnicodeDecodeError):
                    continue
                if '"nodes"' in head:
                    out.append(p.relative_to(self.root).as_posix())
            if len(out) >= 500:
                break
        return out

    def library_listing(self) -> dict:
        """Every graph file under the root with what a browser needs: name, age, size, node count, doc, and whether it is a
        library node (a system) that the engine has loaded."""
        s = self.session
        lib = s.library
        systems = {}
        if lib is not None:
            for f in lib.files.values():
                if str(f.path).endswith(".json"):
                    systems[str(Path(f.path).resolve())] = f
        cur = str(s.path.resolve()) if s.path else None
        out = []
        for rel in self.files():
            p = (self.root / rel).resolve()
            try:
                st = p.stat()
            except OSError:
                continue
            key = (str(p), st.st_mtime_ns, st.st_size)
            meta = self._listing_cache.get(key)
            if meta is None:
                meta = {"nodes": 0, "doc": "", "exposed": 0}
                if st.st_size < 2_000_000:
                    try:
                        with open(p, encoding="utf-8") as fh:
                            data = json.load(fh)
                        meta = {"nodes": len(data.get("nodes") or {}), "doc": str(data.get("doc") or "")[:300],
                                "exposed": len(data.get("exposed") or [])}
                    except (OSError, ValueError, UnicodeDecodeError, AttributeError):
                        pass
                if len(self._listing_cache) > 2000:
                    self._listing_cache.clear()
                self._listing_cache[key] = meta
            f = systems.get(str(p))
            out.append({"path": rel, "name": p.stem, "mtime": st.st_mtime, "size": st.st_size, "current": str(p) == cur,
                        "system": f is not None, "type": (f.types[0] if f is not None and f.types else None),
                        "error": f.error if f is not None else None, **meta})
        folder = str(lib.dirs[0]) if lib is not None and lib.dirs else str(self.root / "nodes")
        return {"root": str(self.root), "files": out, "library_dir": folder, "current": cur}

    def library_dir(self) -> Path:
        lib = self.session.library
        return lib.dirs[0] if lib is not None and lib.dirs else (self.root / "nodes").resolve()

    def make_system(self, name: Any, ids: Any, replace: bool, doc: str, overwrite: bool) -> dict:
        """Save the selected nodes as a library node (a system graph in the node folder); optionally swap the selection for it."""
        from .. import subgraph
        from ..graph import GraphError
        from ..link.session import _atomic_write
        s = self.session
        if not isinstance(name, str) or not subgraph.NAME.match(name):
            raise ApiError(400, "name must be letters, digits and _ (not starting with a digit): it becomes the node's type name")
        if not isinstance(ids, list) or not all(isinstance(i, str) for i in ids) or not ids:
            raise ApiError(400, '"nodes" must list the ids of the nodes to save')
        folder = self.library_dir()
        target = folder / (name + ".json")
        reg = s.evaluator.registry
        if name in reg and not (overwrite and reg.get(name).source == str(target)):
            other = reg.get(name).source
            raise ApiError(409, f"a node type called {name!r} already exists ({'in ' + other if other else 'built in'})")
        if target.exists() and not overwrite:
            raise ApiError(409, f"{target.name} already exists in the node folder")
        with s.lock:
            graph = s.graph.copy()
        try:
            data, _ports = subgraph.extract(graph, ids, doc)
            ops = subgraph.replace_ops(graph, ids, name) if replace else None
        except GraphError as e:
            raise ApiError(400, str(e)) from None
        before = target.read_text(encoding="utf-8") if target.exists() else None
        from ..graph import Graph
        folder.mkdir(parents=True, exist_ok=True)
        _atomic_write(target, Graph.from_dict(data, reg, check=False).to_json())
        s._poll_library()
        if name not in reg or reg.get(name).source != str(target):         # the engine refused it: put things back and say why
            entry = s.library.files.get(target) if s.library is not None else None
            why = entry.error if entry is not None and entry.error else "the engine could not load it"
            if before is None:
                target.unlink(missing_ok=True)
            else:
                _atomic_write(target, before)
            s._poll_library()
            raise ApiError(400, f"cannot make a node from that selection: {why}")
        result = {"name": name, "path": str(target.relative_to(self.root)) if self.root in target.parents else str(target)}
        if ops is not None:
            try:
                r = s.edit(ops, origin="api")
            except Exception as e:  # noqa: BLE001
                raise ApiError(400, f"saved {target.name}, but replacing the selection failed: {e}") from None
            result.update(replaced=True, rev=r["rev"], node=r["results"][0].get("id"))
        return result

    def file_op(self, op: Any, path: Any, to: Any) -> dict:
        s = self.session
        p = self.resolve(path)
        cur = s.path.resolve() if s.path else None
        if not p.is_file():
            raise ApiError(404, f"no such graph file: {path}")
        if op == "duplicate":
            if to:
                dst = self.resolve(to)
            else:
                k, dst = 1, p.with_name(p.stem + "_copy.json")
                while dst.exists():
                    k += 1
                    dst = p.with_name(f"{p.stem}_copy{k}.json")
            if dst.exists():
                raise ApiError(409, f"{dst.name} already exists")
            dst.write_bytes(p.read_bytes())
            return {"path": dst.relative_to(self.root).as_posix()}
        if cur is not None and p == cur:
            raise ApiError(409, "that graph is open: open another one first (or use Save As)")
        if op == "rename":
            dst = self.resolve(to)
            if dst.exists():
                raise ApiError(409, f"{dst.name} already exists")
            dst.parent.mkdir(parents=True, exist_ok=True)
            p.rename(dst)
            return {"path": dst.relative_to(self.root).as_posix()}
        if op == "delete":
            p.unlink()
            return {"deleted": path}
        raise ApiError(400, 'op must be "rename", "duplicate" or "delete"')

    def resolve(self, path: Any, exts: tuple = (".json",)) -> Path:
        if not isinstance(path, str) or not path.strip():
            raise ApiError(400, "path must be a non-empty string")
        p = Path(path)
        p = (p if p.is_absolute() else self.root / p).resolve()
        if p.suffix.lower() not in exts:
            raise ApiError(400, ("graph files must end in .json" if exts == (".json",) else f"the file must end in {', '.join(exts)}"))
        if p != self.root and self.root not in p.parents:
            raise ApiError(403, f"path is outside the workspace root {self.root}")
        return p


def _python_allowed() -> bool:
    from ..nodes import python_node
    return python_node.allowed()


def _version() -> str:
    from .. import __version__
    return __version__


class _Handler(BaseHTTPRequestHandler):
    app: WebServer
    protocol_version = "HTTP/1.1"
    server_version = "minihou"

    def log_message(self, fmt, *args):          # quiet: requests are logged at debug level only
        log.debug("%s %s", self.address_string(), fmt % args)

    # -- plumbing ------------------------------------------------------------
    def _send(self, status: int, body: bytes, ctype: str, extra: Optional[dict] = None):
        self.send_response(status)
        self.send_header("Content-Type", ctype)
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Cache-Control", "no-store")
        self.send_header("X-Content-Type-Options", "nosniff")
        for k, v in (extra or {}).items():
            self.send_header(k, v)
        self.end_headers()
        if self.command != "HEAD":
            self.wfile.write(body)

    def _json(self, obj: Any, status: int = 200):
        extra = None
        if status >= 400:                       # the request body may be unread: don't reuse this connection
            self.close_connection = True
            extra = {"Connection": "close"}
            self._drain_body()
        self._send(status, json.dumps(obj, allow_nan=False).encode("utf-8"), "application/json", extra)

    def _drain_body(self) -> None:
        """Read and discard an unread request body before replying with an error. Closing a socket that still holds
        unread data makes Windows reset the connection, and the client can then lose the reply (WinError 10053)."""
        if getattr(self, "_body_read", False):
            return
        self._body_read = True
        try:
            n = int(self.headers.get("Content-Length") or 0)
            if 0 < n <= MAX_BODY:
                self.rfile.read(n)
        except (ValueError, OSError):
            pass

    def _host_ok(self) -> bool:
        if self.app.host not in ("127.0.0.1", "localhost", "::1"):
            return True                              # explicitly bound to a LAN address: the user opted in
        host = (self.headers.get("Host") or "").strip().lower()
        name = host.rsplit(":", 1)[0] if not host.startswith("[") else host.split("]")[0] + "]"
        return name in LOOPBACK

    def _origin_ok(self) -> bool:
        origin = self.headers.get("Origin")
        if origin is None:
            return True                              # non-browser clients (MCP server, curl)
        o = urlparse(origin)
        return o.scheme == "http" and (o.hostname or "") in ("127.0.0.1", "localhost", "::1") and o.port == self.app.port

    def _body(self) -> dict:
        ctype = (self.headers.get("Content-Type") or "").split(";")[0].strip().lower()
        if ctype != "application/json":
            raise ApiError(415, "POST bodies must be application/json")
        n = int(self.headers.get("Content-Length") or 0)
        if n > MAX_BODY:
            raise ApiError(413, "request too large")
        raw = self.rfile.read(n) if n else b"{}"
        self._body_read = True
        try:
            data = json.loads(raw.decode("utf-8"))
        except (ValueError, UnicodeDecodeError) as e:
            raise ApiError(400, f"invalid JSON: {e}") from None
        if not isinstance(data, dict):
            raise ApiError(400, "body must be a JSON object")
        return data

    def _guard(self, fn):
        self._body_read = False
        try:
            if not self._host_ok():
                raise ApiError(403, "bad Host header")
            fn()
        except ApiError as e:
            self._json({"error": e.message, **e.extra}, e.status)
        except EditError as e:
            self._json({"error": str(e), "index": e.index}, 400)
        except (GraphError, ParamError) as e:
            self._json({"error": str(e)}, 400)
        except (BrokenPipeError, ConnectionResetError, ConnectionAbortedError):
            pass
        except Exception as e:  # noqa: BLE001
            log.exception("request failed: %s %s", self.command, self.path)
            try:
                self._json({"error": f"internal error: {e!r}"}, 500)
            except OSError:
                pass

    # -- verbs -----------------------------------------------------------------
    def do_GET(self):
        self._guard(self._get)

    def do_HEAD(self):
        self._guard(self._get)

    def do_POST(self):
        self._guard(self._post)

    def _upload(self):
        """A dropped file: stream the request body to ``imports/<name>`` under the root. A page cannot send this cross-site (the custom
        header forces a CORS preflight, which is never granted), and the Origin was checked."""
        if self.headers.get("X-MiniHou-Upload") != "1":
            raise ApiError(403, "uploads need the X-MiniHou-Upload header")
        name = os.path.basename(parse_qs(urlparse(self.path).query).get("name", [""])[0].replace("\\", "/"))
        ext = os.path.splitext(name)[1].lower()
        if not name or name.startswith(".") or ext not in UPLOAD_EXTENSIONS:
            raise ApiError(400, f"a dropped file must be one of {', '.join(UPLOAD_EXTENSIONS)}")
        try:
            n = int(self.headers.get("Content-Length") or "")
        except ValueError:
            raise ApiError(411, "uploads need a Content-Length") from None
        if n <= 0 or n > MAX_UPLOAD:
            raise ApiError(413, f"a dropped file may be at most {MAX_UPLOAD // 2**30} GB")
        folder = self.app.root / "imports"
        folder.mkdir(parents=True, exist_ok=True)
        target = folder / name
        k = 1
        while target.exists():                                   # never replace a file that is already there
            target = folder / f"{os.path.splitext(name)[0]}_{k}{ext}"
            k += 1
        part = target.with_name(target.name + ".part")
        left = n
        try:
            with open(part, "wb") as f:
                while left:
                    chunk = self.rfile.read(min(1 << 20, left))
                    if not chunk:
                        raise ApiError(400, "the upload ended early")
                    f.write(chunk)
                    left -= len(chunk)
            os.replace(part, target)
        except BaseException:
            try:
                os.remove(part)
            except OSError:
                pass
            self.close_connection = True
            raise
        self._body_read = True
        self._json({"path": target.relative_to(self.app.root).as_posix(), "size": n})

    def _get(self):
        u = urlparse(self.path)
        path, q = u.path, parse_qs(u.query)
        app, s = self.app, self.app.session
        if path in ("/", "/index.html"):
            return self._static("index.html")
        if path.startswith("/static/"):
            return self._static(path[len("/static/"):])
        if path == "/api/state":
            return self._json(app.state())
        if path == '/api/preferences':
            from .. import user_preferences
            from ..nodes import python_node
            value = user_preferences.describe()
            value['python_nodes'] = python_node.allowed()
            return self._json(value)
        if path == "/api/nodes":
            return self._json([nt.describe() for nt in s.evaluator.registry])
        if path == "/api/graph":
            return self._json(s.snapshot())
        if path == "/api/status":
            if q.get("wait", ["0"])[0] not in ("0", ""):
                s.wait_idle(float(q.get("timeout", ["30"])[0]))
            st = s.status()
            st["clients"] = app.link_clients()
            return self._json(st)
        if path == "/api/blender":
            return self._json(app.blender_info())
        if path == "/addon/minihou_link.zip":
            try:
                data = blender_setup.build_addon_zip()
            except FileNotFoundError as e:
                raise ApiError(404, str(e)) from None
            return self._send(200, data, "application/zip", {"Content-Disposition": 'attachment; filename="minihou_link.zip"'})
        if path == "/api/files":
            return self._json({"root": str(app.root), "files": app.files(),
                               "current": str(s.path) if s.path else None})
        if path == "/api/library":
            return self._json(app.library_listing())
        if path == "/api/file":
            p = app.resolve(parse_qs(urlparse(self.path).query).get("path", [None])[0])
            if not p.is_file() or p.stat().st_size > 8_000_000:
                raise ApiError(404, f"no such graph file (or too large): {p.name}")
            try:
                return self._json({"path": p.relative_to(app.root).as_posix(), "graph": json.loads(p.read_text(encoding="utf-8"))})
            except (OSError, ValueError, UnicodeDecodeError) as e:
                raise ApiError(400, f"cannot read {p.name}: {e}") from None
        if path == "/api/browse":
            try:
                return self._json(filedialog.list_dir(q.get("path", [""])[0], filedialog.clean_exts(q.get("exts", [""])[0]), root=app.root,
                                                      hidden=q.get("hidden", ["0"])[0] not in ("0", ""), places=q.get("places", ["0"])[0] not in ("0", ""),
                                                      fallback=q.get("fallback", ["0"])[0] not in ("0", "")))
            except FileNotFoundError as e:
                raise ApiError(404, str(e)) from None
        if path == "/api/proxy":
            return self._json(self.app.session.proxy_info())
        if path == "/api/events":
            return self._events()
        raise ApiError(404, f"no such endpoint: {path}")

    def _post(self):
        if not self._origin_ok():
            raise ApiError(403, "cross-origin request refused")
        path = urlparse(self.path).path
        if path == "/api/upload":
            return self._upload()
        body = self._body()
        app, s = self.app, self.app.session
        origin = str(body.get("origin") or "api")[:32]
        if path == "/api/preferences":
            from .. import user_preferences
            from ..nodes import python_node
            values = {key: body[key] for key in ('python_nodes', 'copy_memory_limit_mib') if key in body}
            with s._cook_lock:
                try:
                    user_preferences.update(values)
                except ValueError as e:
                    raise ApiError(400, str(e)) from None
                if 'python_nodes' in values:
                    python_node.set_allowed(values['python_nodes'])
                s.evaluator.cache.clear()
            with s.lock:
                s._rev += 1
                s._wake.set()
            value = user_preferences.describe()
            value['python_nodes'] = python_node.allowed()
            return self._json(value)
        if path == "/api/cancel":
            s.cancel_cook()
            return self._json({"ok": True})
        if path == "/api/desktop/activate":
            activate = getattr(app, "desktop_activate", None)
            return self._json({"desktop": bool(activate and activate(body))})
        if path == "/api/edit":
            ops = body.get("ops")
            if not isinstance(ops, list):
                raise ApiError(400, '"ops" must be a list of edit operations')
            out = s.edit(ops, origin=origin, document_id=body.get("document_id"), gesture=body.get('gesture'))
            if body.get("wait"):
                s.wait_idle(float(body.get("timeout", 30)))
                out["status"] = s.status()
                s.flush()                                 # callers that wait may read the file next
            out["graph"] = s.snapshot()
            return self._json(out)
        if path in ('/api/gesture_cancel', '/api/gesture_finish'):
            out = s.gesture_finish(body.get('gesture'), origin=origin, document_id=body.get('document_id'),
                                   cancel=path == '/api/gesture_cancel')
            out['graph'] = s.snapshot()
            return self._json(out)
        if path in ("/api/undo", "/api/redo"):
            ok = (s.undo if path == "/api/undo" else s.redo)(origin=origin, document_id=body.get("document_id"))
            return self._json({"ok": ok, "graph": s.snapshot()})
        if path == "/api/inspect":
            node = body.get("node")
            if not isinstance(node, str):
                raise ApiError(400, '"node" must be a node id')
            return self._json(s.inspect(node, bool(body.get("detail"))))
        if path == "/api/select_group":
            return self._json(app.select_group(body.get('node')))
        if path == "/api/preview":
            node = body.get("node")
            if not isinstance(node, str):
                raise ApiError(400, '"node" must be a node id')
            imode = body.get("instances") if body.get("instances") in ("auto", "always", "never") else "auto"
            r = s.preview(node, templates=bool(body.get("templates")), instances=imode)
            if not r["ok"]:
                return self._json({"ok": False, "node": node, "rev": r["rev"], "error": r["error"]})
            from .. import impostors, preview
            full, budget = bool(body.get("full")), _budget(body.get("tris"))
            iset = None
            try:
                iset = impostors.plan(r["geo"], imode, full, budget, r.get("instance_sources"))
            except Exception:                                        # impostors are a nicety: never lose the preview over them
                log.exception("impostors failed for %s", node)
            if iset is not None and iset.replaces:
                head, arrays = impostors.shell(r["geo"])
            else:
                head, arrays = preview.arrays_for(r["geo"], full=full, tris=budget)
            head.update(node=node, rev=r["rev"], handles=r.get("handles", []), cook_level=s.view_level())
            if iset is not None:
                arrays = dict(arrays)
                head["instances"], extra = impostors.pack(iset)
                arrays.update(extra)
                if iset.replaces:
                    head["proxy"].update(instanced=True, instances=int(iset.total or len(iset.M)))
            if isinstance(body.get("attr"), dict) or isinstance(body.get("mask"), dict):
                arrays = dict(arrays)                                # the memoised arrays are shared: never add to them
                try:
                    if isinstance(body.get("attr"), dict):
                        preview.attr_arrays(r["geo"], body["attr"], head, arrays)
                    if isinstance(body.get("mask"), dict):
                        preview.mask_arrays(r["geo"], body["mask"], head, arrays)
                except ValueError as e:
                    raise ApiError(400, str(e)) from None
            if (isinstance(body.get("have"), str) and head.get("topology") == body["have"] and not head.get("instances")
                    and not (iset is not None and iset.replaces)):
                arrays = {k: v for k, v in arrays.items() if k not in ("tri", "tri_prim", "edge")}   # the viewer has these already
                head["same_topology"] = True
            head.update(selectable_templates=r.get("selectable", []), template_warnings=r.get("template_warnings", []))
            if r.get("reference") is not None:
                refhead, refs = preview.arrays_for(r["reference"], full=bool(body.get("full")), tris=_budget(body.get("tris")))
                for key in ("P", "edge", "tri"):
                    arrays["template_" + key] = refs[key]
                head["template_points"] = refhead["points"]
                if refhead["points"]:
                    if not head["points"]:
                        head["bounds"] = refhead["bounds"]
                    else:
                        head["bounds"] = [[min(a, b) for a, b in zip(head["bounds"][0], refhead["bounds"][0])],
                                          [max(a, b) for a, b in zip(head["bounds"][1], refhead["bounds"][1])]]
            return self._send(200, preview.pack(head, arrays), "application/octet-stream")
        if path == "/api/spreadsheet":
            node = body.get("node")
            if not isinstance(node, str):
                raise ApiError(400, '"node" must be a node id')
            r = s.preview(node)
            if not r["ok"]:
                return self._json({"ok": False, "node": node, "rev": r["rev"], "error": r["error"]})
            from .. import spreadsheet
            ids = body.get("ids")
            if ids is not None and (not isinstance(ids, list) or any(isinstance(i, bool) or not isinstance(i, int) for i in ids[:100000])):
                raise ApiError(400, '"ids" must be a list of element numbers')
            try:
                out = spreadsheet.page(r["geo"], str(body.get("domain") or "point"), int(body.get("start") or 0), int(body.get("count") or 100), ids,
                                       body.get("sort"), str(body.get("filter") or ""))
            except spreadsheet.FilterError as e:                          # a filter or sort the geometry cannot satisfy: shown in the table, not an HTTP error
                return self._json({"ok": False, "node": node, "rev": r["rev"], "error": {"node": node, "message": str(e)}})
            except (ValueError, TypeError) as e:
                raise ApiError(400, str(e)) from None
            if body.get("origins") and out.get("columns"):
                out["origins"] = s.column_origins(node, out["domain"], [c["name"] for c in out["columns"] if not c.get("computed")])
            out.update(ok=True, node=node, rev=r["rev"], cook_level=s.view_level())
            return self._json(out)
        if path == "/api/open":
            p = app.resolve(body.get("path"))
            if not p.exists() and not body.get("create"):
                raise ApiError(404, f"no such graph file: {p}")
            snap = s.open_file(p, create=bool(body.get("create")))
            return self._json(snap)
        if path == "/api/save_as":
            p = app.resolve(body.get("path"))
            try:
                return self._json(s.save_as(p, overwrite=bool(body.get("overwrite"))))
            except GraphError as e:
                raise ApiError(409, str(e)) from None
        if path == "/api/file_op":
            return self._json(app.file_op(body.get("op"), body.get("path"), body.get("to")))
        if path == "/api/make_system":
            return self._json(app.make_system(body.get("name"), body.get("nodes"), bool(body.get("replace")), str(body.get("doc") or ""),
                                              bool(body.get("overwrite"))))
        if path == "/api/save":
            return self._json({"saved": s.flush(), "error": s.save_error})
        if path in ("/api/blender/install", "/api/blender/launch"):
            found = blender_setup.list_blenders()
            idx = body.get("index")
            if not isinstance(idx, int) or isinstance(idx, bool) or not 0 <= idx < len(found):
                raise ApiError(400, '"index" must pick one of the Blenders listed by GET /api/blender')
            exe = found[idx]["path"]                  # never a path from the request: only what discovery found
            if path.endswith("install"):
                return self._json(blender_setup.install_addon(exe))
            port = getattr(app.link, "port", None)
            if not port:
                raise ApiError(409, "the live link is not running, so Blender would have nothing to connect to")
            try:
                blender_setup.launch_blender(exe, port)
            except (OSError, FileNotFoundError) as e:
                return self._json({"ok": False, "message": f"could not start Blender: {e}"})
            return self._json({"ok": True, "message": f"Starting Blender {found[idx]['version'] or ''}; it connects to this engine by itself."})
        if path == "/api/pick_file":
            mode = "save" if body.get("mode") == "save" else "open"
            exts = filedialog.clean_exts(body.get("exts"))
            start = body.get("start")
            start = paths.resolve(start) if isinstance(start, str) and start.strip() else str(app.root)
            title = str(body.get("title") or "")[:120]
            native = app.pick_file or filedialog.system_dialog
            res = native(mode, exts, title, start)
            if res is None:
                return self._json({"native": False})
            if res.get("busy"):
                raise ApiError(409, "a file dialog is already open")
            chosen = str(res.get("path") or "")
            return self._json({"native": True, "cancelled": not chosen, "path": paths.to_param(chosen) if chosen else ""})
        if path == "/api/node_action":
            node, name = body.get("node"), body.get("action")
            if not isinstance(node, str) or not isinstance(name, str):
                raise ApiError(400, '"node" and "action" must be strings')
            try:
                return self._json(actions.run(s, node, name))
            except FileExistsError as e:
                raise ApiError(409, str(e)) from None
            except usd_io.UsdUnavailable as e:
                raise ApiError(501, str(e)) from None
            except (GraphError, NodeError, ParamError, ValueError) as e:
                raise ApiError(400, str(e)) from None
            except OSError as e:
                raise ApiError(409, f"cannot write the file: {e.strerror or e}") from None
        if path == "/api/export_usd":
            p = app.resolve(body.get("path"), usd_io.USD_EXTENSIONS)
            node = body.get("node")
            if node is not None and not isinstance(node, str):
                raise ApiError(400, '"node" must be a node id')
            try:
                return self._json(s.export_usd(p, node=node, overwrite=bool(body.get("overwrite")), embed_graph=body.get("graph", True) is not False))
            except FileExistsError as e:
                raise ApiError(409, f"{e}; set overwrite to replace it") from None
            except usd_io.UsdUnavailable as e:
                raise ApiError(501, str(e)) from None
            except (GraphError, NodeError, ParamError, ValueError) as e:
                raise ApiError(400, str(e)) from None
            except OSError as e:                                  # e.g. the file is open in another program (Windows), or no permission
                raise ApiError(409, f"cannot write {p.name}: {e.strerror or e}") from None
        if path == "/api/disk_cache":
            enabled, clear = body.get("enabled"), bool(body.get("clear"))
            if enabled is not None and not isinstance(enabled, bool):
                raise ApiError(400, '"enabled" must be true or false')
            if enabled is None:
                stats = s.disk_cache.stats() if s.disk_cache is not None else None
                if clear and s.disk_cache is not None:
                    stats = s.set_disk_cache(s.disk_cache.folder, clear=True)
            elif enabled:
                stats = s.set_disk_cache(app.root / ".minihou_cache", clear=clear)
            else:
                stats = s.set_disk_cache(None, clear=clear)
            return self._json({"enabled": stats is not None, "stats": stats})
        if path == "/api/proxy":
            try:
                level = body.get("level")
                if level is not None and (isinstance(level, bool) or not isinstance(level, (int, float))):
                    raise ValueError('"level" must be a number between 0.001 and 1')
                auto = body.get("auto")
                if auto is not None and not isinstance(auto, bool):
                    raise ValueError('"auto" must be true or false')
                cap = body.get("link_tris")
                if cap is not None and (isinstance(cap, bool) or not isinstance(cap, int) or cap < 0):
                    raise ValueError('"link_tris" must be a whole number, 0 for no limit')
                return self._json(s.set_proxy(level, auto, cap, persist=True))
            except ValueError as e:
                raise ApiError(400, str(e)) from None
        if path == "/api/frame":
            try:
                frame = float(body.get("frame"))
                fps = float(body["fps"]) if body.get("fps") is not None else None
            except (TypeError, ValueError):
                raise ApiError(400, '"frame" must be a number') from None
            if not (abs(frame) < 1e7) or (fps is not None and not (0 < fps < 1000)):
                raise ApiError(400, "frame or fps out of range")
            s.set_frame(frame, fps)
            return self._json({"frame": s.frame, "fps": s.fps})
        raise ApiError(404, f"no such endpoint: {path}")

    def _static(self, name: str):
        if "/" in name or "\\" in name or name.startswith(".") or not name:
            raise ApiError(404, "not found")
        p = STATIC_DIR / name
        if not p.is_file():
            raise ApiError(404, "not found")
        self._send(200, p.read_bytes(), CONTENT_TYPES.get(p.suffix, "application/octet-stream"))

    def _events(self):
        stream = _EventStream()
        s, app = self.app.session, self.app
        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream; charset=utf-8")
        self.send_header("Cache-Control", "no-store")
        self.send_header("Connection", "keep-alive")
        self.end_headers()
        self.close_connection = True
        s.add_listener(stream)
        clients = None
        try:
            self.wfile.write(b"retry: 1000\n\n")
            self.wfile.flush()
            while not app.stopping.is_set():
                with stream.cv:
                    if not (stream.graph or stream.status or stream.types):
                        stream.cv.wait(2.0)
                    send_types, stream.types = stream.types, False
                    send_graph, stream.graph = stream.graph, False
                    send_status, stream.status = stream.status, False
                now_clients = app.link_clients()
                if now_clients != clients:
                    clients, send_status = now_clients, True
                chunks = []
                if send_types:                            # before the graph: it may use the new node types
                    chunks.append(("nodes", {"nodes": [nt.describe() for nt in s.evaluator.registry],
                                             "library": s.library_status()}))
                if send_graph:
                    chunks.append(("graph", s.snapshot()))
                if send_status:
                    st = s.status()
                    st["clients"] = clients
                    chunks.append(("status", st))
                if not chunks:
                    self.wfile.write(b": keepalive\n\n")
                for name, obj in chunks:
                    self.wfile.write(f"event: {name}\ndata: {json.dumps(obj, allow_nan=False)}\n\n".encode("utf-8"))
                self.wfile.flush()
        except (OSError, ValueError):
            pass
        finally:
            s.remove_listener(stream)
