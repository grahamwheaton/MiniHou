"""Geometry that comes from outside the graph: Blender objects streamed over the live link.

The engine keeps the newest version of each object the graph asked for (``blender_object`` nodes). Each version gets a
fresh token; nodes fold it into their cache key, so new data recooks exactly the nodes that depend on it. Engine-only
(no bpy): the add-on reads meshes and sends them; :mod:`minihou.link.server` decodes them into this store.
"""
from __future__ import annotations

import threading
import uuid
from dataclasses import dataclass, field
from typing import Dict, List, Optional

from .geometry import Geometry


@dataclass
class ObjectData:
    geo: Optional[Geometry]
    matrix: Optional[List[float]] = None          # object -> world, row-major 4x4
    error: str = ""                               # why there is no geometry (missing object, not a mesh, ...)
    token: str = field(default_factory=lambda: uuid.uuid4().hex)
    info: dict = field(default_factory=dict)      # what was read: attribute names, vertex groups, timings


class ObjectInputs:
    def __init__(self):
        self._lock = threading.Lock()
        self._data: Dict[str, ObjectData] = {}

    def put(self, name: str, geo: Optional[Geometry], matrix=None, error: str = "", info: Optional[dict] = None) -> None:
        with self._lock:
            self._data[name] = ObjectData(geo.freeze() if geo is not None else None, matrix, error, info=info or {})

    def get(self, name: str) -> Optional[ObjectData]:
        with self._lock:
            return self._data.get(name)

    def token(self, name: str) -> str:
        d = self.get(name)
        return d.token if d is not None else "none"

    def names(self) -> List[str]:
        with self._lock:
            return sorted(self._data)

    def retain(self, names) -> None:
        """Forget objects no graph reads any more (the add-on resends everything watched on each change)."""
        keep = set(names)
        with self._lock:
            for n in [n for n in self._data if n not in keep]:
                del self._data[n]

    def clear(self) -> None:
        with self._lock:
            self._data.clear()


INPUTS = ObjectInputs()
