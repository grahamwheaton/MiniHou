"""Shared memory zero-copy transport for local link connections. Standard library + numpy only.
Verbatim copy shared with the Blender add-on (see tools/sync_addon.py).
"""
from __future__ import annotations

import os
import sys
import threading
from typing import Dict, List, Optional, Tuple

import numpy as np

# Neutralize multiprocessing.resource_tracker on Windows / embedded Blender Python
# to prevent rogue helper subprocess spawning and unlinking errors on exit.
try:
    from multiprocessing import resource_tracker
    resource_tracker.register = lambda *a, **kw: None
    resource_tracker.unregister = lambda *a, **kw: None
except Exception:
    pass

from multiprocessing import shared_memory

SHM_THRESHOLD = 64 * 1024  # 64 KB hybrid threshold
SHM_ALIGNMENT = 8
_SUPPORTED_DTYPES = {"<f4", "<f8", "<i4", "<i8", "|u1", "|b1"}


def calculate_arrays_size(arrays: Optional[Dict[str, np.ndarray]]) -> int:
    """Calculate the total byte size required for 8-byte aligned arrays."""
    if not arrays:
        return 0
    total = 0
    for arr in arrays.values():
        a = np.asarray(arr)
        nb = a.nbytes
        pad = (-nb) % SHM_ALIGNMENT
        total += nb + pad
    return total


def write_arrays_to_buffer(buf: memoryview, arrays: Dict[str, np.ndarray], offset_start: int = 0) -> Tuple[List[dict], int]:
    """Write arrays into contiguous aligned chunks in buf. Returns (descriptors, total_bytes)."""
    offset = offset_start
    descs = []
    for key, arr in arrays.items():
        a = np.asarray(arr)
        if a.dtype.str not in _SUPPORTED_DTYPES:
            raise ValueError(f"array {key!r}: unsupported dtype {a.dtype}")
        if not a.flags.c_contiguous:
            a = np.ascontiguousarray(a)
        nbytes = a.nbytes
        descs.append({"key": key, "dtype": a.dtype.str, "shape": list(a.shape), "offset": offset, "nbytes": nbytes})
        if nbytes:
            buf[offset:offset + nbytes] = memoryview(a).cast("B")
        pad = (-nbytes) % SHM_ALIGNMENT
        if pad:
            buf[offset + nbytes:offset + nbytes + pad] = b"\0" * pad
        offset += nbytes + pad
    return descs, offset - offset_start


def read_arrays_from_buffer(buf: memoryview, descriptors: List[dict]) -> Dict[str, np.ndarray]:
    """Expose zero-copy numpy array views from buf according to descriptors."""
    arrays: Dict[str, np.ndarray] = {}
    for d in descriptors:
        if d["dtype"] not in _SUPPORTED_DTYPES:
            raise ValueError(f"unsupported dtype {d['dtype']!r}")
        dt = np.dtype(d["dtype"])
        shape = tuple(int(s) for s in d["shape"])
        count = int(np.prod(shape, dtype=np.int64)) if shape else 1
        off = int(d["offset"])
        if count:
            a = np.frombuffer(buf, dtype=dt, count=count, offset=off).reshape(shape)
        else:
            a = np.zeros(0, dt).reshape(shape)
        arrays[d["key"]] = a
    return arrays


