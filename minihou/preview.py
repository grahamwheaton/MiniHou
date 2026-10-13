"""Geometry for the editor's 3-D preview: positions, triangles, edges and (optionally) point colours as raw arrays.

Wire format of ``POST /api/preview`` (``application/octet-stream``)::

    "MHPV" | u32 header_length | JSON header (padded with spaces to a multiple of 4) | array data

The header has ``points``, ``prims``, ``tris``, ``edges``, ``bounds`` ([min, max]), ``truncated``, ``proxy`` and ``arrays``: a list of
``{name, dtype, count, offset}`` (offset in bytes from the start of the data, every array 4-byte aligned). Arrays:
``P`` float32 (3 per point), ``tri`` uint32 (3 per triangle, point indices), ``edge`` uint32 (2 per edge), ``tri_prim`` uint32 (the
primitive number of each triangle, for picking faces), ``Cd`` float32 (3 per point) when the geometry has point colours and
``point_src`` uint32 (the original point each drawn point stands for) when the geometry was reduced. ``points``, ``prims`` are the
real counts; ``tris`` / ``edges`` what is drawn. ``proxy`` says whether the drawing was reduced to fit the budget (``active``), from
how many points / triangles (``points``, ``tris``) to how many (``shown_points``, ``shown_tris``).
"""
from __future__ import annotations

import copy
import json
import struct
import threading
import weakref
from typing import Dict, Tuple

import numpy as np

from . import proxy

MAGIC = b"MHPV"
# The editor's budget: more than this is reduced by :func:`proxy.decimate` (the whole shape, coarser), not cut off.
DISPLAY_TRIS = 250_000
DISPLAY_POINTS = 500_000
# "Full resolution" on request: the most the viewer is asked to hold (past this it reduces too, so a request can never hang the browser)
MAX_TRIS = 2_500_000
MAX_EDGES = 5_000_000
MAX_POINTS = 4_000_000
EXACT_PRIMS = 100_000            # up to this many primitives, polygons with more than 3 corners are triangulated exactly (concave ones too)


def _triangles(geo):
    """(triangles as point triples (T, 3), owning primitive (T,)) of the closed polygons."""
    from .nodes.model import triangulate
    t = geo.topology
    vp = t.vertex_point.astype(np.int64)
    if not t.nprims:
        return np.zeros((0, 3), np.int64), np.zeros(0, np.int64)
    if t.nprims <= EXACT_PRIMS:
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
        if not parts:
            return np.zeros((0, 3), np.int64), np.zeros(0, np.int64)
        return np.concatenate(parts), np.concatenate(owners)
    tri_v, owner = proxy.fan_triangles(geo)                         # a big mesh: fans (concave polygons may show a wrong diagonal)
    return vp[tri_v], owner.astype(np.int64)


_MEMO: "weakref.WeakKeyDictionary" = weakref.WeakKeyDictionary()      # geometry -> {(full, tris): (header, arrays)}
_MEMO_LOCK = threading.Lock()


def arrays_for(geo, full: bool = False, tris: int = 0) -> Tuple[dict, Dict[str, np.ndarray]]:
    """:func:`_arrays_for`, remembered per geometry (cooked geometry never changes, and the editor asks again for the same node
    whenever the selection or the view changes)."""
    key = (bool(full), int(tris), MAX_TRIS, MAX_POINTS, DISPLAY_TRIS, DISPLAY_POINTS)
    with _MEMO_LOCK:
        hit = _MEMO.get(geo, {}).get(key)
    if hit is None:
        hit = _arrays_for(geo, full, tris)
        if geo.frozen:
            with _MEMO_LOCK:
                slot = _MEMO.setdefault(geo, {})
                if len(slot) > 4:
                    slot.clear()
                slot[key] = hit
    head, arrays = hit
    return copy.deepcopy(head), dict(arrays)


