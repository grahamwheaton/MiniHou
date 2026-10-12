"""Per-client change detection: decide which arrays to send.

Each connected client has a :class:`DiffState` remembering the arrays it was last sent.
:meth:`DiffState.build` compares a freshly cooked geometry against that and produces the
``geo`` message header plus *only the changed* arrays:

* an array that is the same Python object as the last one sent is unchanged (free check --
  untouched attributes survive node passthrough as the same object);
* otherwise it is compared bytewise with the last one sent (cheap early-out on the first
  few thousand elements, so a genuinely changed array rarely needs a full scan);
* topology (``vertex_point`` + ``prim_offsets``) is sent only if either array changed.

Diffing at send time (not cook time) is what makes slider-drag conflation correct: if the
client was busy and three cooks happened, it is diffed against what it actually has.
"""
from __future__ import annotations

import time
from typing import Dict, Optional, Tuple

import numpy as np

from ..geometry import Geometry
from .protocol import TOPO_PRIM_CLOSED, TOPO_PRIM_OFFSETS, TOPO_VERTEX_POINT, attr_key

_HEAD = 4096


def arrays_equal(a: Optional[np.ndarray], b: Optional[np.ndarray]) -> bool:
    """Exact (bitwise) equality; same dtype and shape required. Arrays must be C-contiguous."""
    if a is b:
        return True
    if a is None or b is None or a.dtype != b.dtype or a.shape != b.shape:
        return False
    if a.size == 0:
        return True
    av = a.reshape(-1).view(np.uint8)
    bv = b.reshape(-1).view(np.uint8)
    head = _HEAD * a.dtype.itemsize
    if av.size > head and not np.array_equal(av[:head], bv[:head]):
        return False                      # cheap early-out for the common "everything moved" case
    return np.array_equal(av, bv)


def _json_detail(detail: dict) -> dict:
    out = {}
    for k, v in detail.items():
        if isinstance(v, np.ndarray):
            out[k] = v.tolist()
        elif isinstance(v, (np.generic,)):
            out[k] = v.item()
        elif isinstance(v, (int, float, str, bool, list, tuple, type(None))):
            out[k] = v
    return out


class DiffState:
    def __init__(self):
        self.reset()

    def reset(self) -> None:
        """Forget everything: the next build is a full update."""
        self._sent: Dict[str, np.ndarray] = {}

    def build(self, geo: Geometry, seq: int, cook: Optional[dict] = None,
              output: Optional[str] = None) -> Tuple[dict, Dict[str, np.ndarray]]:
        full = not self._sent
        current: Dict[str, np.ndarray] = {}
        manifest = {}
        for dom in ("point", "vertex", "prim"):
            for name, arr in geo.table(dom).items():
                key = attr_key(dom, name)
                current[key] = arr
                manifest[key] = {"dtype": arr.dtype.str, "shape": list(arr.shape)}
        topo = {TOPO_VERTEX_POINT: geo.topology.vertex_point, TOPO_PRIM_OFFSETS: geo.topology.prim_offsets}
        if geo.topology.has_open:                       # only geometry with curves pays for the flags
            topo[TOPO_PRIM_CLOSED] = geo.topology.closed.astype(np.uint8)

        send: Dict[str, np.ndarray] = {}
        topo_changed = (full or any(not arrays_equal(self._sent.get(k), v) for k, v in topo.items())
                        or (TOPO_PRIM_CLOSED in self._sent) != (TOPO_PRIM_CLOSED in topo))
        if topo_changed:
            send.update(topo)
        for key, arr in current.items():
            if not arrays_equal(self._sent.get(key), arr):
                send[key] = arr

        if topo_changed and TOPO_PRIM_CLOSED not in topo and TOPO_PRIM_CLOSED in self._sent:
            send[TOPO_PRIM_CLOSED] = np.ones(0, np.uint8)   # curves gone: tell the client (empty = all polygons)
        # Remember what the client will have after this message.
        new_sent = dict(current)
        new_sent.update(topo)
        self._sent = new_sent

        header = {"type": "geo", "seq": seq, "full": full,
                  "counts": {"point": geo.npoints, "vertex": geo.nvertices, "prim": geo.nprims},
                  "topology_changed": topo_changed, "manifest": manifest,
                  "detail": _json_detail(geo.detail), "cook": cook or {}, "t_sent": time.time()}
        if output is not None:
            header["output"] = output
        return header, send
