"""TCP server for the live link (loopback by default).

Per client: a reader thread (handles ``set_param`` etc.) and a writer thread. The writer
sends from a "latest wins" slot: if the client is slow, intermediate results are skipped and
the next message is diffed against what the client *actually* received (see ``DiffState``).

The protocol is unauthenticated; bind to loopback only unless you know what you are doing.
"""
from __future__ import annotations

import errno
import logging
import os
import socket
import threading
import time
from typing import Dict, List, Optional

from ..graph import GraphError
from ..sdk import ParamError
from .diff import DiffState
from ..geometry import Geometry
from .protocol import (DEFAULT_PORT, PROTOCOL_VERSION, TOPO_KEYS, TOPO_PRIM_CLOSED, TOPO_PRIM_OFFSETS, TOPO_VERTEX_POINT,
                       ProtocolError,
                       attr_key, recv_message, send_message, split_key)
from .session import Result, Session
from .instances import InstanceManager

log = logging.getLogger("minihou.server")


class _Connection:
    def __init__(self, server: "LinkServer", sock: socket.socket, addr):
        self.server, self.session, self.sock, self.addr = server, server.session, sock, addr
        self.diff = DiffState()
        self.instances = InstanceManager(self.session)
        self.instance_mode = False

        self.extra_diffs: Dict[str, DiffState] = {}      # one per extra output (named Blender objects)
        self._extra_keys: Dict[str, str] = {}
        self._sent_names: Optional[List[str]] = None
        self.cv = threading.Condition()
        self.closed = False
        self._outbox: List[tuple] = []
        self._want_result = False
        self._iface_pending = False
        self._sent_good_seq = 0
        self._sent_error_seq = 0
        self._scene_keys: Dict[str, str] = {}              # scene_objects node -> key of the manifest this client has
        self._scene_nodes: Optional[List[str]] = None
        self.name = "?"
        self.bytes_sent = 0
        self.threads = [threading.Thread(target=self._read_loop, name=f"minihou-r-{addr[1]}", daemon=True),
                        threading.Thread(target=self._write_loop, name=f"minihou-w-{addr[1]}", daemon=True)]

    def start(self):
        for t in self.threads:
            t.start()

    # -- session listener interface (called on the cook thread) --------------
    def on_result(self, res: Result):
        with self.cv:
            self._want_result = True
            self.cv.notify_all()

    def on_interface(self, iface: dict):
        with self.cv:
            self._iface_pending = True
            self.cv.notify_all()

    def on_extra_error(self, name: str, message: str):
        self.post({"type": "error", "where": "output", "node": None, "message": f"output {name!r}: {message}"})

    def on_watch(self, names, images=()):
        self.post({"type": "watch_objects", "objects": list(names), "images": list(images)})

    # -- helpers -----------------------------------------------------------
    def post(self, header: dict, arrays=None):
        with self.cv:
            self._outbox.append((header, arrays))
            self.cv.notify_all()

    def close(self):
        with self.cv:
            if self.closed:
                return
            self.closed = True
            self.cv.notify_all()
        self.session.remove_listener(self)
        self.instances.clear()
        try:
            self.sock.shutdown(socket.SHUT_RDWR)
        except OSError:
            pass
        try:
            self.sock.close()
        except OSError:
            pass
        self.server._forget(self)

    # -- reader ------------------------------------------------------------
    def _read_loop(self):
        try:
            while not self.closed:
                header, arrays = recv_message(self.sock)
                self._handle(header, arrays)
        except (OSError, ConnectionError, ProtocolError) as e:
            if not self.closed:
                log.info("client %s disconnected (%s)", self.addr, e)
        finally:
            self.close()

    def _handle(self, h: dict, _arrays=None):
        t = h.get("type")
        if t == "hello":
            if h.get("protocol") != PROTOCOL_VERSION:
                self.post({"type": "error", "message": f"protocol mismatch: server speaks {PROTOCOL_VERSION}, "
                                                       f"client sent {h.get('protocol')!r}"})
                return
            self.name = str(h.get("client", "?"))
            log.info("client %s connected (%s)", self.addr, self.name)
            self.instance_mode = h.get("mode") == "instances"
            self.post({"type": "welcome", "protocol": PROTOCOL_VERSION, "server": "minihou",
                       "capabilities": ["instances_v1", "live_modifier_v1", "object_inputs_v1", "scene_objects_v1", "image_exchange_v1"], "editor_url": self.server.editor_url,
                       "graph_path": str(self.session.path.resolve()) if self.session.path else None})
            if self.instance_mode:
                return
            self.post({"type": "watch_objects", "objects": self.session.watched_objects(), "images": self.session.watched_images()})
            with self.cv:
                self._iface_pending = True
                self._want_result = True
                self.session.add_listener(self)
                self.cv.notify_all()
        elif t in ("instance_cook", "instance_describe", "instance_remove", "instance_edit", "instance_sync", "instance_sources"):
            try:
                if not self.instance_mode:
                    raise ValueError("instance requests require hello mode instances")
                if t == "instance_remove":
                    self.instances.remove(h["instance_id"])
                    response, output = {"type": "instance_removed"}, None
                elif t == "instance_sources":
                    names = h.get("sources", [])
                    if not isinstance(names, list) or len(names) > 64 or any(not isinstance(n, str) or len(n) > 128 for n in names):
                        raise ValueError("invalid source inputs")
                    sources = {name: _decode_object({}, {k.split("/", 1)[1]: v for k, v in (_arrays or {}).items()
                                                       if k.startswith(str(i) + "/")}) for i, name in enumerate(names)}
                    response, output = self.instances.set_sources(h.get("instance_id"), sources), None
                elif t == "instance_edit":
                    response, output = self.instances.edit_live(h, _decode_object(h, _arrays or {})), None
                elif t == "instance_sync":
                    response, output = self.instances.sync_live(h), None
                elif t == "instance_describe":
                    response, output = self.instances.describe(h), None
                else:
                    response, output = self.instances.cook(h, _decode_object(h, _arrays or {}))
                response.update(request_id=h.get("request_id"), instance_id=h.get("instance_id"))
                self.post(response, output)
            except Exception as e:
                self.post({"type": "instance_error", "request_id": h.get("request_id"),
                           "instance_id": h.get("instance_id"), "message": str(e)})
        elif t == "set_param":
            if self.session.live_modifier is not None:
                self.post({"type": "error", "message": "Use the selected modifier controls or live editor"})
                return
            try:
                if "name" in h:
                    self.session.set_param(h["name"], h["value"])
                else:
                    self.session.set_node_param(h["node"], h["param"], h["value"])
            except (GraphError, ParamError, KeyError) as e:
                self.post({"type": "error", "where": "set_param", "message": str(e).strip("'")})
        elif t == "resync":
            self.diff.reset()
            self.extra_diffs.clear()
            self._extra_keys.clear()
            self._scene_keys.clear()
            self._scene_nodes = None
            self._sent_good_seq = 0
            with self.cv:
                self._iface_pending = True
                self._want_result = True
                self.cv.notify_all()
        elif t == "object_geo":
            name = str(h.get("object", ""))
            try:
                geo = _decode_object(h, _arrays)
            except Exception as e:  # noqa: BLE001 - bad data from a client must not kill the connection
                self.session.object_received(name, None, error=f"could not read the mesh: {e}")
                self.post({"type": "error", "where": "object_geo", "message": f"{name}: {e}"})
                return
            self.session.object_received(name, geo, h.get("matrix"), info=h.get("info") or {})
        elif t == "image_data":
            name = str(h.get("image", ""))
            try:
                pix = _arrays["pixels"].reshape(int(h["height"]), int(h["width"]), 4)
            except Exception as e:  # noqa: BLE001 - bad data from a client must not kill the connection
                self.session.image_received(name, None, error=f"could not read the pixels: {e}")
                self.post({"type": "error", "where": "image_data", "message": f"{name}: {e}"})
                return
            self.session.image_received(name, pix, {k: h[k] for k in ("is_float", "colorspace", "source") if k in h})
        elif t == "image_missing":
            self.session.image_received(str(h.get("image", "")), None, error=str(h.get("reason", "no such image")))
        elif t == "set_frame":
            try:
                self.session.set_frame(float(h.get("frame", 1.0)), h.get("fps"))
            except (TypeError, ValueError):
                self.post({"type": "error", "where": "set_frame", "message": f"bad frame {h.get('frame')!r}"})
        elif t == "object_missing":
            self.session.object_received(str(h.get("object", "")), None, error=str(h.get("reason", "not found")))
        elif t == "ping":
            self.post({"type": "pong", "t": h.get("t")})
        else:
            self.post({"type": "error", "message": f"unknown message type {t!r}"})

    # -- writer ------------------------------------------------------------
    def _write_loop(self):
        try:
            while True:
                with self.cv:
                    while not (self._outbox or self._want_result or self._iface_pending or self.closed):
                        self.cv.wait(1.0)
                    if self.closed:
                        return
                    outbox, self._outbox = self._outbox, []
                    want, self._want_result = self._want_result, False
                    iface, self._iface_pending = self._iface_pending, False
                for header, arrays in outbox:
                    self.bytes_sent += send_message(self.sock, header, arrays)
                if iface and self.session.live_modifier is None:
                    self.bytes_sent += send_message(self.sock, self.session.interface())
                if want:
                    self._send_results()
        except (OSError, ConnectionError) as e:
            if not self.closed:
                log.info("client %s write failed (%s)", self.addr, e)
        finally:
            self.close()

    def _send_scene(self, good):
        """The ``scene_objects`` manifests (real Blender objects), each only when it changed for this client."""
        scene = good.scene
        if scene is None:                                    # this pass did not produce them (e.g. the quick pass of auto proxy)
            return
        nodes = sorted(scene)
        if nodes != self._scene_nodes:
            self._scene_nodes = nodes
            self.bytes_sent += send_message(self.sock, {"type": "scene_nodes", "nodes": nodes})
            self._scene_keys = {n: k for n, k in self._scene_keys.items() if n in scene}
        for nid, manifest in scene.items():
            if self._scene_keys.get(nid) != manifest["key"]:
                self._scene_keys[nid] = manifest["key"]
                self.bytes_sent += send_message(self.sock, manifest, good.scene_arrays.get(nid) or None)

    def _send_results(self):
        if self.session.live_modifier is not None:
            return  # Never publish a live modifier back into the legacy proxy.
        good, err = self.session.latest_good, self.session.latest_error
        if good is not None and good.seq > self._sent_good_seq:
            header, arrays = self.diff.build(self.session.link_geometry(good), good.seq, good.cook)
            self._sent_good_seq = good.seq
            self.bytes_sent += send_message(self.sock, header, arrays)
            self._send_scene(good)
            names = sorted(good.extras)
            if names != self._sent_names:                    # which extra outputs exist (the add-on removes the others)
                self._sent_names = names
                self.bytes_sent += send_message(self.sock, {"type": "outputs", "names": names})
                for gone in set(self.extra_diffs) - set(names):
                    self.extra_diffs.pop(gone, None)
                    self._extra_keys.pop(gone, None)
            for name in names:
                geo, key = good.extras[name]
                if self._extra_keys.get(name) == key:
                    continue
                self._extra_keys[name] = key
                header, arrays = self.extra_diffs.setdefault(name, DiffState()).build(geo, good.seq, good.cook, output=name)
                self.bytes_sent += send_message(self.sock, header, arrays)
        if err is not None and err.seq > self._sent_error_seq:
            self._sent_error_seq = err.seq
            self.bytes_sent += send_message(self.sock, {"type": "error", "where": "cook", "seq": err.seq,
                                                        "node": err.error.get("node"),
                                                        "message": err.error["message"]})


