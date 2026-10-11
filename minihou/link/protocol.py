"""Wire protocol for the live link. Standard library + numpy only, and **no relative imports**:
this exact file is copied into the Blender addon (``tools/sync_addon.py``; a test keeps them
identical), so it must stay self-contained.

Frame layout (all integers little-endian)::

    magic   4 bytes  b"MHL1"
    hlen    u32      length of the (space padded) JSON header, always a multiple of 8
    blen    u64      length of the binary blob
    header  hlen     UTF-8 JSON object, must contain "type"
    blob    blen     raw array data; each array starts on an 8-byte boundary

``header["arrays"]`` (optional) describes the blob::

    [{"key": "point/P", "dtype": "<f4", "shape": [N, 3], "offset": 0, "nbytes": 12N}, ...]

The receiver reads header+blob into one ``bytearray`` and exposes arrays as zero-copy
``np.frombuffer`` views, so a 1M-point position array costs one ``recv_into`` and no copies.

Message types (``header["type"]``)
----------------------------------
client -> server: ``hello``, ``set_param``, ``resync``, ``ping``, ``object_geo``, ``object_missing``, ``set_frame``
(``{frame, fps}``: the timeline moved; nodes using @Frame / @Time recook)
server -> client: ``welcome``, ``interface``, ``geo``, ``error``, ``status``, ``pong``, ``watch_objects``

``watch_objects`` lists the Blender objects the graph reads (``blender_object`` nodes). The client answers with
``object_geo`` for each (and again whenever one changes): ``{"object", "matrix": 16 floats row-major, "info"}`` plus
arrays keyed like ``geo`` (``point/P``, the two ``topo/...`` arrays, ``<domain>/<name>``), or ``object_missing``
``{"object", "reason"}``.

Modifier RPC (opt in with ``hello`` ``mode: "instances"``) is separate from the legacy
proxy subscription. ``welcome`` advertises ``capabilities: ["instances_v1"]``, ``graph_path``
and ``editor_url``. Requests carry a ``request_id`` echoed in replies:

* ``instance_describe``: ``graph_path`` or embedded ``graph`` -> ``instance_description``
  with graph snapshot and exposed ``interface`` list.
* ``instance_cook``: ``instance_id``, graph source, complete exposed ``params`` overrides,
  ``frame``, ``fps``, plus input mesh arrays -> ``instance_result`` (full geometry arrays,
  the usual geo header and exposed interface). ``modifier_input`` receives these arrays.
* ``instance_remove`` -> ``instance_removed``. Disconnect releases all owned instances.
* Errors are ``instance_error`` with ``message`` and matching request/instance IDs.

Instance IDs are scoped to the connection; requests cannot mutate the browser graph.
``blender_object`` is rejected in modifier graphs to prevent evaluated-output feedback.

``geo`` carries only the arrays that changed since the last ``geo`` sent to *that* client,
plus a manifest of every array that currently exists (so removals are explicit) and, if the
connectivity changed, the two topology arrays. See :class:`GeoUpdate` and :class:`Mirror`.
"""
from __future__ import annotations

import json
import socket
import struct
import time
from typing import Dict, Iterable, List, Optional, Tuple

import numpy as np

PROTOCOL_VERSION = 1
MAGIC = b"MHL1"
_PREFIX = struct.Struct("<4sIQ")
MAX_HEADER = 64 * 1024 * 1024
MAX_BLOB = 8 * 1024 ** 3
DEFAULT_PORT = 8765

TOPO_VERTEX_POINT = "topo/vertex_point"
TOPO_PRIM_OFFSETS = "topo/prim_offsets"
TOPO_PRIM_CLOSED = "topo/prim_closed"      # uint8 per primitive: 1 polygon, 0 open curve (absent: all polygons)
TOPO_REQUIRED = (TOPO_VERTEX_POINT, TOPO_PRIM_OFFSETS)            # always sent when topology changes
TOPO_KEYS = TOPO_REQUIRED + (TOPO_PRIM_CLOSED,)
DOMAINS = ("point", "vertex", "prim")

_SUPPORTED_DTYPES = {"<f4", "<f8", "<i4", "<i8", "|u1", "|b1"}


class ProtocolError(RuntimeError):
    pass


def attr_key(domain: str, name: str) -> str:
    return f"{domain}/{name}"


def split_key(key: str) -> Tuple[str, str]:
    dom, _, name = key.partition("/")
    return dom, name


