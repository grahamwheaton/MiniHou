"""A cache of cooked geometry on disk, so reopening a big scene (or restarting the engine) does not cook it again.

The in-memory cache is keyed by a Merkle hash of everything a node's result depends on (node type and version, its library
code, parameters at the current proxy level, the keys of its inputs). That key is just as good a file name: if a key is on disk, the
geometry in it is what cooking would produce. :class:`~minihou.evaluator.Evaluator` asks the disk before cooking a missing node,
so a hit also skips cooking everything upstream of it.

Only what is worth it is written: results that took at least ``min_ms`` to cook and that read back in well under half that time (a
box is cheaper to cook than to read; a small result of a slow node is the best catch), and never results that depend on something outside the graph that changes between runs (a Blender
object, a running simulation, geometry fed into a system), whose keys are not stable. Writes happen on a background thread; the
geometry is frozen, so nothing can change under it. A result whose detail attributes would not survive JSON unchanged is not
written, so a hit is always identical to a fresh cook. The folder is capped (least recently used results go first).

On disk: ``<folder>/<key[:2]>/<key>/`` holds ``meta.json`` and one ``.npy`` file per array; a result appears atomically (written to a
temporary folder, then renamed).
"""
from __future__ import annotations

import json
import logging
import os
import queue
import shutil
import threading
import time
from pathlib import Path
from typing import Dict, Optional

import numpy as np

from .geometry import AttrTable, Geometry, Topology

log = logging.getLogger("minihou.diskcache")

FORMAT = 1
DEFAULT_MAX_BYTES = 4 * 1024 ** 3
MIN_MS = 100.0
MIN_BYTES = 0
READ_BYTES_PER_MS = 300_000            # about what a disk gives back; a result is kept only if reading it is clearly quicker than cooking it
# node types whose results are not a pure function of their key
VOLATILE_TYPES = {"blender_object", "blender_image", "modifier_input", "rbd_solver", "solver", "input"}


def folder_size(path: Path) -> int:
    total = 0
    for dirpath, _dirs, files in os.walk(path):
        for f in files:
            try:
                total += os.path.getsize(os.path.join(dirpath, f))
            except OSError:
                pass
    return total


