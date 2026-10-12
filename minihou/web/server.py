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
``POST /api/undo``     also ``/api/redo``
``POST /api/inspect``  ``{node, detail?}`` -> geometry summary of any node
``POST /api/preview``  ``{node}`` -> positions, triangles and edges of any node for the 3-D view (binary, see
                       :mod:`minihou.preview`), or JSON ``{ok: false, error}``
``POST /api/open``     ``{path, create?}`` switch to (or create) a graph file inside the workspace root
``GET  /api/blender``  installed Blenders (by index) and the add-on zip, for the editor's Blender panel
``GET  /addon/minihou_link.zip``  the Blender add-on, ready to drag into Blender or install from disk
``POST /api/blender/install``  ``{index}`` install + enable the add-on in that Blender (runs it in the background once)
``POST /api/blender/launch``   ``{index}`` open that Blender with the add-on connected to this engine
``POST /api/frame``    ``{frame, fps?}`` set the current frame for @Frame / @Time (Blender's timeline also sets it)
=====================  =======================================================================================

Hardening, since any web page can try to talk to localhost: the ``Host`` header must be a loopback name (blocks DNS
rebinding), POSTs must be ``application/json`` (cross-site forms can't send that without a CORS preflight, which is
never granted) and a present ``Origin`` must be this server. File access is limited to ``.json`` under the root.
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

from .. import blender_setup
from ..edit import OP_NAMES, EditError
from ..graph import GraphError
from ..link.protocol import PROTOCOL_VERSION
from ..sdk import ParamError

log = logging.getLogger("minihou.web")

DEFAULT_HTTP_PORT = 8766
STATIC_DIR = Path(__file__).with_name("static")
MAX_BODY = 16 * 1024 * 1024
SKIP_DIRS = {".git", "__pycache__", "node_modules", ".venv", "venv", "gui_test_output", ".pytest_cache"}
CONTENT_TYPES = {".html": "text/html; charset=utf-8", ".js": "text/javascript; charset=utf-8",
                 ".css": "text/css; charset=utf-8", ".svg": "image/svg+xml", ".png": "image/png",
                 ".ico": "image/x-icon", ".json": "application/json"}
LOOPBACK = ("127.0.0.1", "localhost", "[::1]", "::1")


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
        self.bind_retry_seconds = bind_retry_seconds
        self._httpd: Optional[_HTTPServer] = None
        self._thread: Optional[threading.Thread] = None
        self.stopping = threading.Event()

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

    def blender_info(self) -> dict:
        found = blender_setup.list_blenders()
        return {"blenders": found, "addon": {"available": blender_setup.addon_source() is not None, "url": "/addon/minihou_link.zip",
                                              "name": "minihou_link.zip"},
                "clients": self.link_clients(), "link_port": getattr(self.link, "port", None)}

    def state(self) -> dict:
        s = self.session
        out = s.snapshot()
        out.update(status=s.status(), nodes=[nt.describe() for nt in s.evaluator.registry], library=s.library_status(),
                   ops=OP_NAMES, root=str(self.root), files=self.files(),
                   link={"port": getattr(self.link, "port", None), "clients": self.link_clients()},
                   engine={"pid": os.getpid(), "version": _version(), "python_nodes": _python_allowed(), "protocol": PROTOCOL_VERSION,
                           "capabilities": ["instances_v1"]})
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

    def resolve(self, path: Any) -> Path:
        if not isinstance(path, str) or not path.strip():
            raise ApiError(400, "path must be a non-empty string")
        p = Path(path)
        p = (p if p.is_absolute() else self.root / p).resolve()
        if p.suffix.lower() != ".json":
            raise ApiError(400, "graph files must end in .json")
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
        if path == "/api/events":
            return self._events()
        raise ApiError(404, f"no such endpoint: {path}")

    def _post(self):
        if not self._origin_ok():
            raise ApiError(403, "cross-origin request refused")
        path = urlparse(self.path).path
        body = self._body()
        app, s = self.app, self.app.session
        origin = str(body.get("origin") or "api")[:32]
        if path == "/api/desktop/activate":
            activate = getattr(app, "desktop_activate", None)
            return self._json({"desktop": bool(activate and activate(body))})
        if path == "/api/edit":
            ops = body.get("ops")
            if not isinstance(ops, list):
                raise ApiError(400, '"ops" must be a list of edit operations')
            out = s.edit(ops, origin=origin, document_id=body.get("document_id"))
            if body.get("wait"):
                s.wait_idle(float(body.get("timeout", 30)))
                out["status"] = s.status()
                s.flush()                                 # callers that wait may read the file next
            out["graph"] = s.snapshot()
            return self._json(out)
        if path in ("/api/undo", "/api/redo"):
            ok = (s.undo if path == "/api/undo" else s.redo)(origin=origin, document_id=body.get("document_id"))
            return self._json({"ok": ok, "graph": s.snapshot()})
        if path == "/api/inspect":
            node = body.get("node")
            if not isinstance(node, str):
                raise ApiError(400, '"node" must be a node id')
            return self._json(s.inspect(node, bool(body.get("detail"))))
        if path == "/api/preview":
            node = body.get("node")
            if not isinstance(node, str):
                raise ApiError(400, '"node" must be a node id')
            r = s.preview(node)
            if not r["ok"]:
                return self._json({"ok": False, "node": node, "rev": r["rev"], "error": r["error"]})
            from .. import preview
            head, arrays = preview.arrays_for(r["geo"])
            head.update(node=node, rev=r["rev"], handles=r.get("handles", []))
            return self._send(200, preview.pack(head, arrays), "application/octet-stream")
        if path == "/api/open":
            p = app.resolve(body.get("path"))
            if not p.exists() and not body.get("create"):
                raise ApiError(404, f"no such graph file: {p}")
            snap = s.open_file(p, create=bool(body.get("create")))
            return self._json(snap)
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