# ---------------------------------------------------------------------------
# Framing
# ---------------------------------------------------------------------------
def _pad8(n: int) -> int:
    return (-n) % 8


def encode_message(header: dict, arrays: Optional[Dict[str, np.ndarray]] = None) -> List[object]:
    """Return buffers (bytes / memoryview) that together form one frame. Arrays are not copied
    unless they are non-contiguous or have an unsupported byte order."""
    descs = []
    chunks: List[object] = []
    offset = 0
    for key, arr in (arrays or {}).items():
        a = np.asarray(arr)
        if a.dtype.str not in _SUPPORTED_DTYPES:
            raise ProtocolError(f"array {key!r}: unsupported dtype {a.dtype}")
        if not a.flags.c_contiguous:
            a = np.ascontiguousarray(a)
        nbytes = a.nbytes
        descs.append({"key": key, "dtype": a.dtype.str, "shape": list(a.shape), "offset": offset, "nbytes": nbytes})
        if nbytes:
            chunks.append(memoryview(a).cast("B"))
        pad = _pad8(nbytes)
        if pad:
            chunks.append(b"\0" * pad)
        offset += nbytes + pad
    h = dict(header)
    if descs:
        h["arrays"] = descs
    hb = json.dumps(h, separators=(",", ":"), allow_nan=False).encode("utf-8")
    hb += b" " * _pad8(len(hb))
    return [_PREFIX.pack(MAGIC, len(hb), offset), hb] + chunks


def send_message(sock: socket.socket, header: dict, arrays: Optional[Dict[str, np.ndarray]] = None) -> int:
    """Send one frame. Returns the number of bytes written."""
    bufs = encode_message(header, arrays)
    total = 0
    for b in bufs:
        sock.sendall(b)
        total += len(b)
    return total


def _recv_exact_into(sock: socket.socket, view: memoryview) -> None:
    got = 0
    n = len(view)
    while got < n:
        r = sock.recv_into(view[got:], n - got)
        if r == 0:
            raise ConnectionError("connection closed by peer")
        got += r


def recv_message(sock: socket.socket) -> Tuple[dict, Dict[str, np.ndarray]]:
    """Read one frame. Returns ``(header, arrays)``; arrays are views into a fresh bytearray."""
    prefix = bytearray(_PREFIX.size)
    _recv_exact_into(sock, memoryview(prefix))
    magic, hlen, blen = _PREFIX.unpack(prefix)
    if magic != MAGIC:
        raise ProtocolError(f"bad magic {bytes(magic)!r}")
    if hlen > MAX_HEADER or blen > MAX_BLOB or hlen % 8:
        raise ProtocolError(f"unreasonable frame sizes hlen={hlen} blen={blen}")
    buf = bytearray(hlen + blen)
    _recv_exact_into(sock, memoryview(buf))
    return decode_payload(buf, hlen)


def decode_payload(buf, hlen: int) -> Tuple[dict, Dict[str, np.ndarray]]:
    mv = memoryview(buf)
    try:
        header = json.loads(bytes(mv[:hlen]).decode("utf-8"))
    except (UnicodeDecodeError, json.JSONDecodeError) as e:
        raise ProtocolError(f"bad header: {e}") from None
    if not isinstance(header, dict) or "type" not in header:
        raise ProtocolError("header must be an object with a 'type'")
    arrays: Dict[str, np.ndarray] = {}
    blob_len = len(mv) - hlen
    for d in header.get("arrays", ()):
        if d["dtype"] not in _SUPPORTED_DTYPES:
            raise ProtocolError(f"unsupported dtype {d['dtype']!r}")
        dt = np.dtype(d["dtype"])
        shape = tuple(int(s) for s in d["shape"])
        count = int(np.prod(shape, dtype=np.int64)) if shape else 1
        off = hlen + int(d["offset"])
        if int(d["offset"]) + count * dt.itemsize > blob_len or int(d["nbytes"]) != count * dt.itemsize:
            raise ProtocolError(f"array {d['key']!r} lies outside the blob")
        a = np.frombuffer(buf, dtype=dt, count=count, offset=off) if count else np.zeros(0, dt)
        arrays[d["key"]] = a.reshape(shape)
    return header, arrays