def _arrays_for(geo, full: bool = False, tris: int = 0) -> Tuple[dict, Dict[str, np.ndarray]]:
    """(header fields, named arrays) describing ``geo`` for drawing.

    More than the budget (``DISPLAY_TRIS`` triangles / ``DISPLAY_POINTS`` points, or ``tris`` triangles when given; the larger
    ``MAX_*`` caps with ``full``) is reduced by clustering, so the whole shape stays visible; the header's ``proxy`` then says
    how much was reduced. ``point_src`` / ``prim_src`` (when reduced) give the original point and primitive each drawn one stands for.
    """
    from . import topo
    n0, tris0 = geo.npoints, proxy.triangle_count(geo)
    tri_cap = int(tris) if tris else (MAX_TRIS if full else DISPLAY_TRIS)
    pt_cap = MAX_POINTS if full else DISPLAY_POINTS
    red = proxy.decimate(geo, tri_cap, pt_cap)
    shown = red.geo
    t = shown.topology
    n = shown.npoints
    out: Dict[str, np.ndarray] = {}
    out["P"] = np.ascontiguousarray(shown.P, np.float32).reshape(-1)
    tri, tri_prim = _triangles(shown)
    edge = topo.edges(shown).points.astype(np.int64) if t.nprims else np.zeros((0, 2), np.int64)
    truncated = False
    if len(tri) > MAX_TRIS:                                          # only if a caller asked for more than the cap
        tri, tri_prim, truncated = tri[:MAX_TRIS], tri_prim[:MAX_TRIS], True
    if len(edge) > MAX_EDGES:
        edge, truncated = edge[:MAX_EDGES], True
    if red.reduced:
        tri_prim = red.prim_src[tri_prim] if len(tri_prim) else tri_prim
    out["tri"] = np.ascontiguousarray(tri, np.uint32).reshape(-1)
    out["tri_prim"] = np.ascontiguousarray(tri_prim, np.uint32)
    out["edge"] = np.ascontiguousarray(edge, np.uint32).reshape(-1)
    cd = shown.point.get("Cd")
    if cd is not None and cd.ndim == 2 and cd.shape[1] >= 3 and len(cd) == n:
        out["Cd"] = np.ascontiguousarray(np.clip(cd[:, :3], 0.0, 1.0), np.float32).reshape(-1)
    if red.reduced:
        out["point_src"] = np.ascontiguousarray(red.point_src, np.uint32)
    if geo.npoints:
        lo, hi = geo.P.min(0).astype(float).tolist(), geo.P.max(0).astype(float).tolist()
    else:
        lo = hi = [0.0, 0.0, 0.0]
    head = {"points": int(n0), "prims": int(geo.topology.nprims), "tris": int(len(tri)), "edges": int(len(edge)),
            "bounds": [lo, hi], "truncated": truncated,
            "proxy": {"active": bool(red.reduced), "points": int(n0), "tris": int(tris0), "shown_points": int(n), "shown_tris": int(len(tri)),
                      "full": bool(full), "max_tris": MAX_TRIS, "max_points": MAX_POINTS}}
    return head, out


def attr_arrays(geo, spec: dict, head: dict, arrays: Dict[str, np.ndarray]) -> None:
    """Add one attribute's values to a preview (``spec``: ``{"name", "domain": point|prim|edge}``) so the viewer can colour by it: the array
    ``attr`` (float32, one to three numbers per element, point order / primitive order / derived edge order) and the header's ``attr``
    (name, domain, components, ranges). Only for a view that is not reduced: the numbers of a clustered view's elements are not the real ones."""
    name, dom = str(spec.get("name") or ""), str(spec.get("domain") or "point")
    info = {"name": name, "domain": dom}
    head["attr"] = info
    if head["proxy"]["active"]:
        info["reduced"] = True
        return
    if dom == "edge":
        from . import edges
        a = edges.values(geo, edges.PREFIX + name) if edges.PREFIX + name in geo.vertex else None
    elif dom in ("point", "prim"):
        a = geo.table(dom).get(name)
    else:
        raise ValueError("domain must be point, prim or edge")
    if a is None or a.dtype.kind not in "fiub":
        info["missing"] = True
        return
    a2 = np.asarray(a).reshape(len(a), -1).astype(np.float32)[:, :3]
    nc = a2.shape[1]
    arrays["attr"] = np.ascontiguousarray(a2).reshape(-1)
    info.update(comps=nc, count=int(len(a2)))
    if len(a2):
        info["min"], info["max"] = [float(x) for x in a2.min(0)], [float(x) for x in a2.max(0)]
    else:
        info["min"], info["max"] = [0.0] * nc, [0.0] * nc


def mask_arrays(geo, spec: dict, head: dict, arrays: Dict[str, np.ndarray]) -> None:
    """Add a group's membership to a preview (``spec``: ``{"name", "domain": point|prim|edge}``) so the paint tool can limit a brush to it: the
    array ``mask`` (uint32, 1 = member, one per element in point order / primitive order / derived edge order) and the header's ``mask``
    (name, domain, count, members; ``missing`` when there is no such group). A point group asked for on primitives (or the other way round)
    is converted like every group field does. Like :func:`attr_arrays`, only for a view that is not reduced."""
    from . import edges, groups
    name, dom = str(spec.get("name") or ""), str(spec.get("domain") or "point")
    info = {"name": name, "domain": dom}
    head["mask"] = info
    if head["proxy"]["active"]:
        info["reduced"] = True
        return
    if dom == "edge":
        a = edges.values(geo, edges.PREFIX + name) if edges.PREFIX + name in geo.vertex else None
        m = None if a is None else np.asarray(a).reshape(len(a), -1)[:, 0] != 0
    elif dom in ("point", "prim"):
        m = groups.member(geo, dom, name) if name else None
    else:
        raise ValueError("domain must be point, prim or edge")
    if m is None:
        info["missing"] = True
        return
    arrays["mask"] = np.ascontiguousarray(m, np.uint32)
    info.update(count=int(len(m)), members=int(np.count_nonzero(m)))


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
