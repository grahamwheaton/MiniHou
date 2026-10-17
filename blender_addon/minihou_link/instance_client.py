"""Worker-only RPC I/O with adaptive main-thread budget; shared with Blender.

On timeout raise OSError so modifiers retain their last good mesh and use their
existing retry/backoff path. A timeout never returns a stale reply as a new cook.
"""
from __future__ import annotations
import socket
import threading
import time
from typing import Optional
from .protocol import PROTOCOL_VERSION, ProtocolError, recv_message, send_message
from .shm import ShmReader, unpack_shm_payload


class InstanceClient:
    def __init__(self, host="127.0.0.1", port=8765, timeout=30):
        self.host, self.port, self.timeout = host, port, timeout
        self.sock = None
        self.lock = threading.RLock()
        self.sequence = 0
        self.welcome = {}
        self.shm_reader = ShmReader()
        self._local = threading.local()
        self._thread = None
        self._epoch = 0

    def _invoke(self, operation):
        # Adaptive budget: honour explicit timeout (e.g. generous 30s deadlines for cooks
        # or debug servers), while preserving short budgets for polling/auto-connect.
        budget = max(0.001, float(self.timeout))
        deadline = time.monotonic() + budget
        epoch = self._epoch
        done, cancelled = threading.Event(), threading.Event()
        result = []
        def run():
            if not self.lock.acquire(timeout=max(0.001, deadline - time.monotonic())):
                result.append((False, TimeoutError("modifier client busy")))
                done.set()
                return
            self._local.deadline = deadline
            self._local.cancelled = cancelled
            self._local.epoch = epoch
            try:
                if cancelled.is_set() or time.monotonic() >= deadline:
                    raise TimeoutError("modifier RPC deadline exceeded")
                if epoch != self._epoch:
                    raise ConnectionError("modifier operation cancelled")
                value = operation()
                if epoch != self._epoch:
                    raise ConnectionError("modifier operation cancelled")
                result.append((True, value))
            except Exception as exc:
                result.append((False, exc))
            finally:
                self.lock.release()
                done.set()
        self._thread = threading.Thread(target=run, name="minihou-instance-rpc", daemon=True)
        self._thread.start()
        if not done.wait(max(0.0, deadline - time.monotonic())):
            cancelled.set()
            self.close()
            raise TimeoutError(f"MiniHou did not reply within {budget:.1f}s; last cache retained")
        ok, value = result[0]
        if not ok:
            raise value
        return value

    def connect(self):
        return self._invoke(self._connect)

    def _connect(self):
        with self.lock:
            existing = self.sock
            if existing is not None:
                existing.settimeout(0.025)
                return self.welcome
            remaining = self._local.deadline - time.monotonic()
            if remaining <= 0 or self._local.cancelled.is_set():
                raise TimeoutError("modifier connect cancelled")
            sock = socket.create_connection((self.host, self.port), remaining)
            sock.settimeout(0.025)
            self.sock = sock
            if self._local.cancelled.is_set() or self._local.epoch != self._epoch:
                self.close()
                raise ConnectionError("modifier connect cancelled")
            try:
                send_message(sock, {"type": "hello", "protocol": PROTOCOL_VERSION,
                                    "mode": "instances", "client": "Blender modifiers",
                                    "capabilities": ["shm_v1"]}, deadline=self._local.deadline)
                header, _ = recv_message(sock, deadline=self._local.deadline)
                if (header.get("type") != "welcome" or header.get("server") != "minihou"
                        or header.get("protocol") != PROTOCOL_VERSION
                        or "instances_v1" not in header.get("capabilities", [])):
                    raise RuntimeError("server does not support independent MiniHou modifier instances")
                self.welcome = header
                return header
            except Exception:
                self.close()
                raise

    def close(self):
        # Do not acquire the RPC lock: shutdown must interrupt a pending read/write.
        self._epoch += 1
        sock, self.sock = self.sock, None
        if sock is not None:
            try:
                sock.shutdown(socket.SHUT_RDWR)
            except OSError:
                pass
            sock.close()
        # An SHM read can own its lock. Never wait for it on Blender's main thread.
        reader, self.shm_reader = self.shm_reader, ShmReader()
        threading.Thread(target=reader.close, name="minihou-instance-close", daemon=True).start()

    def _request(self, header, arrays=None):
        return self._invoke(lambda: self._request_io(header, arrays))

    def _request_io(self, header, arrays=None):
        with self.lock:
            self._connect()
            self.sequence += 1
            header["request_id"] = self.sequence
            try:
                sock = self.sock
                if sock is None:
                    raise ConnectionError("modifier connection closed")
                send_message(sock, header, arrays, deadline=self._local.deadline)
                reply, output = recv_message(sock, deadline=self._local.deadline)
                if reply and reply.get("transport") == "shm":
                    output = unpack_shm_payload(reply, self.shm_reader)
                if reply.get("request_id") != self.sequence:
                    raise ProtocolError("modifier response request ID mismatch")
                if reply.get("instance_id") != header.get("instance_id"):
                    raise ProtocolError("modifier response instance ID mismatch")
                expected = {"instance_cook": "instance_result", "instance_describe": "instance_description",
                            "instance_remove": "instance_removed", "instance_edit": "instance_synced",
                            "instance_sync": "instance_synced", "instance_sources": "instance_sources_set",
                            "graph_push": "graph_ack", "graph_pull": "graph_state"}[header["type"]]
                if reply.get("type") not in (expected, "instance_error"):
                    raise ProtocolError("unexpected modifier response type")
            except Exception:
                self.close()
                raise
            # A correlated engine cook error is a complete valid response; keep this
            # connection and its other independent instances alive.
            if reply.get("type") == "instance_error":
                raise RuntimeError(reply.get("message", "modifier cook failed"))
            return reply, output

    def describe(self, graph_path, graph=None):
        return self._request({"type": "instance_describe", "graph_path": graph_path, "graph": graph})[0]

    def cook(self, instance_id, graph_path, params, arrays, *, frame=1, fps=24, graph=None, materials=None, render=False, references=None):
        header = {"type": "instance_cook", "instance_id": instance_id,
                  "graph_path": graph_path, "graph": graph, "params": params,
                  "frame": frame, "fps": fps, "render": bool(render)}
        if references:
            header["references"] = references
        if materials and any(materials):
            header["materials"] = list(materials)              # slot names of the geometry going in (see minihou.materials)
        return self._request(header, arrays)

    def remove(self, instance_id):
        return self._request({"type": "instance_remove", "instance_id": instance_id})[0]

    def edit_live(self, instance_id, graph, params, arrays, *, label="Blender modifier", frame=1, fps=24, references=None):
        self.connect()
        if "live_modifier_v1" not in self.welcome.get("capabilities", []):
            raise RuntimeError("Restart MiniHou with the updated app to enable live modifier editing")
        return self._request({"type": "instance_edit", "instance_id": instance_id,
                              "graph": graph, "params": params, "label": label, "frame": frame, "fps": fps,
                              "references": references or {}}, arrays)[0]

    def set_sources(self, instance_id, sources):
        self.connect()
        if "object_inputs_v1" not in self.welcome.get("capabilities", []):
            raise RuntimeError("Restart the updated MiniHou app to load object inputs")
        arrays = {str(i) + "/" + key: value for i, values in enumerate(sources.values()) for key, value in values.items()}
        return self._request({"type": "instance_sources", "instance_id": instance_id, "sources": list(sources)}, arrays)[0]

    def has_capability(self, name):
        self.connect()
        return name in self.welcome.get("capabilities", [])

    def push_graph(self, instance_id, *, name, label, text, sha256, schema=1):
        """graph_embed_v1: send an embedded network (text is the exact JSON; sha256 covers its UTF-8 bytes)."""
        if not self.has_capability("graph_embed_v1"):
            raise RuntimeError("This MiniHou app predates embedded networks (graph_embed_v1)")
        return self._request({"type": "graph_push", "instance_id": instance_id, "schema": schema,
                              "name": name, "label": label, "json": text, "hash": sha256,
                              "size": len(text.encode("utf-8"))})[0]

    def pull_graph(self, instance_id):
        """graph_embed_v1: the app's current copy of a network, or a pending conflict's outcome."""
        if not self.has_capability("graph_embed_v1"):
            return {"type": "graph_state", "status": "unsupported"}
        return self._request({"type": "graph_pull", "instance_id": instance_id})[0]

    def sync_live(self, instance_id, params):
        return self._request({"type": "instance_sync", "instance_id": instance_id, "params": params})[0]

    def ack(self, seq: int, slot: Optional[int] = None) -> None:
        # ACKs share the same bounded worker path; no raw sendall in a timer.
        def send_ack():
            if self.sock is not None:
                self.sock.settimeout(0.025)
                send_message(self.sock, {"type": "geo_ack", "seq": seq, "slot": slot},
                             deadline=self._local.deadline)
        try:
            self._invoke(send_ack)
        except (OSError, ProtocolError):
            self.close()