# ---------------------------------------------------------------------------
# Geometry updates
# ---------------------------------------------------------------------------
class GeoUpdate:
    """A decoded ``geo`` message."""

    __slots__ = ("seq", "full", "counts", "topology_changed", "manifest", "arrays", "detail", "cook",
                 "t_sent", "t_recv")

    def __init__(self, seq=0, full=False, counts=None, topology_changed=False, manifest=None, arrays=None,
                 detail=None, cook=None, t_sent=0.0, t_recv=0.0):
        self.seq = seq
        self.full = full
        self.counts = counts or {"point": 0, "vertex": 0, "prim": 0}
        self.topology_changed = topology_changed
        self.manifest = manifest or {}       # key -> {"dtype": str, "shape": [..]} for every current array
        self.arrays = arrays or {}           # key -> ndarray, only the ones that changed
        self.detail = detail or {}
        self.cook = cook or {}
        self.t_sent = t_sent
        self.t_recv = t_recv

    @classmethod
    def from_message(cls, header: dict, arrays: Dict[str, np.ndarray]) -> "GeoUpdate":
        return cls(seq=header["seq"], full=header.get("full", False), counts=header["counts"],
                   topology_changed=header.get("topology_changed", False), manifest=header["manifest"],
                   arrays=arrays, detail=header.get("detail", {}), cook=header.get("cook", {}),
                   t_sent=header.get("t_sent", 0.0), t_recv=time.time())

    def header(self) -> dict:
        return {"type": "geo", "seq": self.seq, "full": self.full, "counts": self.counts,
                "topology_changed": self.topology_changed, "manifest": self.manifest,
                "detail": self.detail, "cook": self.cook, "t_sent": self.t_sent}

    @property
    def nbytes(self) -> int:
        return sum(a.nbytes for a in self.arrays.values())


def merge_updates(older: GeoUpdate, newer: GeoUpdate) -> GeoUpdate:
    """Collapse two consecutive updates into one equivalent update (``newer`` wins).

    Used by the Blender side so a burst of slider changes costs one mesh write, not many.
    Applying ``merge_updates(a, b)`` is equivalent to applying ``a`` then ``b``.
    """
    live = set(newer.manifest) | set(TOPO_KEYS if (older.topology_changed or newer.topology_changed) else ())
    arrays = {k: v for k, v in older.arrays.items() if k in live}
    arrays.update(newer.arrays)
    return GeoUpdate(seq=newer.seq, full=older.full or newer.full, counts=newer.counts,
                     topology_changed=older.topology_changed or newer.topology_changed,
                     manifest=newer.manifest, arrays=arrays, detail=newer.detail, cook=newer.cook,
                     t_sent=newer.t_sent, t_recv=newer.t_recv)


class Mirror:
    """The receiver's copy of the current geometry arrays, kept up to date by applying updates."""

    def __init__(self):
        self.arrays: Dict[str, np.ndarray] = {}
        self.counts = {"point": 0, "vertex": 0, "prim": 0}
        self.detail: dict = {}
        self.seq = -1

    def apply(self, u: GeoUpdate) -> None:
        if u.full:
            self.arrays.clear()
        for k in list(self.arrays):
            if k not in u.manifest and k not in TOPO_KEYS:
                del self.arrays[k]
        self.arrays.update(u.arrays)
        self.counts = dict(u.counts)
        self.detail = dict(u.detail)
        self.seq = u.seq
        missing = [k for k in u.manifest if k not in self.arrays]
        if missing:
            raise ProtocolError(f"update is missing arrays for {missing}; a resync is needed")
        for k, info in u.manifest.items():
            a = self.arrays[k]
            if list(a.shape) != list(info["shape"]) or a.dtype.str != info["dtype"]:
                raise ProtocolError(f"array {k!r} is {a.dtype.str}{list(a.shape)} but manifest says {info}")

    # convenient views --------------------------------------------------------
    @property
    def P(self) -> np.ndarray:
        return self.arrays[attr_key("point", "P")]

    @property
    def vertex_point(self) -> np.ndarray:
        return self.arrays[TOPO_VERTEX_POINT]

    @property
    def prim_offsets(self) -> np.ndarray:
        return self.arrays[TOPO_PRIM_OFFSETS]

    @property
    def prim_closed(self):
        """Bool per primitive (False = open curve), or None when every primitive is a polygon."""
        c = self.arrays.get(TOPO_PRIM_CLOSED)
        if c is None or c.size == 0 or bool(np.all(c)):
            return None
        return c.astype(bool)

    def attributes(self) -> Iterable[Tuple[str, str, np.ndarray]]:
        """Yield ``(domain, name, array)`` for every non-topology array."""
        for k, a in self.arrays.items():
            if k in TOPO_KEYS:
                continue
            dom, name = split_key(k)
            yield dom, name, a