def _decode_object(h: dict, arrays) -> Geometry:
    """A mesh sent by the add-on (``object_geo``): positions, topology and attributes keyed like ``geo`` messages."""
    P = arrays[attr_key("point", "P")].reshape(-1, 3)
    closed = arrays.get(TOPO_PRIM_CLOSED)
    geo = Geometry.from_arrays(P, arrays.get(TOPO_VERTEX_POINT), arrays.get(TOPO_PRIM_OFFSETS),
                               None if closed is None or closed.size == 0 else closed.astype(bool))
    for key, a in arrays.items():
        if key in TOPO_KEYS or key == attr_key("point", "P"):
            continue
        dom, name = split_key(key)
        if dom not in ("point", "vertex", "prim") or not name:
            raise ProtocolError(f"bad attribute key {key!r}")
        geo.table(dom)[name] = a
    mats = h.get("materials")                                  # slot names of the Blender object (see minihou.materials)
    if isinstance(mats, list) and mats:
        geo.detail["materials"] = [str(m)[:63] for m in mats[:4096]]
    geo.validate()
    return geo


class LinkServer:
    def __init__(self, session: Session, host: str = "127.0.0.1", port: int = DEFAULT_PORT,
                 bind_retry_seconds: float = 5.0):
        self.session, self.host, self.port = session, host, port
        self.bind_retry_seconds = bind_retry_seconds
        self.editor_url = None
        self._listener: Optional[socket.socket] = None
        self._conns: List[_Connection] = []
        self._lock = threading.Lock()
        self._thread: Optional[threading.Thread] = None
        self._stopping = False

    @property
    def clients(self) -> List[_Connection]:
        with self._lock:
            return list(self._conns)

    def start(self) -> int:
        """Bind and start accepting. Retries briefly if the port is still held (restarting the engine
        right after stopping it); raises ``OSError`` with a readable message if it stays busy."""
        deadline = time.monotonic() + self.bind_retry_seconds
        while True:
            s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
            if os.name == "nt":
                # SO_REUSEADDR on Windows lets a second process steal the port; ask for exclusivity.
                s.setsockopt(socket.SOL_SOCKET, socket.SO_EXCLUSIVEADDRUSE, 1)
            else:
                s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
            try:
                s.bind((self.host, self.port))
                break
            except OSError as e:
                s.close()
                busy = e.errno in (errno.EADDRINUSE, errno.EACCES) or getattr(e, "winerror", 0) in (10048, 10013)
                if not busy:
                    raise
                if time.monotonic() >= deadline:
                    raise OSError(e.errno, f"port {self.port} on {self.host} is already in use "
                                           f"(is another minihou engine running?)") from None
                time.sleep(0.25)
        s.listen(8)
        s.settimeout(0.5)
        self._listener = s
        self.port = s.getsockname()[1]
        self._thread = threading.Thread(target=self._accept_loop, name="minihou-accept", daemon=True)
        self._thread.start()
        return self.port

    def _accept_loop(self):
        while not self._stopping:
            try:
                sock, addr = self._listener.accept()
            except socket.timeout:
                continue
            except OSError:
                break
            sock.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
            sock.settimeout(None)
            c = _Connection(self, sock, addr)
            with self._lock:
                self._conns.append(c)
            c.start()

    def _forget(self, c: _Connection):
        with self._lock:
            if c in self._conns:
                self._conns.remove(c)

    def stop(self):
        self._stopping = True
        if self._listener:
            try:
                self._listener.close()
            except OSError:
                pass
        for c in self.clients:
            c.close()
        if self._thread:
            self._thread.join(2.0)
