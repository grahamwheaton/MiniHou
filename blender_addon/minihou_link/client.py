"""Threaded link client. Standard library + numpy only; shared verbatim with the Blender addon.

The client owns one background thread that connects (retrying), says hello, and then reads
frames forever. It **never** touches Blender: every received message goes to the
``on_message`` callback *on the client thread*, and the addon's callback just puts it on a
queue that a ``bpy.app.timers`` function drains on the main thread.
"""
from __future__ import annotations

import socket
import threading
import time
from typing import Callable, Dict, Optional

import numpy as np

from .protocol import DEFAULT_PORT, PROTOCOL_VERSION, ProtocolError, recv_message, send_message

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
        self._send_lock = threading.Lock()
        self._stop = threading.Event()
        self._thread: Optional[threading.Thread] = None

    # -- lifecycle ---------------------------------------------------------
    def start(self) -> None:
        if self._thread and self._thread.is_alive():
            return
        self._stop.clear()
        self._thread = threading.Thread(target=self._run, name="minihou-link-client", daemon=True)
        self._thread.start()

    def stop(self, timeout: float = 2.0) -> None:
        self._stop.set()
        s = self._sock
        if s is not None:
            try:
                s.shutdown(socket.SHUT_RDWR)
            except OSError:
                pass
            try:
                s.close()
            except OSError:
                pass
        if self._thread and self._thread is not threading.current_thread():
            self._thread.join(timeout)
        self._set_state(self.DISCONNECTED, "stopped")

    # -- sending -----------------------------------------------------------
    def send(self, header: dict, arrays: Optional[dict] = None) -> bool:
        """Thread-safe. Returns False if not connected (the message is dropped)."""
        s = self._sock
        if s is None or self.state != self.CONNECTED:
            return False
        try:
            with self._send_lock:
                send_message(s, header, arrays)
            return True
        except OSError as e:
            self.last_error = str(e)
            return False

    def set_param(self, name: str, value) -> bool:
        return self.send({"type": "set_param", "name": name, "value": value})

    def resync(self) -> bool:
        return self.send({"type": "resync"})

    # -- thread ------------------------------------------------------------
    def _set_state(self, state: str, detail: str = "") -> None:
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
            try:
                sock = socket.create_connection((self.host, self.port), timeout=3.0)
                sock.settimeout(None)
                sock.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
                self._sock = sock
                with self._send_lock:
                    send_message(sock, {"type": "hello", "protocol": PROTOCOL_VERSION, "client": self.client_name})
                self._set_state(self.CONNECTED, "")
                while not self._stop.is_set():
                    header, arrays = recv_message(sock)
                    try:
                        self.on_message(header, arrays)
                    except Exception as e:  # noqa: BLE001
                        self.last_error = f"on_message failed: {e!r}"
            except (OSError, ConnectionError, ProtocolError) as e:
                if not self._stop.is_set():
                    self.last_error = str(e)
            finally:
                self._sock = None
                if sock is not None:
                    try:
                        sock.close()
                    except OSError:
                        pass
            if self._stop.is_set() or not self.reconnect:
                break
            self._set_state(self.DISCONNECTED, self.last_error)
            self._stop.wait(self.retry_seconds)
        self._set_state(self.DISCONNECTED, "stopped")
