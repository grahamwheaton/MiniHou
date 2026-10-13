"""Thread-safe plumbing between the socket thread and Blender's main thread.

**No bpy in this module** (so it is unit-tested outside Blender). The socket thread calls
:meth:`Inbox.push`; the main-thread timer calls :meth:`Inbox.take`.
"""
from __future__ import annotations

import threading
from collections import deque
from typing import Deque, Dict, List, Optional, Tuple

from .protocol import GeoUpdate, merge_updates


class Inbox:
    """Queue of messages from the engine. ``geo`` updates are merged while they wait, so a burst of
    slider changes costs one mesh write instead of one per cook."""

    def __init__(self):
        self._lock = threading.Lock()
        self._events: Deque[dict] = deque()
        self._geo: Optional[GeoUpdate] = None
        self._extras: Dict[str, GeoUpdate] = {}          # extra outputs (named Blender objects), merged per output
        self.geo_received = 0
        self.geo_merged = 0
        self.bytes_received = 0

    def push(self, header: dict, arrays: dict) -> None:
        """Called on the socket thread. Must not touch bpy."""
        if header.get("type") == "geo" and header.get("output") is not None:
            u = GeoUpdate.from_message(header, arrays)
            with self._lock:
                self.geo_received += 1
                self.bytes_received += u.nbytes
                old = self._extras.get(u.output)
                if old is not None:
                    self.geo_merged += 1
                self._extras[u.output] = u if old is None else merge_updates(old, u)
        elif header.get("type") == "geo":
            u = GeoUpdate.from_message(header, arrays)
            with self._lock:
                self.geo_received += 1
                self.bytes_received += u.nbytes
                if self._geo is None:
                    self._geo = u
                else:
                    self.geo_merged += 1
                    self._geo = merge_updates(self._geo, u)
        else:
            if header.get("type") == "scene_objects" and arrays:
                header = dict(header, _arrays=arrays)           # pieces mode: the meshes travel as arrays
            with self._lock:
                self._events.append(header)

    def take(self) -> Tuple[List[dict], Optional[GeoUpdate]]:
        """Called on the main thread. Returns (non-geometry events in order, merged geo update)."""
        with self._lock:
            events, self._events = list(self._events), deque()
            geo, self._geo = self._geo, None
        return events, geo

    def take_extras(self) -> Dict[str, GeoUpdate]:
        """Main thread: the merged updates of the extra outputs, by Blender object name."""
        with self._lock:
            extras, self._extras = self._extras, {}
        return extras

    def clear(self) -> None:
        with self._lock:
            self._events.clear()
            self._geo = None
            self._extras = {}


# Blender stores socket floats as float32, so a value written as 0.4 reads back as 0.4000000059604645
# (relative error ~6e-8). Anything tighter than this would mistake the round-trip for a user edit and
# forward it to the engine -- a phantom change that recooks the graph after every connect.
FLOAT_REL_TOL = 1e-6


def _close(a, b) -> bool:
    if isinstance(a, tuple) and isinstance(b, tuple):
        return len(a) == len(b) and all(_close(x, y) for x, y in zip(a, b))
    if isinstance(a, bool) or isinstance(b, bool):
        return a == b
    return abs(float(a) - float(b)) <= FLOAT_REL_TOL * max(1.0, abs(float(a)), abs(float(b)))


class ParamForwarder:
    """Decides which modifier values have to be sent to the engine.

    ``known`` holds, per exposed parameter, the value the engine is known to have: values the
    engine told us (interface messages) and values we successfully sent. :meth:`changes` is a pure
    diff against it. A depsgraph update caused by our own mesh write therefore finds nothing to
    forward -- that is what breaks the feedback loop.

    :meth:`commit` records delivered values and runs a circuit breaker: more than
    ``max_per_second`` forwarded values in one second means a loop (a person cannot do that), so
    forwarding stops (``tripped``) until :meth:`reset`.
    """

    def __init__(self, max_per_second: int = 120):
        self.known: Dict[str, object] = {}
        self.max_per_second = max_per_second
        self._sent_times: Deque[float] = deque()
        self.tripped = False

    def reset(self, values: Dict[str, object]) -> None:
        self.known = dict(values)
        self._sent_times.clear()
        self.tripped = False

    def changes(self, current: Dict[str, object]) -> Dict[str, object]:
        if self.tripped:
            return {}
        return {n: v for n, v in current.items() if n in self.known and not _close(v, self.known[n])}

    def commit(self, name: str, value: object, now: float) -> None:
        self.known[name] = value
        while self._sent_times and now - self._sent_times[0] > 1.0:
            self._sent_times.popleft()
        self._sent_times.append(now)
        if len(self._sent_times) > self.max_per_second:
            self.tripped = True
