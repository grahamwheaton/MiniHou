"""Geometry for the editor's 3-D preview: positions, triangles, edges and (optionally) point colours as raw arrays.

Wire format of ``POST /api/preview`` (``application/octet-stream``)::

    "MHPV" | u32 header_length | JSON header (padded with spaces to a multiple of 4) | array data

The header has ``points``, ``prims``, ``tris``, ``edges``, ``bounds`` ([min, max]), ``truncated`` and ``arrays``: a list of
``{name, dtype, count, offset}`` (offset in bytes from the start of the data, every array 4-byte aligned). Arrays:
``P`` float32 (3 per point), ``tri`` uint32 (3 per triangle, point indices), ``edge`` uint32 (2 per edge), ``tri_prim`` uint32 (the
primitive number of each triangle, for picking faces) and ``Cd`` float32 (3 per point) when the geometry has point colours.
"""
from __future__ import annotations

import json
import struct
from typing import Dict, Tuple

import numpy as np

MAGIC = b"MHPV"
MAX_TRIS = 600_000
MAX_EDGES = 1_200_000
MAX_POINTS = 2_000_000


def arrays_for(geo) -> Tuple[dict, Dict[str, np.ndarray]]:
    """(header fields, named arrays) describing ``geo`` for drawing."""
    from . import topo
    from .nodes.model import triangulate
    t = geo.topology
    n = geo.npoints
    out: Dict[str, np.ndarray] = {}
    truncated = False
    P = np.ascontiguousarray(geo.P, np.float32)
    keep_points = np.arange(n)
    if n > MAX_POINTS:                                       # too many to draw: an even sample (meshes keep their points)
        if t.nprims == 0:
            keep_points = np.linspace(0, n - 1, MAX_POINTS).astype(np.int64)
            P = P[keep_points]
        truncated = True
    out["P"] = P.reshape(-1)
    tri = np.zeros((0, 3), np.int64)
    tri_prim = np.zeros(0, np.int64)
    edge = np.zeros((0, 2), np.int64)
    if t.nprims and n <= MAX_POINTS:
        vp = t.vertex_point.astype(np.int64)
        offs = t.prim_offsets.astype(np.int64)
        sizes = np.diff(offs)
        closed = t.closed_mask()
        small = np.flatnonzero(closed & (sizes == 3))
        parts = [vp[offs[small][:, None] + np.arange(3)]] if len(small) else []
        owners = [small] if len(small) else []
        if (closed & (sizes > 3)).any():
            tri_v, tri_p, _ = triangulate(geo, np.ones(t.nprims, bool))
            if len(tri_v):
                parts.append(vp[tri_v])
                owners.append(tri_p)
        if parts:
            tri = np.concatenate(parts)
            tri_prim = np.concatenate(owners)
        edge = topo.edges(geo).points.astype(np.int64)
    if len(tri) > MAX_TRIS:
        tri, tri_prim, truncated = tri[:MAX_TRIS], tri_prim[:MAX_TRIS], True
    if len(edge) > MAX_EDGES:
        edge, truncated = edge[:MAX_EDGES], True
    out["tri"] = np.ascontiguousarray(tri, np.uint32).reshape(-1)
    out["tri_prim"] = np.ascontiguousarray(tri_prim, np.uint32)
    out["edge"] = np.ascontiguousarray(edge, np.uint32).reshape(-1)
    cd = geo.point.get("Cd")
    if cd is not None and cd.ndim == 2 and cd.shape[1] >= 3 and len(cd) == n:
        out["Cd"] = np.ascontiguousarray(np.clip(cd[keep_points][:, :3], 0.0, 1.0), np.float32).reshape(-1)
    if n:
        lo, hi = geo.P.min(0).astype(float).tolist(), geo.P.max(0).astype(float).tolist()
    else:
        lo = hi = [0.0, 0.0, 0.0]
    head = {"points": int(n), "prims": int(t.nprims), "tris": int(len(tri)), "edges": int(len(edge)),
            "bounds": [lo, hi], "truncated": truncated}
    return head, out


def pack(head: dict, arrays: Dict[str, np.ndarray]) -> bytes:
    """Serialise header fields and arrays in the wire format above."""
    table, blobs, offset = [], [], 0
    for name, a in arrays.items():
        raw = np.ascontiguousarray(a).tobytes()
        table.append({"name": name, "dtype": str(a.dtype), "count": int(a.size), "offset": offset})
        pad = (-len(raw)) % 4
        blobs.append(raw + b"\0" * pad)
        offset += len(raw) + pad
    body = json.dumps(dict(head, ok=True, arrays=table), allow_nan=False).encode("utf-8")
    body += b" " * ((-len(body)) % 4)
    return MAGIC + struct.pack("<I", len(body)) + body + b"".join(blobs)


def unpack(data: bytes) -> Tuple[dict, Dict[str, np.ndarray]]:
    """Inverse of :func:`pack` (used by the tests; the browser has its own reader)."""
    if data[:4] != MAGIC:
        raise ValueError("not a preview message")
    (hl,) = struct.unpack("<I", data[4:8])
    head = json.loads(data[8:8 + hl].decode("utf-8"))
    base = 8 + hl
    arrays = {}
    for t in head["arrays"]:
        dt = np.dtype(t["dtype"])
        arrays[t["name"]] = np.frombuffer(data, dt, t["count"], base + t["offset"])
    return head, arrays
