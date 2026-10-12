"""A live engine session: one graph, one evaluator, one cook thread.

Socket-free on purpose (the :class:`~minihou.link.server.LinkServer` and the web/MCP API are thin layers on
top), so it can be driven and tested directly.

* ``set_param`` (Blender) and ``edit`` (editor, MCP: batches of :mod:`minihou.edit` ops) change the in-memory
  graph and wake the cook thread. Every change bumps ``rev``.
* The cook thread coalesces: if ten edits arrive during one cook, the next cook sees the latest state once.
* Results are published to listeners (``on_result`` / ``on_interface`` / ``on_graph``) on the calling thread.
* ``watch_file`` binds the session to a graph file: external changes are hot-reloaded, and edits are saved back
  (debounced, atomically, and without reloading our own writes). The evaluator cache is content-addressed, so
  unchanged nodes stay warm across reloads.
* ``undo``/``redo`` cover ``edit`` batches (consecutive edits of the same thing, like a slider drag, merge).
"""
from __future__ import annotations

import hashlib
import json
import logging
import math
import os
import threading
import time
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Dict, List, Optional, Tuple

import numpy as np

from ..edit import apply_ops, layout_missing
from ..evaluator import Cache, CookCancelled, DEFAULT_CACHE_BYTES, Evaluator, NodeError
from ..geometry import Geometry
from ..graph import Graph, GraphError
from .. import proxy, timectx
from ..inputs import INPUTS
from ..sdk import ParamError, Registry
from ..flow import attribute_flow
from ..handles import has_handles, handles_for
from ..summary import summarize
from ..wrangle import compile_snippet

log = logging.getLogger("minihou.session")

UNDO_LIMIT = 200
MERGE_SECONDS = 1.5          # consecutive edits of the same thing within this window form one undo step
REFINE_DELAY = 0.3           # auto proxy: how long edits must pause before the full cook starts

NEW_GRAPH = {"version": 1, "nodes": {"grid1": {"type": "grid", "pos": [0, 0]},
                                     "output1": {"type": "output", "inputs": {"geo": "grid1"}, "pos": [0, 110]}},
             "display": "output1"}


def snap_float32(v: Any) -> Any:
    """Values from Blender are float32: 0.22 arrives as 0.2199999988079071. A float that is exactly a float32 becomes
    the shortest decimal for that float32 (0.22), so graph files don't fill up with float32 noise. Other floats, ints
    and strings pass through unchanged; lists are handled element-wise."""
    if isinstance(v, float) and math.isfinite(v):
        f = np.float32(v)
        if float(f) == v:
            return float(str(f))                  # numpy prints the shortest repr that round-trips the float32
        return v
    if isinstance(v, (list, tuple)):
        return [snap_float32(x) for x in v]
    return v


@dataclass
class Result:
    seq: int
    geo: Optional[Geometry] = None
    key: str = ""
    cook: Dict[str, Any] = field(default_factory=dict)
    error: Optional[Dict[str, Any]] = None       # {"node": id-or-None, "message": str}
    rev: int = 0                                  # graph revision this result was cooked from
    proxy: float = 1.0                            # proxy level it was cooked at (1.0: the real thing)
    link_memo: Any = None                         # (link_tris, reduced geometry) for the Blender link
    extras: Dict[str, Tuple[Geometry, str]] = field(default_factory=dict)   # extra outputs: object name -> (geometry, key)


def _merge_key(ops: List[dict]) -> Optional[tuple]:
    """Edits that should collapse into one undo step when repeated (slider drags, node drags)."""
    if len(ops) != 1 or not isinstance(ops[0], dict):
        return None
    op = ops[0]
    if op.get("op") == "set" and isinstance(op.get("params"), dict):
        return ("set", op.get("id"), tuple(sorted(op["params"])))
    if op.get("op") == "move":
        return ("move", tuple(sorted(op.get("positions") or [op.get("id")])))
    return None


