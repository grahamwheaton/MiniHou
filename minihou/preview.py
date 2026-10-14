"""Geometry for the editor's 3-D preview: positions, triangles, edges and (optionally) point colours as raw arrays.

Wire format of ``POST /api/preview`` (``application/octet-stream``)::

    "MHPV" | u32 header_length | JSON header (padded with spaces to a multiple of 4) | array data

The header has ``points``, ``prims``, ``tris``, ``edges``, ``bounds`` ([min, max]), ``truncated``, ``proxy`` and ``arrays``: a list of
``{name, dtype, count, offset}`` (offset in bytes from the start of the data, every array 4-byte aligned). Arrays:
``P`` float32 (3 per point), ``tri`` uint32 (3 per triangle, point indices), ``edge`` uint32 (2 per edge), ``tri_prim`` uint32 (the
primitive number of each triangle, for picking faces), ``Cd`` float32 (3 per point) when the geometry has point colours and
``point_src`` uint32 (the original point each drawn point stands for) when the geometry was reduced, and ``chunks`` (float32, 8 per run of
``CHUNK_TRIS`` triangles: first triangle, count, bounding box min xyz and max xyz; the triangles are then sorted so that runs are compact) from
``CHUNK_MIN_TRIS`` triangles on. A header with ``instances`` carries impostors, see :mod:`minihou.impostors`. ``points``, ``prims`` are the
real counts; ``tris`` / ``edges`` what is drawn. ``proxy`` says whether the drawing was reduced to fit the budget (``active``), from
how many points / triangles (``points``, ``tris``) to how many (``shown_points``, ``shown_tris``). ``topology`` (when not reduced) names the
triangles and edges: a request that sends it back as ``have`` gets ``same_topology: true`` and no ``tri`` / ``tri_prim`` / ``edge`` arrays when
they have not changed (a deformer moved the points), so the viewer only replaces positions.
"""
from __future__ import annotations

import copy
import json
import struct
import threading
import weakref
from collections import OrderedDict
from typing import Dict, NamedTuple, Tuple

import numpy as np

from . import proxy, rust

MAGIC = b"MHPV"
# The editor's budget: more than this is reduced by :func:`proxy.decimate` (the whole shape, coarser), not cut off. A modern GPU draws
# this many triangles at interactive rates; the Perf menu's display budget lowers it for weaker ones.
DISPLAY_TRIS = 4_000_000
DISPLAY_POINTS = 4_000_000
# "Full resolution" on request: the most the viewer is asked to hold (past this it reduces too, so a request can never hang the browser)
MAX_TRIS = 16_000_000
MAX_EDGES = 32_000_000
MAX_POINTS = 16_000_000
# Instances are drawn as impostors (see :mod:`minihou.impostors`) from this many triangles on; the Blender link uses it the same way.
INSTANCE_TRIS = 250_000
INSTANCE_POINTS = 500_000
EXACT_PRIMS = 100_000            # up to this many primitives, polygons with more than 3 corners are triangulated exactly (concave ones too)
CHUNK_MIN_TRIS = 100_000         # from this many triangles on, they are sorted along a space-filling curve and sent with a chunk table
CHUNK_TRIS = 8192                # triangles per chunk


def _rust_on() -> bool:
    return rust.enabled()


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
    fast = _fan_numba(t)                                             # a big mesh: fans (concave polygons may show a wrong diagonal)
    if fast is not None:
        return fast
    tri_v, owner = proxy.fan_triangles(geo)
    return vp[tri_v], owner.astype(np.int64)


def _fan_numba(t):
    """:func:`proxy.fan_triangles` as point triples, in one parallel pass; None without numba."""
    from . import accel
    k = accel.kernels()
    if k is None:
        return None
    offs = np.ascontiguousarray(t.prim_offsets, np.int64)
    closed = np.ascontiguousarray(t.closed_mask(), np.uint8)
    counts = np.where(closed.astype(bool), np.maximum(np.diff(offs) - 2, 0), 0)
    first = np.zeros(len(counts), np.int64)
    np.cumsum(counts[:-1], out=first[1:])
    total = int(counts.sum())
    tri, owner = np.empty((total, 3), np.int64), np.empty(total, np.int64)
    if accel.call(k.fan_point_triangles, np.ascontiguousarray(t.vertex_point, np.int64), offs, closed, first, tri, owner) is accel.FAILED:
        return None
    return tri, owner


def _spread_bits(v: np.ndarray) -> np.ndarray:
    v = (v | (v << 16)) & np.uint32(0x030000FF)
    v = (v | (v << 8)) & np.uint32(0x0300F00F)
    v = (v | (v << 4)) & np.uint32(0x030C30C3)
    v = (v | (v << 2)) & np.uint32(0x09249249)
    return v