class ShmSlot:
    """A persistent shared memory slot in the double-buffered ring."""

    def __init__(self, slot_id: int, base_name: str, initial_size: int = 128 * 1024):
        self.slot_id = slot_id
        self.base_name = base_name
        self.revision = 0
        self.name = f"{base_name}_{slot_id}_{self.revision}"
        self.size = initial_size
        self.shm: Optional[shared_memory.SharedMemory] = None
        self.busy = False
        self.seq = 0
        self._lock = threading.Lock()
        self._ensure_allocated(initial_size)

    def _ensure_allocated(self, size: int) -> None:
        if self.shm is not None and self.size >= size:
            return
        if self.shm is not None:
            try:
                self.shm.close()
                self.shm.unlink()
            except Exception:
                pass
            self.shm = None
        self.revision += 1
        self.name = f"{self.base_name}_{self.slot_id}_{self.revision}"
        self.size = size
        try:
            old = shared_memory.SharedMemory(name=self.name, create=False)
            old.close()
            old.unlink()
        except Exception:
            pass
        self.shm = shared_memory.SharedMemory(name=self.name, create=True, size=self.size)

    def prepare(self, required_bytes: int, seq: int) -> Tuple[str, memoryview]:
        """Ensure capacity, mark busy with sequence number, and return (shm_name, memoryview)."""
        with self._lock:
            if required_bytes > self.size:
                new_size = max(required_bytes * 3 // 2, 64 * 1024)
                self._ensure_allocated(new_size)
            self.busy = True
            self.seq = seq
            return self.name, self.shm.buf

    def release(self, seq: Optional[int] = None) -> None:
        with self._lock:
            if seq is None or self.seq == seq:
                self.busy = False

    def close(self) -> None:
        with self._lock:
            if self.shm is not None:
                try:
                    self.shm.close()
                    self.shm.unlink()
                except Exception:
                    pass
                self.shm = None


class ShmRing:
    """Double-buffered persistent shared memory ring per connection."""

    def __init__(self, prefix: Optional[str] = None):
        pid = os.getpid()
        ident = id(self)
        base = prefix or f"minihou_shm_{pid}_{ident}"
        self.slots = [ShmSlot(0, base), ShmSlot(1, base)]
        self._lock = threading.Lock()

    def get_slot_for_write(self, required_bytes: int, seq: int) -> Tuple[int, str, memoryview]:
        with self._lock:
            preferred_idx = seq % 2
            slot = self.slots[preferred_idx]
            if slot.busy:
                alt_slot = self.slots[1 - preferred_idx]
                if not alt_slot.busy:
                    slot = alt_slot
                else:
                    # Both busy: overwrite the older unconsumed slot
                    slot = self.slots[preferred_idx]
            name, buf = slot.prepare(required_bytes, seq)
            return slot.slot_id, name, buf

    def on_ack(self, seq: int, slot_id: Optional[int] = None) -> None:
        with self._lock:
            if slot_id is not None and 0 <= slot_id < len(self.slots):
                self.slots[slot_id].release(seq)
            else:
                for s in self.slots:
                    if s.seq == seq:
                        s.release(seq)

    def close(self) -> None:
        with self._lock:
            for s in self.slots:
                s.close()


class ShmReader:
    """Maintains cached attachments to shared memory segments for zero-copy reading."""

    def __init__(self):
        self._attached: Dict[str, shared_memory.SharedMemory] = {}
        self._lock = threading.Lock()

    def get_buffer(self, name: str) -> memoryview:
        with self._lock:
            shm = self._attached.get(name)
            if shm is None:
                shm = shared_memory.SharedMemory(name=name, create=False)
                self._attached[name] = shm
            return shm.buf

    def release_older_than(self, active_names: set) -> None:
        """Close attachments to SHM segments that are no longer active."""
        with self._lock:
            stale = [name for name in self._attached if name not in active_names]
            for name in stale:
                try:
                    self._attached[name].close()
                except Exception:
                    pass
                del self._attached[name]

    def close(self) -> None:
        with self._lock:
            for shm in self._attached.values():
                try:
                    shm.close()
                except Exception:
                    pass
            self._attached.clear()


def encode_shm_message(header: dict, arrays: Dict[str, np.ndarray], shm_ring: ShmRing, seq: int = 0) -> List[object]:
    """Encode a message whose binary arrays are placed into a shared memory slot. The frame's blen is 0."""
    import json
    from .protocol import MAGIC, _PREFIX, _pad8
    nbytes = calculate_arrays_size(arrays)
    slot_id, shm_name, buf = shm_ring.get_slot_for_write(nbytes, seq)
    descs, _ = write_arrays_to_buffer(buf, arrays)
    h = dict(header)
    h["transport"] = "shm"
    h["shm_name"] = shm_name
    h["shm_slot"] = slot_id
    h["shm_size"] = nbytes
    h["arrays"] = descs
    hb = json.dumps(h, separators=(",", ":"), allow_nan=False).encode("utf-8")
    hb += b" " * _pad8(len(hb))
    return [_PREFIX.pack(MAGIC, len(hb), 0), hb]


def send_shm_or_tcp_message(sock, header: dict, arrays: Optional[Dict[str, np.ndarray]] = None,
                            shm_ring: Optional[ShmRing] = None, seq: int = 0) -> int:
    """Send one frame using shared memory if arrays >= SHM_THRESHOLD and shm_ring is available, else TCP."""
    from .protocol import send_message
    if shm_ring is not None and arrays:
        nbytes = calculate_arrays_size(arrays)
        if nbytes >= SHM_THRESHOLD:
            bufs = encode_shm_message(header, arrays, shm_ring, seq)
            total = 0
            for b in bufs:
                sock.sendall(b)
                total += len(b)
            return total
    h = dict(header)
    h.setdefault("transport", "tcp")
    return send_message(sock, h, arrays)


def unpack_shm_payload(header: dict, shm_reader: Optional[ShmReader] = None) -> Dict[str, np.ndarray]:
    """Extract zero-copy numpy array views from shared memory as described in header."""
    shm_name = header.get("shm_name")
    if not shm_name:
        raise ValueError("shm transport message missing shm_name")
    if shm_reader is None:
        r = ShmReader()
        try:
            buf = r.get_buffer(shm_name)
            return read_arrays_from_buffer(buf, header.get("arrays", []))
        finally:
            r.close()
    buf = shm_reader.get_buffer(shm_name)
    return read_arrays_from_buffer(buf, header.get("arrays", []))


def recv_shm_or_tcp_message(sock, shm_reader: Optional[ShmReader] = None) -> Tuple[dict, Dict[str, np.ndarray]]:
    """Receive one frame, reading array data from shared memory if transport is 'shm'."""
    from .protocol import recv_message
    header, arrays = recv_message(sock)
    if header and header.get("transport") == "shm":
        arrays = unpack_shm_payload(header, shm_reader)
    return header, arrays


