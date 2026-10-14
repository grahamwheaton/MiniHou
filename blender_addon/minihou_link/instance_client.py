"""Synchronous instance RPC client shared verbatim with Blender; no engine imports."""
from __future__ import annotations
import socket
import threading
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

    def connect(self):
        with self.lock:
            if self.sock is not None:
                return
            sock = socket.create_connection((self.host, self.port), self.timeout)
            self.sock = sock
            try:
                send_message(sock, {"type": "hello", "protocol": PROTOCOL_VERSION,
                                    "mode": "instances", "client": "Blender modifiers",
                                    "capabilities": ["shm_v1"]})
                header, _ = recv_message(sock)
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
        with self.lock:
            if self.sock is not None:
                try:
                    self.sock.close()
                finally:
                    self.sock = None
            self.shm_reader.close()

    def _request(self, header, arrays=None):
        with self.lock:
            self.connect()
            self.sequence += 1
            header["request_id"] = self.sequence
            try:
                send_message(self.sock, header, arrays)
                reply, output = recv_message(self.sock)
                if reply and reply.get("transport") == "shm":
                    output = unpack_shm_payload(reply, self.shm_reader)
                if reply.get("request_id") != self.sequence:
                    raise ProtocolError("modifier response request ID mismatch")
                if reply.get("instance_id") != header.get("instance_id"):
                    raise ProtocolError("modifier response instance ID mismatch")
                expected = {"instance_cook": "instance_result", "instance_describe": "instance_description",
                            "instance_remove": "instance_removed", "instance_edit": "instance_synced",
                            "instance_sync": "instance_synced", "instance_sources": "instance_sources_set"}[header["type"]]
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

    def cook(self, instance_id, graph_path, params, arrays, *, frame=1, fps=24, graph=None, materials=None, render=False):
        header = {"type": "instance_cook", "instance_id": instance_id,
                  "graph_path": graph_path, "graph": graph, "params": params,
                  "frame": frame, "fps": fps, "render": bool(render)}
        if materials and any(materials):
            header["materials"] = list(materials)              # slot names of the geometry going in (see minihou.materials)
        return self._request(header, arrays)

    def remove(self, instance_id):
        return self._request({"type": "instance_remove", "instance_id": instance_id})[0]

    def edit_live(self, instance_id, graph, params, arrays, *, label="Blender modifier", frame=1, fps=24):
        self.connect()
        if "live_modifier_v1" not in self.welcome.get("capabilities", []):
            raise RuntimeError("Restart MiniHou with the updated app to enable live modifier editing")
        return self._request({"type": "instance_edit", "instance_id": instance_id,
                              "graph": graph, "params": params, "label": label, "frame": frame, "fps": fps}, arrays)[0]

    def set_sources(self, instance_id, sources):
        self.connect()
        if "object_inputs_v1" not in self.welcome.get("capabilities", []):
            raise RuntimeError("Restart the updated MiniHou app to load object inputs")
        arrays = {str(i) + "/" + key: value for i, values in enumerate(sources.values()) for key, value in values.items()}
        return self._request({"type": "instance_sources", "instance_id": instance_id, "sources": list(sources)}, arrays)[0]

    def sync_live(self, instance_id, params):
        return self._request({"type": "instance_sync", "instance_id": instance_id, "params": params})[0]

    def ack(self, seq: int, slot: Optional[int] = None) -> None:
        with self.lock:
            if self.sock is not None:
                try:
                    send_message(self.sock, {"type": "geo_ack", "seq": seq, "slot": slot})
                except Exception:
                    pass