def _chunk_numba(P, tri, tri_prim):
    """:func:`chunk_triangles` with the numba kernels (linear time; the order is a stable sort of the same kind of Morton code), or None."""
    from . import accel
    k = accel.kernels()
    if k is None or not len(tri):
        return None
    P = np.ascontiguousarray(P)
    tri = np.ascontiguousarray(tri, np.int64).reshape(-1, 3)
    used = P[: max(int(tri.max()) + 1, 1)]
    lo = used.min(axis=0).astype(np.float64)
    ext = used.max(axis=0).astype(np.float64) - lo
    scale = 1023.0 / np.maximum(ext, 1e-12)
    code = np.empty(len(tri), np.int64)
    order = np.empty(len(tri), np.int64)
    if accel.call(k.tri_morton, P, tri, lo, scale, code) is accel.FAILED or accel.call(k.radix_order30, code, order) is accel.FAILED:
        return None
    tri_s, tp_s = np.empty_like(tri), np.empty(len(order), np.asarray(tri_prim).dtype)
    if accel.call(k.gather_rows3, tri, order, tri_s) is accel.FAILED or accel.call(k.gather1, np.ascontiguousarray(tri_prim), order, tp_s) is accel.FAILED:
        return None
    tri, tri_prim = tri_s, tp_s
    table = np.empty(((len(tri) + CHUNK_TRIS - 1) // CHUNK_TRIS, 8), np.float32)
    if accel.call(k.chunk_bounds, P, tri, CHUNK_TRIS, table) is accel.FAILED:
        return None
    return tri, tri_prim, table


def chunk_triangles(P: np.ndarray, tri: np.ndarray, tri_prim: np.ndarray) -> Tuple[np.ndarray, np.ndarray, np.ndarray]:
    """``(tri, tri_prim, table)``: the triangles sorted along a Morton curve of their centres, so triangles that are near each other are near
    in the list too, and a table with one row per run of ``CHUNK_TRIS`` of them: first triangle, triangle count, bounding box min xyz, max xyz
    (float32; the viewer skips the runs that are outside its view). The triangles are the same, only in another order."""
    n = len(tri)
    fast = _chunk_numba(P, tri, tri_prim)
    if fast is not None:
        return fast
    corners = P[tri.reshape(-1)].reshape(n, 3, 3)
    c = corners.mean(axis=1)
    lo, hi = c.min(axis=0), c.max(axis=0)
    q = np.minimum(((c - lo) / np.maximum(hi - lo, 1e-12) * 1023.0).astype(np.uint32), np.uint32(1023))
    code = _spread_bits(q[:, 0]) | (_spread_bits(q[:, 1]) << 1) | (_spread_bits(q[:, 2]) << 2)
    order = np.argsort(code, kind="stable")
    tri, tri_prim, corners = tri[order], tri_prim[order], corners[order]
    starts = np.arange(0, n, CHUNK_TRIS)
    counts = np.minimum(starts + CHUNK_TRIS, n) - starts
    tmin = np.minimum.reduceat(corners.min(axis=1), starts, axis=0)
    tmax = np.maximum.reduceat(corners.max(axis=1), starts, axis=0)
    table = np.concatenate([starts[:, None], counts[:, None], tmin, tmax], axis=1).astype(np.float32)
    return tri, tri_prim, table


class _Layout(NamedTuple):
    tri: np.ndarray          # uint32, 3 per triangle, in drawing (chunk) order
    tri_prim: np.ndarray     # uint32
    edge: np.ndarray         # uint32, 2 per edge
    chunked: bool
    truncated: bool


_LAYOUTS: "OrderedDict[str, _Layout]" = OrderedDict()                 # topology token -> layout; the last few (they can be large)
_LAYOUT_KEEP = 3


def _layout_get(token):
    if token is None:
        return None
    key = (token, MAX_TRIS, MAX_EDGES, CHUNK_MIN_TRIS, CHUNK_TRIS)
    with _MEMO_LOCK:
        lay = _LAYOUTS.get(key)
        if lay is not None:
            _LAYOUTS.move_to_end(key)
        return lay


def _layout_put(token, lay):
    if token is None:
        return
    with _MEMO_LOCK:
        _LAYOUTS[(token, MAX_TRIS, MAX_EDGES, CHUNK_MIN_TRIS, CHUNK_TRIS)] = lay
        while len(_LAYOUTS) > _LAYOUT_KEEP:
            _LAYOUTS.popitem(last=False)


def chunk_table(P: np.ndarray, tri: np.ndarray) -> np.ndarray:
    """The run table of :func:`chunk_triangles` for triangles already in order (bounds from the current positions)."""
    n = len(tri)
    if _rust_on() and n:
        return rust.chunk_bounds(P, tri, CHUNK_TRIS)
    from . import accel
    k = accel.kernels()
    if k is not None and n:
        table = np.empty(((n + CHUNK_TRIS - 1) // CHUNK_TRIS, 8), np.float32)
        if accel.call(k.chunk_bounds, np.ascontiguousarray(P), np.ascontiguousarray(tri, np.int64), CHUNK_TRIS, table) is not accel.FAILED:
            return table
    corners = np.asarray(P)[np.asarray(tri, np.int64).reshape(-1)].reshape(n, 3, 3)
    starts = np.arange(0, n, CHUNK_TRIS)
    counts = np.minimum(starts + CHUNK_TRIS, n) - starts
    tmin = np.minimum.reduceat(corners.min(axis=1), starts, axis=0)
    tmax = np.maximum.reduceat(corners.max(axis=1), starts, axis=0)
    return np.concatenate([starts[:, None], counts[:, None], tmin, tmax], axis=1).astype(np.float32)


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
    if full:
        pt_cap = MAX_POINTS
    elif tris:
        pt_cap = min(MAX_POINTS, max(DISPLAY_POINTS, 2 * int(tris)))          # a bigger triangle budget lets the points grow with it
    else:
        pt_cap = DISPLAY_POINTS
    red = proxy.decimate(geo, tri_cap, pt_cap)
    shown = red.geo
    t = shown.topology
    n = shown.npoints
    out: Dict[str, np.ndarray] = {}
    out["P"] = np.ascontiguousarray(shown.P, np.float32).reshape(-1)
    # Triangles, their order and the edges depend only on the topology (unless the drawing is reduced, which clusters by position): a deformer
    # downstream (noise, transform, smooth...) shares the topology of its input, so they are kept per topology and only the run bounds are redone.
    token = f"{t.digest}:{n}" if not red.reduced and t.nprims else None
    lay = _layout_get(token)
    if lay is None and t.nprims > EXACT_PRIMS and _rust_on():
        tri, tri_prim, edge, table, truncated = rust.preview_layout(shown.P, t, MAX_TRIS, MAX_EDGES, CHUNK_MIN_TRIS, CHUNK_TRIS)
        if red.reduced and len(tri_prim):
            tri_prim = np.ascontiguousarray(red.prim_src[tri_prim], np.uint32)
        lay = _Layout(tri, tri_prim, edge, table is not None, truncated)
        _layout_put(token, lay)
    elif lay is None:
        tri, tri_prim = _triangles(shown)
        edge = topo.edges(shown).points.astype(np.int64) if t.nprims else np.zeros((0, 2), np.int64)
        truncated = False
        if len(tri) > MAX_TRIS:                                          # only if a caller asked for more than the cap
            tri, tri_prim, truncated = tri[:MAX_TRIS], tri_prim[:MAX_TRIS], True
        if len(edge) > MAX_EDGES:
            edge, truncated = edge[:MAX_EDGES], True
        if red.reduced:
            tri_prim = red.prim_src[tri_prim] if len(tri_prim) else tri_prim
        table = None
        if len(tri) >= CHUNK_MIN_TRIS:
            tri, tri_prim, table = chunk_triangles(shown.P, tri, tri_prim)
        lay = _Layout(np.ascontiguousarray(tri, np.uint32).reshape(-1), np.ascontiguousarray(tri_prim, np.uint32),
                      np.ascontiguousarray(edge, np.uint32).reshape(-1), table is not None, truncated)
        _layout_put(token, lay)
    else:
        table = chunk_table(shown.P, lay.tri.reshape(-1, 3)) if lay.chunked else None
    truncated = lay.truncated
    if table is not None:
        out["chunks"] = np.ascontiguousarray(table).reshape(-1)
    out["tri"], out["tri_prim"], out["edge"] = lay.tri, lay.tri_prim, lay.edge
    tri, edge = lay.tri.reshape(-1, 3), lay.edge.reshape(-1, 2)
    cd = shown.point.get("Cd")
    if cd is not None and cd.ndim == 2 and cd.shape[1] >= 3 and len(cd) == n:
        out["Cd"] = np.ascontiguousarray(np.clip(cd[:, :3], 0.0, 1.0), np.float32).reshape(-1)
    if red.reduced:
        out["point_src"] = np.ascontiguousarray(red.point_src, np.uint32)
    if geo.npoints and _rust_on():
        lo, hi = (x.astype(float).tolist() for x in rust.bounds(geo.P))
    elif geo.npoints:
        lo, hi = geo.P.min(0).astype(float).tolist(), geo.P.max(0).astype(float).tolist()
    else:
        lo = hi = [0.0, 0.0, 0.0]
    head = {"points": int(n0), "prims": int(geo.topology.nprims), "tris": int(len(tri)), "edges": int(len(edge)),
            "bounds": [lo, hi], "truncated": truncated, "topology": token,
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
