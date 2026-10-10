"""A live engine session: one graph, one evaluator, one cook thread.

Socket-free on purpose (the :class:`~minihou.link.server.LinkServer` is a thin layer on top),
so it can be driven and tested directly.

* ``set_param`` edits the in-memory graph and wakes the cook thread.
* The cook thread coalesces: if ten edits arrive during one cook, the next cook sees the
  latest state once.
* Results are published to listeners (``on_result`` / ``on_interface``) on the cook thread.
* ``watch_file`` hot-reloads the graph JSON when it changes on disk. The evaluator cache is
  content-addressed, so unchanged nodes stay warm across reloads.
"""
from __future__ import annotations

import hashlib
import json
import logging
import os
import threading
import time
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Dict, List, Optional

from ..evaluator import Cache, DEFAULT_CACHE_BYTES, Evaluator, NodeError
from ..geometry import Geometry
from ..graph import Graph, GraphError
from ..sdk import ParamError, Registry

log = logging.getLogger("minihou.session")


@dataclass
class Result:
    seq: int
    geo: Optional[Geometry] = None
    key: str = ""
    cook: Dict[str, Any] = field(default_factory=dict)
    error: Optional[Dict[str, Any]] = None       # {"node": id-or-None, "message": str}


class Session:
    def __init__(self, graph: Graph, *, registry: Optional[Registry] = None, cache_bytes: int = DEFAULT_CACHE_BYTES):
        self.graph = graph
        self.evaluator = Evaluator(registry or graph.registry, Cache(cache_bytes))
        self.lock = threading.RLock()
        self.latest_good: Optional[Result] = None     # newest successful cook
        self.latest_error: Optional[Result] = None    # set while the graph is failing
        self._listeners: List[Any] = []
        self._rev = 0                # bumped on every edit
        self._cooked_rev = -1        # rev of the last completed cook
        self._seq = 0
        self._last_key = None
        self._last_error_msg = None
        self._wake = threading.Event()
        self._idle = threading.Condition()
        self._stop = threading.Event()
        self._thread: Optional[threading.Thread] = None
        self._watch_thread: Optional[threading.Thread] = None
        self._iface = self.interface()

    # -- interface ---------------------------------------------------------
    def interface(self) -> dict:
        with self.lock:
            params = self.graph.interface()
        structure = [{k: v for k, v in p.items() if k not in ("value",)} for p in params]
        h = hashlib.blake2b(json.dumps(structure, sort_keys=True).encode(), digest_size=8).hexdigest()
        return {"type": "interface", "structure": h, "params": params}

    # -- editing -----------------------------------------------------------
    def set_param(self, name: str, value: Any) -> Any:
        """Set an exposed parameter by name. Raises :class:`GraphError` / :class:`ParamError`."""
        with self.lock:
            v = self.graph.set_exposed(name, value)
            self._bump()
        return v

    def set_node_param(self, node: str, param: str, value: Any) -> Any:
        with self.lock:
            v = self.graph.set_param(node, param, value)
            self._bump()
        return v

    def replace_graph(self, graph: Graph) -> None:
        with self.lock:
            self.graph = graph
            self._bump()
            iface = self.interface()
            changed = iface != self._iface
            self._iface = iface
        if changed:
            for l in list(self._listeners):
                l.on_interface(iface)

    def _bump(self) -> None:
        self._rev += 1
        self._wake.set()

    # -- listeners ---------------------------------------------------------
    def add_listener(self, l) -> None:
        self._listeners.append(l)

    def remove_listener(self, l) -> None:
        if l in self._listeners:
            self._listeners.remove(l)

    # -- cooking -----------------------------------------------------------
    def _publish(self, res: Result) -> None:
        """Record and broadcast a result. ``latest_good`` always holds the newest geometry."""
        with self.lock:
            self._seq += 1
            res.seq = self._seq
            if res.error is None:
                self.latest_good = res
                self.latest_error = None
            else:
                self.latest_error = res
        for l in list(self._listeners):
            try:
                l.on_result(res)
            except Exception:  # noqa: BLE001 - a bad listener must not stop the cook thread
                log.exception("listener failed")

    def _report_error(self, node: Optional[str], message: str) -> None:
        if message == self._last_error_msg:
            return                                           # same failure again: don't spam clients
        self._last_error_msg = message
        self._publish(Result(0, error={"node": node, "message": message}))

    def cook_once(self) -> Optional[Result]:
        """Cook the current graph now and publish if the outcome changed. Returns the result
        that was published (or None if nothing changed)."""
        with self.lock:
            rev = self._rev
            snapshot = self.graph.copy()
        t0 = time.perf_counter()
        published: Optional[Result] = None
        try:
            geo = self.evaluator.cook(snapshot)
            stats = self.evaluator.last_stats
            self.evaluator.cache.retain(self.evaluator.live_keys(snapshot))
            if stats.key != self._last_key or self._last_error_msg is not None:
                d = stats.to_dict()
                cook = {"ms": round((time.perf_counter() - t0) * 1000, 3), "cooked": d["cooked"],
                        "cached": d["cached"], "node_ms": d["node_ms"]}
                published = Result(0, geo, stats.key, cook)
                self._last_key, self._last_error_msg = stats.key, None
                self._publish(published)
        except (NodeError, GraphError, ParamError) as e:
            self._report_error(getattr(e, "node_id", None), str(e))
            log.warning("cook failed: %s", e)
        except Exception as e:  # noqa: BLE001 - never let the cook thread die
            log.exception("unexpected cook failure")
            self._report_error(None, f"internal error: {e!r}")
        with self._idle:
            self._cooked_rev = rev
            self._idle.notify_all()
        return published

    def start(self, cook_immediately: bool = True) -> None:
        if self._thread and self._thread.is_alive():
            return
        self._stop.clear()
        if cook_immediately:
            self._bump()
        self._thread = threading.Thread(target=self._run, name="minihou-cook", daemon=True)
        self._thread.start()

    def _run(self) -> None:
        while not self._stop.is_set():
            self._wake.wait(0.5)
            if self._stop.is_set():
                break
            if not self._wake.is_set():
                continue
            self._wake.clear()
            self.cook_once()

    def stop(self) -> None:
        self._stop.set()
        self._wake.set()
        for t in (self._thread, self._watch_thread):
            if t and t is not threading.current_thread():
                t.join(2.0)

    def wait_idle(self, timeout: float = 10.0) -> bool:
        """Block until every edit made so far has been cooked (for tests and scripts)."""
        end = time.monotonic() + timeout
        with self._idle:
            while self._cooked_rev < self._rev:
                left = end - time.monotonic()
                if left <= 0:
                    return False
                self._idle.wait(left)
        return True

    # -- file watching -----------------------------------------------------
    def watch_file(self, path, interval: float = 0.25) -> None:
        path = Path(path)

        def stamp():
            try:
                st = path.stat()
                return (st.st_mtime_ns, st.st_size)
            except OSError:
                return None

        last = stamp()

        def loop():
            nonlocal last
            while not self._stop.wait(interval):
                cur = stamp()
                if cur is None or cur == last:
                    continue
                last = cur
                try:
                    g = Graph.load(path, self.evaluator.registry)
                except (GraphError, OSError, ValueError) as e:
                    log.warning("graph reload failed: %s: %s", path.name, e)
                    self._report_error(None, f"{path.name}: {e}")
                    continue
                log.info("reloaded %s", path.name)
                self.replace_graph(g)

        self._watch_thread = threading.Thread(target=loop, name="minihou-watch", daemon=True)
        self._watch_thread.start()