class DiskCache:
    def __init__(self, folder, max_bytes: int = DEFAULT_MAX_BYTES, min_ms: float = MIN_MS, min_bytes: int = MIN_BYTES):
        self.folder = Path(folder)
        self.folder.mkdir(parents=True, exist_ok=True)
        self.max_bytes, self.min_ms, self.min_bytes = int(max_bytes), float(min_ms), int(min_bytes)
        self.hits = self.misses = self.writes = self.skipped = 0
        self._lock = threading.Lock()
        self._queue: "queue.Queue" = queue.Queue()
        self._pending: set = set()
        self._size = folder_size(self.folder)
        self._thread = threading.Thread(target=self._run, name="minihou-diskcache", daemon=True)
        self._thread.start()

    # -- paths ---------------------------------------------------------------
    def path_for(self, key: str) -> Path:
        return self.folder / key[:2] / key

    def __contains__(self, key: str) -> bool:
        return (self.path_for(key) / "meta.json").is_file()

    # -- writing -------------------------------------------------------------
    def wanted(self, geo: Geometry, ms: float) -> bool:
        """Worth keeping: it took long enough to cook, and reading it back would take well under half that time (a small result
        of a slow node is the best catch; a huge result of a quick node costs more to read than to cook)."""
        if geo.nbytes < self.min_bytes:
            return False
        if self.min_ms <= 0:
            return True
        return ms >= self.min_ms and geo.nbytes / READ_BYTES_PER_MS < ms * 0.5

    def put(self, key: str, geo: Geometry, ms: float) -> bool:
        """Queue ``geo`` (frozen) for writing if it is worth keeping. Returns whether it was queued."""
        if not self.wanted(geo, ms):
            return False
        with self._lock:
            if key in self._pending or key in self:
                return False
            if not _detail_roundtrips(geo.detail):
                self.skipped += 1
                return False
            self._pending.add(key)
        self._queue.put((key, geo))
        return True

    def flush(self, timeout: float = 30.0) -> bool:
        """Wait until everything queued has been written."""
        end = time.monotonic() + timeout
        while time.monotonic() < end:
            with self._lock:
                if not self._pending:
                    return True
            time.sleep(0.01)
        return False

    def _run(self) -> None:
        while True:
            item = self._queue.get()
            if item is None:
                return
            key, geo = item
            try:
                self._write(key, geo)
            except Exception:  # noqa: BLE001 - a full disk or a permission problem must not stop cooking
                log.warning("could not write cache entry %s", key, exc_info=True)       # full disk, permissions: cooking carries on
            finally:
                with self._lock:
                    self._pending.discard(key)

    def _write(self, key: str, geo: Geometry) -> None:
        final = self.path_for(key)
        final.parent.mkdir(parents=True, exist_ok=True)
        tmp = final.parent / f".{key}.{os.getpid()}.{threading.get_ident()}"
        if tmp.exists():
            shutil.rmtree(tmp, ignore_errors=True)
        tmp.mkdir()
        t = geo.topology
        meta: Dict[str, object] = {"format": FORMAT, "npoints": geo.npoints, "detail": _json_detail(geo.detail),
                                   "tables": {}, "has_closed": t.closed is not None}
        np.save(tmp / "vertex_point.npy", t.vertex_point, allow_pickle=False)
        np.save(tmp / "prim_offsets.npy", t.prim_offsets, allow_pickle=False)
        if t.closed is not None:
            np.save(tmp / "closed.npy", t.closed, allow_pickle=False)
        for dom in ("point", "vertex", "prim"):
            names = []
            for i, (name, arr) in enumerate(geo.table(dom).items()):
                np.save(tmp / f"{dom}_{i}.npy", arr, allow_pickle=False)
                names.append(name)
            meta["tables"][dom] = names
        (tmp / "meta.json").write_text(json.dumps(meta), encoding="utf-8")
        size = folder_size(tmp)
        try:
            os.rename(tmp, final)
        except OSError:                                              # another process stored it first
            shutil.rmtree(tmp, ignore_errors=True)
            return
        with self._lock:
            self._size += size
            self.writes += 1
        self._evict()

    def _evict(self) -> None:
        if self._size <= self.max_bytes:
            return
        entries = []
        for shard in self.folder.iterdir():
            if not shard.is_dir():
                continue
            for d in shard.iterdir():
                if d.name.startswith("."):
                    continue
                try:
                    entries.append((d.stat().st_mtime, d))
                except OSError:
                    pass
        entries.sort(key=lambda e: e[0])
        for _mtime, d in entries:
            if self._size <= self.max_bytes * 0.9:
                break
            size = folder_size(d)
            shutil.rmtree(d, ignore_errors=True)
            with self._lock:
                self._size -= size

    # -- reading -------------------------------------------------------------
    def get(self, key: str) -> Optional[Geometry]:
        d = self.path_for(key)
        meta_file = d / "meta.json"
        try:
            meta = json.loads(meta_file.read_text(encoding="utf-8"))
            if meta.get("format") != FORMAT:
                raise ValueError("other format")
            geo = Geometry(int(meta["npoints"]))
            closed = np.load(d / "closed.npy", allow_pickle=False) if meta.get("has_closed") else None
            geo.topology = Topology(np.load(d / "vertex_point.npy", allow_pickle=False),
                                    np.load(d / "prim_offsets.npy", allow_pickle=False), closed)
            geo.vertex = AttrTable(geo.topology.nvertices)
            geo.prim = AttrTable(geo.topology.nprims)
            for dom in ("point", "vertex", "prim"):
                table = geo.table(dom)
                for i, name in enumerate(meta["tables"][dom]):
                    table[name] = np.load(d / f"{dom}_{i}.npy", allow_pickle=False)
            geo.detail = dict(meta.get("detail") or {})
            geo.validate()
            geo.freeze()
        except FileNotFoundError:
            with self._lock:
                self.misses += 1
            return None
        except Exception:  # noqa: BLE001 - a damaged entry is just a miss; drop it so it is cooked again
            log.warning("discarding unreadable cache entry %s (it will be cooked again)", key)
            shutil.rmtree(d, ignore_errors=True)
            with self._lock:
                self.misses += 1
            return None
        try:
            os.utime(d)                                              # recently used: evicted last
        except OSError:
            pass
        with self._lock:
            self.hits += 1
        return geo

    def clear(self) -> None:
        self.flush()
        shutil.rmtree(self.folder, ignore_errors=True)
        self.folder.mkdir(parents=True, exist_ok=True)
        with self._lock:
            self._size = 0

    def close(self) -> None:
        self.flush(5.0)
        self._queue.put(None)

    def stats(self) -> dict:
        with self._lock:
            return {"folder": str(self.folder), "bytes": self._size, "max_bytes": self.max_bytes, "hits": self.hits,
                    "misses": self.misses, "writes": self.writes, "skipped": self.skipped, "pending": len(self._pending)}


def _json_detail(detail: dict) -> dict:
    return json.loads(json.dumps(detail, allow_nan=False))


def _detail_roundtrips(detail: dict) -> bool:
    """True when the detail attributes come back from JSON exactly as they went in."""
    try:
        return _json_detail(detail) == detail and json.dumps(_json_detail(detail), sort_keys=True) == json.dumps(detail, sort_keys=True)
    except (TypeError, ValueError):
        return False
