"""Geometry that comes from outside the graph: Blender objects streamed over the live link.

The engine keeps the newest version of each object the graph asked for (``blender_object`` nodes). Each version gets a
fresh token; nodes fold it into their cache key, so new data recooks exactly the nodes that depend on it. Engine-only
(no bpy): the add-on reads meshes and sends them; :mod:`minihou.link.server` decodes them into this store.
"""
from __future__ import annotations

import contextlib
from contextvars import ContextVar
import threading
import uuid
from dataclasses import dataclass, field
from typing import Dict, List, Optional

import numpy as np

from .geometry import Geometry


_READ_ALLOWED = ContextVar("minihou_object_input_allowed", default=True)


def input_cache_scope():
    """Partition nested shared caches from legacy cooks that may read live input."""
    return "" if _READ_ALLOWED.get() else "modifier-no-object-inputs"


@contextlib.contextmanager
def without_object_inputs():
    """Block legacy live geometry access throughout a nested modifier cook.

    Context-local so concurrent legacy proxy cooks continue to work normally.
    Enforced at get/token, before a nested evaluator can reuse a live-input key.
    """
    token = _READ_ALLOWED.set(False)
    try:
        yield
    finally:
        _READ_ALLOWED.reset(token)


@dataclass
class ObjectData:
    geo: Optional[Geometry]
    matrix: Optional[List[float]] = None          # object -> world, row-major 4x4
    error: str = ""                               # why there is no geometry (missing object, not a mesh, ...)
    token: str = field(default_factory=lambda: uuid.uuid4().hex)
    info: dict = field(default_factory=dict)      # what was read: attribute names, vertex groups, timings


@dataclass
class ImageData:
    pixels: Optional["np.ndarray"]                # (H, W, 4) float32, rows from the bottom; None when there is none (see error)
    meta: dict = field(default_factory=dict)      # is_float, colorspace (the image's own), source file ...
    error: str = ""
    token: str = field(default_factory=lambda: uuid.uuid4().hex)


@dataclass
class CollectionData:
    objects: List[str]                            # names of the objects in the collection (children included), sorted
    error: str = ""                               # why there are none (no such collection)
    token: str = field(default_factory=lambda: uuid.uuid4().hex)


class ObjectInputs:
    def __init__(self):
        self._lock = threading.Lock()
        self._data: Dict[str, ObjectData] = {}
        self._images: Dict[str, ImageData] = {}
        self._collections: Dict[str, "CollectionData"] = {}

    # -- Blender collections -------------------------------------------------------------------------
    def put_collection(self, name: str, objects, error: str = "") -> None:
        with self._lock:
            members = [str(o) for o in objects]
            old = self._collections.get(name)
            if old is not None and old.objects == members and old.error == error:
                return                                      # unchanged (the add-on resends after every watch change): keep the token, no recook
            self._collections[name] = CollectionData(members, error)

    def get_collection(self, name: str) -> Optional["CollectionData"]:
        if not _READ_ALLOWED.get():
            raise ValueError("modifier graphs cannot read legacy Blender collections: feedback loop prevention")
        with self._lock:
            return self._collections.get(name)

    def collection_token(self, name: str) -> str:
        d = self.get_collection(name)
        return d.token if d is not None else "none"

    def retain_collections(self, names) -> None:
        keep = set(names)
        with self._lock:
            for n in [n for n in self._collections if n not in keep]:
                del self._collections[n]

    # -- Blender images ------------------------------------------------------------------------------
    def put_image(self, name: str, pixels, meta: Optional[dict] = None, error: str = "") -> None:
        if pixels is not None:
            pixels = np.ascontiguousarray(pixels, np.float32)
            pixels.setflags(write=False)
        with self._lock:
            self._images[name] = ImageData(pixels, dict(meta or {}), error)

    def get_image(self, name: str) -> Optional[ImageData]:
        if not _READ_ALLOWED.get():
            raise ValueError("modifier graphs cannot read legacy Blender image inputs: feedback loop prevention")
        with self._lock:
            return self._images.get(name)

    def image_token(self, name: str) -> str:
        d = self.get_image(name)
        return d.token if d is not None else "none"

    def retain_images(self, names) -> None:
        keep = set(names)
        with self._lock:
            for n in [n for n in self._images if n not in keep]:
                del self._images[n]

    def put(self, name: str, geo: Optional[Geometry], matrix=None, error: str = "", info: Optional[dict] = None) -> None:
        with self._lock:
            self._data[name] = ObjectData(geo.freeze() if geo is not None else None, matrix, error, info=info or {})

    def get(self, name: str) -> Optional[ObjectData]:
        if not _READ_ALLOWED.get():
            raise ValueError("modifier graphs cannot read legacy Blender object inputs: feedback loop prevention")
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
            self._images.clear()
            self._collections.clear()


INPUTS = ObjectInputs()
