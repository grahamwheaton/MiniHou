"""Threaded link client. Standard library + numpy only; shared verbatim with the Blender addon.

The client owns background reader/writer threads that connect (retrying), say hello, and read
frames forever. It **never** touches Blender: every received message goes to the
``on_message`` callback *on the client thread*, and the addon's callback just puts it on a
queue that a ``bpy.app.timers`` function drains on the main thread.
"""
from __future__ import annotations

import queue
import select
import socket
import threading
import time
from typing import Callable, Dict, Optional

import numpy as np

from .protocol import DEFAULT_PORT, PROTOCOL_VERSION, ProtocolError, recv_message, send_message
from .shm import ShmReader, unpack_shm_payload

MessageCallback = Callable[[dict, Dict[str, np.ndarray]], None]
StateCallback = Callable[[str, str], None]


class LinkClient:
    DISCONNECTED, CONNECTING, CONNECTED = "disconnected", "connecting", "connected"

    def __init__(self, host: str = "127.0.0.1", port: int = DEFAULT_PORT, *,
                 on_message: Optional[MessageCallback] = None, on_state: Optional[StateCallback] = None,
                 client_name: str = "client", reconnect: bool = True, retry_seconds: float = 1.0):
        self.host, self.port = host, port
        self.client_name = client_name
        self.reconnect = reconnect
        self.retry_seconds = retry_seconds
        self.on_message = on_message or (lambda header, arrays: None)
        self.on_state = on_state or (lambda state, detail: None)
        self.state = self.DISCONNECTED
        self.last_error = ""
        self._sock: Optional[socket.socket] = None
        self._outgoing = None
        self._writer = None
        self._stop = threading.Event()
        self._thread: Optional[threading.Thread] = None
        self._shm_reader = ShmReader()

    # -- lifecycle ---------------------------------------------------------
    def start(self) -> None:
        if self._thread and self._thread.is_alive():
            return
        self._stop.clear()
        self._thread = threading.Thread(target=self._run, name="minihou-link-client", daemon=True)
        self._thread.start()

    @staticmethod
    def _close_socket(sock):
        if sock is not None:
            try:
                sock.shutdown(socket.SHUT_RDWR)
            except OSError:
                pass
            sock.close()

    def stop(self, timeout: float = 0.0) -> None:
        # Blender teardown must never wait for a connect/recv/send or an SHM lock.
        self._stop.set()
        self._outgoing = None
        self._close_socket(self._sock)
        if (timeout and self._thread and self._thread is not threading.current_thread()
                and threading.current_thread() is not threading.main_thread()):
            self._thread.join(min(timeout, 0.3))
        self._set_state(self.DISCONNECTED, "stopped")

    # -- sending -----------------------------------------------------------
    def send(self, header: dict, arrays: Optional[dict] = None) -> bool:
        """Enqueue only: never encode or perform socket I/O on Blender's main thread.

        True means accepted for delivery. The bounded, connection-local queue is
        discarded on disconnect; callers must not mutate submitted array buffers.
        """
        outgoing = self._outgoing
        if outgoing is None or self.state != self.CONNECTED or self._stop.is_set():
            return False
        try:
            outgoing.put_nowait((dict(header), dict(arrays) if arrays else None))
            return True
        except queue.Full:
            # Dropping a parameter/ACK silently would leave the peers out of sync.
            self.last_error = "link send queue full"
            self._close_socket(self._sock)
            return False

    def _write(self, sock, outgoing, ended):
        try:
            while not self._stop.is_set() and not ended.is_set():
                try:
                    header, arrays = outgoing.get(timeout=0.05)
                except queue.Empty:
                    continue
                send_message(sock, header, arrays, deadline=time.monotonic() + 2.0)
        except (OSError, ProtocolError, ValueError) as exc:
            self.last_error = str(exc)
            self._close_socket(sock)  # Wake the reader, which owns reconnect/state.

    def set_param(self, name: str, value) -> bool:
        return self.send({"type": "set_param", "name": name, "value": value})

    def resync(self) -> bool:
        return self.send({"type": "resync"})

    # -- thread ------------------------------------------------------------
    def _set_state(self, state: str, detail: str = "") -> None:
        if self._stop.is_set() and state != self.DISCONNECTED:
            return
        changed = state != self.state
        self.state = state
        if changed:
            try:
                self.on_state(state, detail)
            except Exception:  # noqa: BLE001 - a bad callback must not kill the link thread
                pass

    def _run(self) -> None:
        while not self._stop.is_set():
            self._set_state(self.CONNECTING, f"{self.host}:{self.port}")
            sock = None
            writer = None
            ended = threading.Event()
            try:
                sock = socket.create_connection((self.host, self.port), timeout=0.2)
                if self._stop.is_set():
                    break
                sock.settimeout(0.1)
                sock.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
                self._sock = sock
                send_message(sock, {"type": "hello", "protocol": PROTOCOL_VERSION, "client": self.client_name,
                                    "capabilities": ["shm_v1", "viewport_materials_v1", "image_assets_v1"]}, deadline=time.monotonic() + 2.0)
                outgoing = queue.Queue(maxsize=64)
                self._outgoing = outgoing
                writer = threading.Thread(target=self._write, args=(sock, outgoing, ended),
                                          name="minihou-link-writer", daemon=True)
                self._writer = writer
                writer.start()
                self._set_state(self.CONNECTED, "")
                while not self._stop.is_set():
                    if not select.select([sock], [], [], 0.1)[0]:
                        continue  # Idle links are valid; a partial frame has a total deadline.
                    header, arrays = recv_message(sock, deadline=time.monotonic() + 2.0)
                    if header.get("type") == "bye":
                        raise ConnectionError("engine closed")
                    if header.get("transport") == "shm":
                        arrays = unpack_shm_payload(header, self._shm_reader)
                    try:
                        self.on_message(header, arrays)
                    except Exception as e:  # noqa: BLE001
                        self.last_error = f"on_message failed: {e!r}"
            except (OSError, ProtocolError, ValueError) as e:
                if not self._stop.is_set():
                    self.last_error = str(e)
            finally:
                self._outgoing = None
                ended.set()
                self._close_socket(sock)
                if writer is not None:
                    writer.join(0.3)  # Worker thread only; never Blender's timer.
                self._sock = None
                self._shm_reader.close()  # Same owner as read(); no UI-thread lock wait.
                self._set_state(self.DISCONNECTED, self.last_error)
            if self._stop.is_set() or not self.reconnect:
                break
            self._set_state(self.DISCONNECTED, self.last_error)
            self._stop.wait(self.retry_seconds)
        self._set_state(self.DISCONNECTED, "stopped")