class Session:
    def __init__(self, graph: Graph, *, registry: Optional[Registry] = None, cache_bytes: int = DEFAULT_CACHE_BYTES):
        layout_missing(graph)
        self.graph = graph
        self.evaluator = Evaluator(registry or graph.registry, Cache(cache_bytes))
        self.lock = threading.RLock()
        self._cook_lock = threading.Lock()            # the evaluator is single-threaded
        self.latest_good: Optional[Result] = None     # newest successful cook
        self.latest_error: Optional[Result] = None    # set while the graph is failing
        self._listeners: List[Any] = []
        self._rev = 0                # bumped on every change of the graph
        self._document_id = 0        # fences delayed browser edits across file switches
        self._cooked_rev = -1        # rev of the last completed cook
        self._seq = 0
        self._last_key = None
        self._last_error_msg = None
        self._extra_errors: Dict[str, str] = {}
        self._wake = threading.Event()
        self._idle = threading.Condition()
        self._stop = threading.Event()
        self._thread: Optional[threading.Thread] = None
        self._watch_thread: Optional[threading.Thread] = None
        self._undo: List[tuple] = []                  # (graph dict, merge key, time)
        self._redo: List[dict] = []
        self.path: Optional[Path] = None
        self._dirty = False                           # in-memory graph differs from the file
        self._file_text: Optional[str] = None         # last text read from / written to the file
        self.save_error: Optional[str] = None
        self.library = None                           # minihou.library.NodeLibrary, if attached
        self._watched: List[str] = []
        self.frame = 1.0                 # current time (Blender's frame), for @Frame / @Time
        self.fps = timectx.DEFAULT_FPS
        self.proxy_level = 1.0           # cook proxy: node parameters marked as proxy-able scale with it (1.0 = full)
        self.proxy_auto = False          # auto: slow scenes cook at the proxy level first and refine to full when edits pause
        self.link_tris = 0               # Blender link: reduce geometry above this many triangles (0 = send everything)
        self._full_ms: Optional[float] = None   # how long the last full-level cook that did work took
        self._refining = False
        self._save_enabled = False
        self._stamp = None
        self._file_lock = threading.RLock()           # saving vs. noticing external changes
        self._iface = self.interface()

    @property
    def rev(self) -> int:
        return self._rev

    # -- interface ---------------------------------------------------------
    def interface(self) -> dict:
        with self.lock:
            params = self.graph.interface()
        structure = [{k: v for k, v in p.items() if k not in ("value",)} for p in params]
        h = hashlib.blake2b(json.dumps(structure, sort_keys=True).encode(), digest_size=8).hexdigest()
        return {"type": "interface", "structure": h, "params": params}

    def snapshot(self) -> dict:
        """The graph as JSON plus its revision (what editors render)."""
        with self.lock:
            return {"rev": self._rev, "document_id": self._document_id, "graph": self.graph.to_dict(),
                    "path": str(self.path.resolve()) if self.path else None,        # resolved: the editor compares it with the (resolved) root
                    "can_undo": bool(self._undo), "can_redo": bool(self._redo)}

    # -- editing -----------------------------------------------------------
    def set_param(self, name: str, value: Any, origin: str = "link") -> Any:
        """Set an exposed parameter by name. Raises :class:`GraphError` / :class:`ParamError`."""
        if origin == "link":
            value = snap_float32(value)
        with self.lock:
            v = self.graph.set_exposed(name, value)
            self._iface = self.interface()            # the link client already has this value: don't echo it
            self._changed()
        self._notify_graph(origin)
        return v

    def set_node_param(self, node: str, param: str, value: Any, origin: str = "link") -> Any:
        if origin == "link":
            value = snap_float32(value)
        with self.lock:
            v = self.graph.set_param(node, param, value)
            self._iface = self.interface()
            self._changed()
        self._notify_graph(origin)
        return v

    def edit(self, ops: List[dict], origin: str = "api", document_id=None) -> dict:
        """Apply a batch of :mod:`minihou.edit` ops atomically. Raises :class:`~minihou.edit.EditError`.

        Returns ``{"rev", "results", "changed"}``. Interface changes (including exposed values) are pushed to link
        clients, so Blender follows edits made in the editor or by an AI."""
        key = _merge_key(ops)
        with self.lock:
            self._check_document(document_id)
            before = self.graph.to_dict()
            new, results = apply_ops(self.graph, ops)
            changed = new.to_dict() != before
            if changed:
                now = time.monotonic()
                top = self._undo[-1] if self._undo else None
                if not (key and top and top[1] == key and now - top[2] < MERGE_SECONDS):
                    self._undo.append((before, key, now))
                    del self._undo[:-UNDO_LIMIT]
                else:
                    self._undo[-1] = (top[0], key, now)
                self._redo.clear()
                self.graph = new
                self._changed()
            rev = self._rev
        if changed:
            self._after_structural_change(origin)
        return {"rev": rev, "results": results, "changed": changed}

    def _check_document(self, document_id):
        if document_id is not None and document_id != self._document_id:
            raise GraphError("Editor graph changed; this edit belongs to the previously open graph")

    def undo(self, origin: str = "api", document_id=None) -> bool:
        return self._step(self._undo, self._redo, origin, document_id)

    def redo(self, origin: str = "api", document_id=None) -> bool:
        return self._step(self._redo, self._undo, origin, document_id)

    def _step(self, src: list, dst: list, origin: str, document_id=None) -> bool:
        with self.lock:
            self._check_document(document_id)
            if not src:
                return False
            entry = src.pop()
            data = entry[0] if isinstance(entry, tuple) else entry
            cur = self.graph.to_dict()
            dst.append((cur, None, 0.0) if dst is self._undo else cur)
            self.graph = Graph.from_dict(data, self.graph.registry, check=False)
            self._changed()
        self._after_structural_change(origin)
        return True

    def replace_graph(self, graph: Graph, origin: str = "file", save: bool = False) -> None:
        layout_missing(graph)
        with self.lock:
            self.graph = graph
            self._rev += 1
            self._wake.set()
            if save:
                self._dirty = True
        self._after_structural_change(origin)

    def _changed(self) -> None:
        """Call with the lock held after mutating the graph."""
        self._rev += 1
        self._dirty = True
        self._wake.set()

    def _after_structural_change(self, origin: str) -> None:
        with self.lock:
            iface = self.interface()
            push = iface != self._iface
            self._iface = iface
        if push:
            for l in list(self._listeners):
                l.on_interface(iface)
        self._notify_graph(origin)

    # -- Blender objects as inputs ---------------------------------------------------------------------
    def watched_objects(self) -> List[str]:
        """Blender objects the graph reads (``blender_object`` nodes): the add-on streams exactly these."""
        with self.lock:
            return sorted({str(inst.params.get("object") or "") for inst in self.graph.nodes.values()
                           if inst.type == "blender_object"} - {""})

    def set_frame(self, frame: float, fps: Optional[float] = None) -> None:
        """The timeline moved (Blender sends this). Recooks only nodes that use @Frame / @Time; not a graph edit."""
        frame = float(frame)
        fps = float(fps) if fps else self.fps
        with self.lock:
            if frame == self.frame and fps == self.fps:
                return
            self.frame, self.fps = frame, fps
            self._rev += 1
            self._wake.set()

    def object_received(self, name: str, geo, matrix=None, error: str = "", info: Optional[dict] = None) -> None:
        """New data for a Blender object (or why there is none). Recooks what depends on it; not a graph edit."""
        INPUTS.put(name, geo, matrix, error, info)
        if name not in self.watched_objects():
            return                                         # stale (the graph changed meanwhile): stored, no recook
        with self.lock:
            self._rev += 1
            self._wake.set()

    def _check_watch(self) -> None:
        w = self.watched_objects()
        if w == self._watched:
            return
        self._watched = w
        INPUTS.retain(w)
        for l in list(self._listeners):
            fn = getattr(l, "on_watch", None)
            if fn:
                try:
                    fn(w)
                except Exception:  # noqa: BLE001
                    log.exception("listener failed")

    def _notify_graph(self, origin: str) -> None:
        self._check_watch()
        rev = self._rev
        for l in list(self._listeners):
            fn = getattr(l, "on_graph", None)
            if fn:
                try:
                    fn(rev, origin)
                except Exception:  # noqa: BLE001
                    log.exception("listener failed")

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

    def _extra_error(self, name: str, message: str) -> None:
        """An extra output failed to cook: say so once, keep the main result flowing."""
        if self._extra_errors.get(name) != message:
            self._extra_errors[name] = message
            log.warning("output %r failed: %s", name, message)
            for l in list(self._listeners):
                fn = getattr(l, "on_extra_error", None)
                if fn:
                    fn(name, message)

    def _report_error(self, node: Optional[str], message: str, rev: int = 0) -> None:
        if message == self._last_error_msg:
            return                                           # same failure again: don't spam clients
        self._last_error_msg = message
        self._publish(Result(0, error={"node": node, "message": message}, rev=rev))

    # -- the proxy ---------------------------------------------------------
    def set_proxy(self, level: Optional[float] = None, auto: Optional[bool] = None, link_tris: Optional[int] = None) -> dict:
        """Change the proxy settings and recook. ``level`` is the cook level (1 = full; ``auto`` cooks that level first when the
        full cook is slow, then the full one when edits pause); ``link_tris`` caps what Blender receives (0 = everything)."""
        with self.lock:
            changed = False
            if level is not None:
                lvl = proxy.clamp(level)
                changed |= lvl != self.proxy_level
                self.proxy_level = lvl
            if auto is not None:
                changed |= bool(auto) != self.proxy_auto
                self.proxy_auto = bool(auto)
            if link_tris is not None:
                n = max(0, int(link_tris))
                changed |= n != self.link_tris
                self.link_tris = n
            if changed:
                self._rev += 1
                self._wake.set()
        return self.proxy_info()

    def proxy_info(self) -> dict:
        good = self.latest_good
        return {"level": self.proxy_level, "auto": self.proxy_auto, "link_tris": self.link_tris, "levels": list(proxy.LEVELS),
                "cooked": good.proxy if good is not None else 1.0, "refining": self._refining,
                "full_ms": None if self._full_ms is None else round(self._full_ms, 1)}

    def export_usd(self, path, node: Optional[str] = None, overwrite: bool = False, embed_graph: bool = True) -> dict:
        """Write the cooked geometry to a USD file at full size, whatever the proxy level: the display output and every extra output
        (as Blender receives them), or just ``node``. Returns the counts written per output."""
        from .. import usd_io
        with self.lock:
            snapshot = self.graph.copy()
            frame, fps = self.frame, self.fps
        if node is not None and node not in snapshot.nodes:
            raise GraphError(f"no such node {node!r}")
        if not usd_io.available():
            raise usd_io.UsdUnavailable("USD support needs the 'usd-core' package: pip install usd-core")
        outputs: Dict[str, Geometry] = {}
        with self._cook_lock, timectx.at(frame, fps), proxy.using(1.0):
            ev = self.evaluator
            saved, ev.proxy_level = ev.proxy_level, None             # ambient level 1.0: never a proxy cook, never the link's reduction
            try:
                if node is not None:
                    outputs[node] = ev.cook(snapshot, node)
                else:
                    outputs[snapshot.output_node()] = ev.cook(snapshot)
                    for name, nid in snapshot.extra_outputs().items():
                        outputs[name] = ev.cook(snapshot, nid)
            finally:
                ev.proxy_level = saved
        rep = usd_io.write_stage(path, outputs, graph=snapshot.to_dict() if embed_graph else None, overwrite=overwrite, frame=frame)
        out = rep.to_dict()
        out["frame"] = frame
        return out

    def view_level(self) -> float:
        """The proxy level the editor is looking at: the settings' level, or in auto mode the level of the newest result."""
        good = self.latest_good
        return good.proxy if (self.proxy_auto and good is not None) else self.proxy_level

    def link_geometry(self, res: Result) -> Geometry:
        """What the Blender link sends for ``res``: the geometry, or its reduction when ``link_tris`` is set and it is bigger."""
        geo = res.geo
        cap = self.link_tris
        if not cap or geo is None:
            return geo
        memo = res.link_memo
        if memo is not None and memo[0] == cap:
            return memo[1]
        red = proxy.decimate(geo, cap, 2 * cap)
        res.link_memo = (cap, red.geo)
        return red.geo

    # -- cooking -----------------------------------------------------------
    def cook_once(self) -> Optional[Result]:
        """Cook the current graph now and publish if the outcome changed. Returns the result
        that was published (or None if nothing changed). In auto proxy mode a slow scene is cooked at the proxy level first
        (published at once) and then at full level once edits pause."""
        with self.lock:
            rev = self._rev
            snapshot = self.graph.copy()
            frame, fps = self.frame, self.fps
            lvl, auto = self.proxy_level, self.proxy_auto
        if not auto:
            plan = [lvl]
        elif self._full_ms is not None and self._full_ms <= proxy.AUTO_SLOW_MS and lvl >= 1.0:
            plan = [1.0]                                  # a fast scene: nothing to gain from a proxy
        else:
            coarse = lvl if lvl < 1.0 else proxy.AUTO_LEVEL
            plan = [1.0] if self._same_key(snapshot, frame, fps, coarse, 1.0) else [coarse, 1.0]     # nothing scales: one pass
        published: Optional[Result] = None
        for step, L in enumerate(plan):
            if step:                                      # refine: only when no edit follows and the proxy was not already full
                if self._wake.wait(REFINE_DELAY) or self._rev != rev:
                    break
                if self._same_key(snapshot, frame, fps, plan[0], L):
                    break
                self._refining = True
                self._notify_status(rev)
            status, res = self._cook_pass(rev, snapshot, frame, fps, L, cancelable=step > 0)
            published = res or published
            if status != "ok":
                break
        self._refining = False
        with self._idle:
            self._cooked_rev = rev
            self._idle.notify_all()
        self._notify_status(rev)
        return published

    def _notify_status(self, rev: int) -> None:
        for l in list(self._listeners):                      # cook finished (even if nothing new was published)
            fn = getattr(l, "on_status", None)
            if fn:
                try:
                    fn(rev)
                except Exception:  # noqa: BLE001
                    log.exception("listener failed")

    def _same_key(self, snapshot: Graph, frame, fps, a: float, b: float) -> bool:
        """True when the graph's result is the same at proxy levels ``a`` and ``b`` (nothing in it depends on the level)."""
        try:
            with self._cook_lock, timectx.at(frame, fps):
                keys = []
                for L in (a, b):
                    with proxy.using(L):
                        keys.append(self.evaluator.compute_keys(snapshot, snapshot.upstream_order(snapshot.output_node()))[snapshot.output_node()])
            return keys[0] == keys[1]
        except Exception:  # noqa: BLE001 - if keys cannot be computed the cook itself will say why
            return False

    def _cook_pass(self, rev: int, snapshot: Graph, frame, fps, level: float, cancelable: bool = False):
        """One cook at ``level``. Returns ("ok" | "error" | "cancelled", the Result it published or None)."""
        t0 = time.perf_counter()
        published: Optional[Result] = None
        ev = self.evaluator
        try:
            extras: Dict[str, Tuple[Geometry, str]] = {}
            with self._cook_lock, timectx.at(frame, fps):
                ev.proxy_level = level
                ev.cancel = (lambda: self._rev != rev or self._wake.is_set()) if cancelable else None
                try:
                    geo = ev.cook(snapshot)
                    stats = ev.last_stats
                    live = ev.live_keys(snapshot)
                    for name, nid in snapshot.extra_outputs().items():     # more outputs: each its own Blender object
                        try:
                            eg = ev.cook(snapshot, nid)
                        except (NodeError, GraphError, ParamError) as e:
                            self._extra_error(name, f"{nid}: {e}")
                            continue
                        extras[name] = (eg, ev.last_stats.key)
                        live += ev.live_keys(snapshot, nid)
                    ev.cache.retain(live)
                finally:
                    ev.proxy_level, ev.cancel = None, None
            ms = (time.perf_counter() - t0) * 1000
            if level >= 1.0 and stats.cooked:
                self._full_ms = ms
            overall = stats.key + "".join(f"|{n}:{k}" for n, (_, k) in sorted(extras.items()))
            if overall != self._last_key or self._last_error_msg is not None:
                d = stats.to_dict()
                cook = {"ms": round(ms, 3), "cooked": d["cooked"], "cached": d["cached"], "node_ms": d["node_ms"]}
                if level < 1.0:
                    cook["proxy"] = level
                published = Result(0, geo, stats.key, cook, rev=rev, extras=extras, proxy=level)
                self._last_key, self._last_error_msg = overall, None
                self._publish(published)
            elif self.latest_good is not None:
                self.latest_good.rev = rev                   # same geometry, newer graph (e.g. a node was moved)
            return "ok", published
        except CookCancelled:
            return "cancelled", None
        except (NodeError, GraphError, ParamError) as e:
            self._report_error(getattr(e, "node_id", None), str(e), rev)
            log.warning("cook failed: %s", e)
            return "error", None
        except Exception as e:  # noqa: BLE001 - never let the cook thread die
            log.exception("unexpected cook failure")
            self._report_error(None, f"internal error: {e!r}", rev)
            return "error", None

    def status(self) -> dict:
        """Outcome of the newest cook: ok/error, counts, timings, and whether it is up to date."""
        with self.lock:
            good, err, rev = self.latest_good, self.latest_error, self._rev
        out: Dict[str, Any] = {"rev": rev, "cooked_rev": self._cooked_rev, "busy": self._cooked_rev < rev,
                               "frame": self.frame, "fps": self.fps}
        out["proxy"] = self.proxy_info()
        if err is not None:
            out.update(ok=False, error=err.error)
        else:
            out["ok"] = good is not None
        if good is not None and good.geo is not None:
            g = good.geo
            out.update(points=g.npoints, prims=g.nprims, vertices=g.nvertices, cook=good.cook)
        return out

    def inspect(self, node: str, detail: bool = False) -> dict:
        """Cook ``node`` (usually from cache) and summarise its geometry. Errors are returned, not raised."""
        with self.lock:
            snapshot = self.graph.copy()
            rev = self._rev
            frame, fps = self.frame, self.fps
        if node not in snapshot.nodes:
            return {"node": node, "rev": rev, "ok": False, "error": {"node": node, "message": f"no such node {node!r}"}}
        t0 = time.perf_counter()
        try:
            with self._cook_lock, timectx.at(frame, fps), proxy.using(self.view_level()):
                sources = list(dict.fromkeys(snapshot.input_ids(node)))
                if snapshot.nodes[node].type == "loop_end":
                    # A repeat body cannot cook outside its iteration context. Compare
                    # the completed loop against its initial input for attribute flow.
                    begin = snapshot.nodes[node].inputs.get("begin")
                    sources = list(snapshot.input_ids(begin)) if begin in snapshot.nodes else []
                ins = [self.evaluator.cook(snapshot, src) for src in sources if src in snapshot.nodes]
                geo = self.evaluator.cook(snapshot, node)
                stats = self.evaluator.last_stats.to_dict()
        except (NodeError, GraphError, ParamError) as e:
            return {"node": node, "rev": rev, "ok": False,
                    "error": {"node": getattr(e, "node_id", None), "message": str(e)}}
        out = {"node": node, "rev": rev, "ok": True, "ms": round((time.perf_counter() - t0) * 1000, 3),
               "cooked": stats["cooked"], "cached": stats["cached"]}
        out.update(summarize(geo, detail))
        out["flow"] = attribute_flow(geo, ins)
        n = snapshot.nodes[node]
        if n.type == "wrangle":                              # which attributes the snippet reads and writes
            try:
                prog = compile_snippet(str(snapshot.get_param(node, "snippet")))
                out["wrangle"] = {"reads": sorted(prog.reads), "writes": sorted(prog.writes)}
            except Exception:  # noqa: BLE001 - a broken snippet already failed the cook above
                pass
        if n.type == "blender_object":                       # what Blender sent: attributes, vertex groups, skips
            data = INPUTS.get(str(n.params.get("object", "")))
            if data is not None:
                out["blender"] = dict(data.info, error=data.error) if data.error else dict(data.info)
        return out

    def preview(self, node: str) -> dict:
        """Cook ``node`` and return ``{"ok", "node", "rev", "geo"}``, or ``{"ok": False, "error"}`` (never raises)."""
        with self.lock:
            snapshot = self.graph.copy()
            rev = self._rev
            frame, fps = self.frame, self.fps
        if node not in snapshot.nodes:
            return {"ok": False, "node": node, "rev": rev, "error": {"node": node, "message": f"no such node {node!r}"}}
        try:
            with self._cook_lock, timectx.at(frame, fps), proxy.using(self.view_level()):
                geo = self.evaluator.cook(snapshot, node)
                handles = []
                inst = snapshot.nodes[node]
                nt = self.evaluator.registry.get(inst.type) if inst.type in self.evaluator.registry else None
                if has_handles(inst.type, nt):                   # inputs (cached) and parameters for the viewport handles
                    ins = [self.evaluator.cook(snapshot, src) for src in snapshot.input_ids(node) if src in snapshot.nodes]
                    handles = handles_for(inst.type, ins, geo, snapshot.effective_params(node), nt)
        except (NodeError, GraphError, ParamError) as e:
            return {"ok": False, "node": node, "rev": rev,
                    "error": {"node": getattr(e, "node_id", None), "message": str(e)}}
        return {"ok": True, "node": node, "rev": rev, "geo": geo, "handles": handles}

    def start(self, cook_immediately: bool = True) -> None:
        if self._thread and self._thread.is_alive():
            return
        self._stop.clear()
        if cook_immediately:
            self._wake.set()
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
        self.flush()

        with self._cook_lock:
            self.evaluator.rbd.clear()

    def wait_idle(self, timeout: float = 10.0) -> bool:
        """Block until every edit made so far has been cooked (for tests, scripts and AI tools)."""
        end = time.monotonic() + timeout
        with self._idle:
            while self._cooked_rev < self._rev:
                left = end - time.monotonic()
                if left <= 0:
                    return False
                self._idle.wait(left)
        return True

    # -- the graph file ----------------------------------------------------
    def watch_file(self, path, interval: float = 0.25, save: bool = True) -> None:
        """Bind to ``path``: reload it when it changes on disk; with ``save``, write edits back to it."""
        self.path = Path(path)
        self._save_enabled = save
        try:
            self._file_text = self.path.read_text(encoding="utf-8")
        except OSError:
            self._file_text = None
        with self.lock:
            self._dirty = False
        self._stamp = self._file_stamp()
        self._ensure_watcher(interval)

    def _ensure_watcher(self, interval: float = 0.25) -> None:
        if self._watch_thread and self._watch_thread.is_alive():
            return

        def loop():
            while not self._stop.wait(interval):
                try:
                    self._poll_file()
                    self._poll_library()
                except Exception:  # noqa: BLE001 - keep watching
                    log.exception("file watcher failed")

        self._watch_thread = threading.Thread(target=loop, name="minihou-watch", daemon=True)
        self._watch_thread.start()

    # -- node libraries ------------------------------------------------------
    def attach_library(self, library, interval: float = 0.25) -> None:
        """Load a :class:`~minihou.library.NodeLibrary` now and hot-reload it from then on."""
        self.library = library
        self._poll_library()
        self._ensure_watcher(interval)

    def _poll_library(self) -> None:
        lib = self.library
        if lib is None:
            return
        with self._cook_lock:                         # swap node code between cooks, never during one
            changed = lib.refresh()
        if not changed:
            return
        with self.lock:
            self._rev += 1                            # code hashes are in the cache keys: affected nodes recook
            self._wake.set()
        for l in list(self._listeners):
            fn = getattr(l, "on_types", None)
            if fn:
                try:
                    fn()
                except Exception:  # noqa: BLE001
                    log.exception("listener failed")
        self._after_structural_change("library")

    def library_status(self) -> Optional[dict]:
        return self.library.status() if self.library is not None else None

    def _file_stamp(self):
        try:
            st = self.path.stat()
            return (st.st_mtime_ns, st.st_size)
        except (OSError, AttributeError):
            return None

    def _poll_file(self) -> None:
        with self._file_lock:
            self._poll_file_locked()

    def _poll_file_locked(self) -> None:
        path = self.path
        if path is None:
            return
        cur = self._file_stamp()
        if cur is not None and cur != self._stamp:
            try:
                text = path.read_text(encoding="utf-8")
            except OSError:
                return                  # e.g. Windows mid-replace: keep the old stamp so the next poll retries
            self._stamp = cur
            if text is not None and text != self._file_text:      # not our own write
                self._file_text = text
                try:
                    g = Graph.from_json(text, self.evaluator.registry, check=False)
                except (GraphError, ValueError) as e:
                    log.warning("graph reload failed: %s: %s", path.name, e)
                    self._report_error(None, f"{path.name}: {e}", self._rev)
                    return
                log.info("reloaded %s", path.name)
                with self.lock:
                    self._undo.append((self.graph.to_dict(), None, 0.0))
                    del self._undo[:-UNDO_LIMIT]
                    self._redo.clear()
                self.replace_graph(g, origin="file")
                with self.lock:
                    self._dirty = False
                return
        if self._dirty and self._save_enabled:
            self._flush_locked()

    def flush(self) -> bool:
        """Write unsaved edits to the graph file now. Returns True if the file is up to date."""
        with self._file_lock:
            return self._flush_locked()

    def _flush_locked(self) -> bool:
        if self.path is None or not self._save_enabled:
            return False
        with self.lock:
            if not self._dirty:
                return True
            text = self.graph.to_json()
            self._dirty = False
        if text == self._file_text:
            return True
        try:
            _atomic_write(self.path, text)
        except OSError as e:
            with self.lock:
                self._dirty = True
            if self.save_error != str(e):
                log.warning("cannot save %s: %s", self.path, e)
            self.save_error = str(e)
            return False
        self.save_error = None
        self._file_text = text
        self._stamp = self._file_stamp()
        return True

    def save_as(self, path, overwrite: bool = False) -> dict:
        """Write the open graph to another file and switch to it (the old file is kept as last saved). Raises
        :class:`GraphError` if the target exists and ``overwrite`` is false."""
        path = Path(path).resolve()
        with self._file_lock, self.lock:
            if self.path is not None and path == self.path.resolve():
                raise GraphError("that is the file already open: edits are saved to it as you make them")
            if path.exists() and not overwrite:
                raise GraphError(f"{path.name} already exists")
            text = self.graph.to_json()
            path.parent.mkdir(parents=True, exist_ok=True)
            _atomic_write(path, text)
        return self.open_file(path)

    def open_file(self, path, create: bool = False) -> dict:
        """Switch the session to another graph file (creating a starter graph if asked). The old file is saved
        first. Raises :class:`GraphError` / ``OSError``."""
        path = Path(path).resolve()
        # Same lock order as the watcher. Edits cannot arrive between saving the old
        # graph and switching it; a failed save leaves graph/path/history untouched.
        with self._file_lock, self.lock:
            same = self.path is not None and path == self.path.resolve()
            if same and path.is_file():
                if self._dirty and not self._flush_locked():
                    raise GraphError("Cannot save current graph; editor was not switched: " + (self.save_error or "saving is disabled"))
                return self.snapshot()  # never reload stale disk data over current edits
            if path.exists():
                text = path.read_text(encoding="utf-8")
                g = Graph.from_json(text, self.evaluator.registry, check=False)
            elif create:
                g = Graph.from_dict(NEW_GRAPH, self.evaluator.registry, check=False)
                text = g.to_json()
            else:
                raise GraphError(f"no such file: {path}")
            if self._dirty and not self._flush_locked():
                raise GraphError("Cannot save current graph; editor was not switched: " + (self.save_error or "no writable current graph file"))
            if not path.exists():
                path.parent.mkdir(parents=True, exist_ok=True)
                _atomic_write(path, text)
            self.path = path
            self._save_enabled = True
            self._file_text = text
            self._stamp = self._file_stamp()
            self.save_error = None
            self._undo.clear()
            self._redo.clear()
            self._document_id += 1
            self._dirty = layout_missing(g)
            self.replace_graph(g, origin="open")
            snapshot = self.snapshot()
        self._ensure_watcher()
        return snapshot


def _atomic_write(path: Path, text: str) -> None:
    tmp = path.with_name(f".{path.name}.{os.getpid()}.tmp")
    with open(tmp, "w", encoding="utf-8", newline="\n") as f:
        f.write(text)
    try:
        os.replace(tmp, path)
    except PermissionError:                     # Windows: target held open without FILE_SHARE_DELETE
        try:
            os.unlink(tmp)
        except OSError:
            pass
        with open(path, "w", encoding="utf-8", newline="\n") as f:
            f.write(text)
