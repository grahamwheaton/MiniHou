"""Main-thread runtime: connects the socket thread to Blender.

Threading contract
------------------
* The socket thread (``LinkClient``) only calls ``Inbox.push`` / ``_on_state`` -- plain Python.
* **Everything that touches bpy runs in** :meth:`Runtime.tick`, a ``bpy.app.timers`` callback
  (main thread), or in the depsgraph handler, which only sets a flag.

Feedback-loop guards (engine -> Blender mesh write -> depsgraph update -> param forward -> engine ...)
-------------------------------------------------------------------------------------------------
1. The depsgraph handler writes nothing; it sets ``params_dirty``. The timer does the work.
2. Forwarding is a *diff* against the values the engine is known to have (``ParamForwarder``), so an
   update caused by the engine's own mesh write forwards nothing.
3. The engine never echoes parameter changes back to the client that made them.
4. Values the engine pushes (interface messages) are recorded as known *before* they are written to
   the modifier.
5. Forwarding is rate limited (<= 30 Hz) and a circuit breaker trips on > 120 forwarded values/s.
6. ``tick`` is not re-entrant.

Blender objects as graph inputs (``blender_object`` nodes)
---------------------------------------------------------
The engine names the objects its graph reads (``watch_objects``). The depsgraph handler flags a watched object when it
changes; ``tick`` reads it (evaluated mesh) at most every ``OBJECT_INTERVAL`` and sends ``object_geo``, or
``object_missing`` when it cannot be read. A content digest skips sends of unchanged data, which also ends the loop if a
watched object depends on MiniHou's output (output write -> object update -> same data -> nothing sent).
"""
from __future__ import annotations

import hashlib
import time
import traceback
from collections import deque
from typing import Dict, List, Optional

import bpy
from bpy.app.handlers import persistent

from . import mesh_reader, mesh_writer, proxy
from .client import LinkClient
from .protocol import GeoUpdate, Mirror, ProtocolError
from .state import Inbox, ParamForwarder

MIN_FORWARD_INTERVAL = 1.0 / 30.0
BUSY_INTERVAL = 1.0 / 60.0
IDLE_INTERVAL = 0.05
OBJECT_INTERVAL = 0.1             # seconds between sends of one watched object
EXISTENCE_INTERVAL = 0.5          # how often watched objects are checked for being added, deleted or renamed


class Runtime:
    def __init__(self):
        self.client: Optional[LinkClient] = None
        self.inbox = Inbox()
        self.mirror = Mirror()
        self.forwarder = ParamForwarder()
        self.params: List[dict] = []
        self.state = "disconnected"
        self.detail = ""
        self.last_error = ""
        self.params_dirty = False
        self.forwarding_enabled = True
        self.sent_count = 0
        self.applied_count = 0
        self.last_timings: Dict[str, object] = {}
        self.last_cook_ms = 0.0
        self.last_latency_ms = 0.0
        self.counts = {"point": 0, "vertex": 0, "prim": 0}
        self.log: deque = deque(maxlen=200)
        self._in_tick = False
        self._timer_on = False
        self._last_forward = 0.0
        self._ui_dirty = True
        self.watched: List[str] = []
        self.dirty_objects: set = set()
        self.objects_sent = 0
        self.object_info: Dict[str, str] = {}          # name -> what was last sent (shown in the panel)
        self._object_digest: Dict[str, str] = {}
        self._object_last: Dict[str, float] = {}
        self._object_present: Dict[str, bool] = {}
        self._last_existence = 0.0
        self._missing_instances: List[str] = []
        self.frame_dirty = True                 # send the current frame after connecting
        self._sent_frame = None
        self.extras: Dict[str, Mirror] = {}      # extra outputs: Blender object name -> mirrored geometry
        self._refused_extras: set = set()

    # -- lifecycle ---------------------------------------------------------
    def connect(self, host: str = "127.0.0.1", port: int = 8765) -> None:
        mesh_writer.assert_main_thread()
        self.disconnect()
        self.client = LinkClient(host, port, on_message=self.inbox.push, on_state=self._on_state,
                                 client_name=f"blender {bpy.app.version_string}")
        self.last_error = ""
        self.client.start()
        if not self._timer_on:
            bpy.app.timers.register(timer_entry, first_interval=0.0, persistent=True)
            self._timer_on = True
        if _depsgraph_post not in bpy.app.handlers.depsgraph_update_post:
            bpy.app.handlers.depsgraph_update_post.append(_depsgraph_post)
        if _frame_change not in bpy.app.handlers.frame_change_post:
            bpy.app.handlers.frame_change_post.append(_frame_change)
        self.frame_dirty, self._sent_frame = True, None
        self._log(f"connecting to {host}:{port}")

    def disconnect(self) -> None:
        if self.client is not None:
            self.client.stop()
            self.client = None
        self.inbox.clear()
        self.state, self.detail = "disconnected", ""
        self._ui_dirty = True
        if _depsgraph_post in bpy.app.handlers.depsgraph_update_post:
            bpy.app.handlers.depsgraph_update_post.remove(_depsgraph_post)
        if _frame_change in bpy.app.handlers.frame_change_post:
            bpy.app.handlers.frame_change_post.remove(_frame_change)

    def _log(self, msg: str) -> None:
        line = f"[minihou] {msg}"
        self.log.append(line)
        print(line, flush=True)

    # -- socket thread -----------------------------------------------------
    def _on_state(self, state: str, detail: str) -> None:
        # Runs on the socket thread: plain attributes only.
        self.state, self.detail = state, detail
        self._ui_dirty = True

    # -- main thread -------------------------------------------------------
    def _timer(self) -> Optional[float]:
        if self.client is None:
            self._timer_on = False
            return None
        return self.tick()

    def tick(self) -> float:
        """One main-thread step. Returns the delay until the next step."""
        if mesh_writer.modal_ui_active():
            return IDLE_INTERVAL
        if self._in_tick:
            return IDLE_INTERVAL
        self._in_tick = True
        busy = False
        try:
            events, geo = self.inbox.take()
            for e in events:
                busy = True
                self._handle_event(e)
            if geo is not None:
                busy = True
                self._apply_geo(geo)
            for name, update in self.inbox.take_extras().items():
                busy = True
                self._apply_extra(name, update)
            if self.params_dirty:
                busy = self._forward_params() or busy
            if self.watched:
                busy = self._send_objects() or busy
            if self.frame_dirty:
                busy = self._send_frame() or busy
            if self._ui_dirty:
                self._ui_dirty = False
                _tag_redraw()
        except Exception as e:  # noqa: BLE001 - the timer must survive anything
            self.last_error = f"{type(e).__name__}: {e}"
            self._log("tick failed:\n" + traceback.format_exc())
            self._ui_dirty = True
        finally:
            self._in_tick = False
        return BUSY_INTERVAL if busy else IDLE_INTERVAL

    def _handle_event(self, e: dict) -> None:
        t = e.get("type")
        if t == "interface":
            self._apply_interface(e)
        elif t == "error":
            self.last_error = f"{e.get('where', 'engine')}: {e.get('message', '')}"
            self._log(self.last_error)
            self._ui_dirty = True
        elif t == "welcome":
            self.last_error = ""
            self._log(f"connected to engine (protocol {e.get('protocol')})")
            self.frame_dirty, self._sent_frame = True, None       # a (re)started engine needs the time
        elif t == "outputs":
            names = [str(n) for n in e.get("names", [])]
            for gone in mesh_writer.remove_extra_objects(names):
                self._log(f"output object {gone!r} removed (no longer in the graph)")
            self.extras = {n: m for n, m in self.extras.items() if n in names}
            self._ui_dirty = True
        elif t == "watch_objects":
            self.watched = [str(n) for n in e.get("objects", [])]
            self.dirty_objects = set(self.watched)       # (re)send everything: the engine may have restarted
            self._object_digest.clear()
            self._object_present.clear()
            self.object_info = {n: v for n, v in self.object_info.items() if n in self.watched}
            if self.watched:
                self._log(f"engine reads Blender objects: {', '.join(self.watched)}")
            self._ui_dirty = True

    def _apply_interface(self, iface: dict) -> None:
        params = iface["params"]
        out, proxy_obj, mod = proxy.ensure_objects()
        ng = mod.node_group
        if ng.get(proxy.STRUCTURE_KEY) != iface["structure"]:
            proxy.sync_interface(ng, out, params, iface["structure"])
            self._log(f"synced node group inputs: {[p['name'] for p in params]}")
        values = {p["name"]: proxy.canonical(p["kind"], p["value"]) for p in params}
        self.forwarder.reset(values)                 # guard 4: record as known *before* writing
        self.params = params
        proxy.write_values(mod, ng, params, values)
        self.params_dirty = False
        self._ui_dirty = True

    def _apply_geo(self, u: GeoUpdate) -> None:
        try:
            self.mirror.apply(u)
        except ProtocolError as e:
            self.last_error = f"out of sync ({e}); requesting full resend"
            self._log(self.last_error)
            self.mirror = Mirror()
            if self.client:
                self.client.resync()
            return
        out, _proxy, mod = proxy.ensure_objects()
        t0 = time.perf_counter()
        timings = mesh_writer.apply_update(out.data, self.mirror, u)
        timings["warnings"].extend(mesh_writer.write_detail(out, self.mirror.detail))
        names = self.mirror.detail.get("instance_objects") or []
        missing = proxy.sync_instances(mod.node_group, [str(n) for n in names] if isinstance(names, list) else [])
        if missing and missing != self._missing_instances:
            self._log(f"instance objects not found in the Blender file: {', '.join(missing)}")
        self._missing_instances = missing
        timings["total_ms"] = (time.perf_counter() - t0) * 1000
        self.last_timings = timings
        self.applied_count += 1
        self.counts = dict(u.counts)
        self.last_cook_ms = float(u.cook.get("ms", 0.0))
        if u.t_sent:
            self.last_latency_ms = (time.time() - u.t_sent) * 1000
        for w in timings["warnings"]:
            self._log(w)
        if self.last_error.startswith("cook"):
            self.last_error = ""
        self._ui_dirty = True

    def _apply_extra(self, name: str, u: GeoUpdate) -> None:
        """A named output of the graph: its own mesh object, updated like the main output."""
        mirror = self.extras.setdefault(name, Mirror())
        try:
            mirror.apply(u)
        except ProtocolError as e:
            self._log(f"output {name!r} out of sync ({e}); requesting full resend")
            self.extras.pop(name, None)
            if self.client:
                self.client.resync()
            return
        obj = mesh_writer.ensure_extra_object(name)
        if obj is None:
            if name not in self._refused_extras:
                self._refused_extras.add(name)
                self._log(f"output {name!r}: an object with that name already exists in the file and was not made by "
                          f"MiniHou; pick another object name")
            return
        self._refused_extras.discard(name)
        timings = mesh_writer.apply_update(obj.data, mirror, u)
        for w in timings["warnings"] + mesh_writer.write_detail(obj, mirror.detail):
            self._log(f"output {name!r}: {w}")
        self.applied_count += 1
        self._ui_dirty = True

    def _forward_params(self) -> bool:
        """Forward changed modifier values. Returns True if anything was sent."""
        if not self.params or not self.forwarding_enabled or self.client is None:
            self.params_dirty = False
            return False
        now = time.monotonic()
        if now - self._last_forward < MIN_FORWARD_INTERVAL:
            return False                              # guard 5: stay dirty, try again next tick
        proxy_obj = bpy.data.objects.get(proxy.PROXY_OBJECT)
        mod = proxy_obj.modifiers.get(proxy.MODIFIER) if proxy_obj else None
        if mod is None or mod.node_group is None:
            self.params_dirty = False
            return False
        self.params_dirty = False
        current = proxy.read_values(mod, mod.node_group, self.params)
        sent = False
        for name, v in self.forwarder.changes(current).items():
            payload = list(v) if isinstance(v, tuple) else v
            if self.client.set_param(name, payload):
                self.forwarder.commit(name, v, now)
                self.sent_count += 1
                sent = True
            if self.forwarder.tripped:
                self.last_error = "parameter forwarding stopped: possible feedback loop (reconnect to reset)"
                self._log(self.last_error)
                self._ui_dirty = True
                break
        if sent:
            self._last_forward = now
        return sent

    def _send_frame(self) -> bool:
        """Tell the engine the current frame (for @Frame / @Time). Latest wins; repeats are not sent."""
        if self.client is None or self.state != "connected":
            return False
        scene = bpy.context.scene
        frame = float(scene.frame_current) + float(scene.frame_subframe)
        fps = float(scene.render.fps) / float(scene.render.fps_base or 1.0)
        self.frame_dirty = False
        if (frame, fps) == self._sent_frame:
            return False
        if self.client.send({"type": "set_frame", "frame": frame, "fps": fps}):
            self._sent_frame = (frame, fps)
            return True
        self.frame_dirty = True
        return False

    def _send_objects(self) -> bool:
        """Send watched objects that changed (main thread). Returns True if anything was sent."""
        if self.client is None:
            return False
        now = time.monotonic()
        if now - self._last_existence >= EXISTENCE_INTERVAL:           # deletions and renames do not flag the object
            self._last_existence = now
            for name in self.watched:
                present = bpy.data.objects.get(name) is not None
                if self._object_present.get(name) != present:
                    self.dirty_objects.add(name)
        sent = False
        depsgraph = None
        for name in sorted(self.dirty_objects):
            if name not in self.watched:
                self.dirty_objects.discard(name)
                continue
            if now - self._object_last.get(name, -1e9) < OBJECT_INTERVAL:
                continue                                                # stays dirty: sent on a later tick
            self.dirty_objects.discard(name)
            self._object_last[name] = now
            obj = bpy.data.objects.get(name)
            self._object_present[name] = obj is not None
            if obj is None:
                header, arrays = {"type": "object_missing", "object": name,
                                  "reason": f"no object named {name!r} in the Blender file"}, None
            else:
                if depsgraph is None:
                    depsgraph = bpy.context.evaluated_depsgraph_get()
                try:
                    fields, arrays = mesh_reader.read_object(obj, depsgraph)
                    header = {"type": "object_geo", "object": name, **fields}
                except ValueError as e:
                    header, arrays = {"type": "object_missing", "object": name, "reason": str(e)}, None
            digest = _digest(header, arrays)
            if self._object_digest.get(name) == digest:
                continue                                                # unchanged (e.g. a selection change)
            if self.client.send(header, arrays):
                self._object_digest[name] = digest
                self.objects_sent += 1
                sent = True
                info = header.get("info", {})
                self.object_info[name] = (f"{info.get('points', 0)} pts, {info.get('prims', 0)} prims, "
                                          f"{info.get('read_ms', 0)} ms" if header["type"] == "object_geo"
                                          else header["reason"])
                self._ui_dirty = True
        return sent


def _digest(header: dict, arrays: Optional[dict]) -> str:
    """Content hash of a message, without the timing info (which changes on every read)."""
    h = hashlib.blake2b(digest_size=16)
    h.update(repr((header["type"], header.get("matrix"), header.get("reason"))).encode())
    for key in sorted(arrays or {}):
        a = arrays[key]
        h.update(f"{key}{a.dtype}{a.shape}".encode())
        h.update(a.tobytes() if not a.flags.c_contiguous else memoryview(a).cast("B"))
    return h.hexdigest()


_runtime: Optional[Runtime] = None


def timer_entry() -> Optional[float]:
    """The function registered with ``bpy.app.timers`` (module level, so registration identity is stable)."""
    return get_runtime()._timer()


def get_runtime() -> Runtime:
    global _runtime
    if _runtime is None:
        _runtime = Runtime()
    return _runtime


@persistent
def _depsgraph_post(scene, depsgraph) -> None:
    """Guard 1: only flags what changed (the proxy, watched objects). Never writes bpy data."""
    rt = _runtime
    if rt is None or rt.client is None:
        return
    watched = rt.watched
    for u in depsgraph.updates:
        idb = u.id
        if isinstance(idb, bpy.types.Object):
            name = idb.name
            if name == proxy.PROXY_OBJECT:
                rt.params_dirty = True
            elif watched and name in watched:
                rt.dirty_objects.add(name)


@persistent
def _frame_change(scene, depsgraph=None) -> None:
    """The timeline moved: flag it (the timer sends it). Never writes bpy data."""
    rt = _runtime
    if rt is not None and rt.client is not None:
        rt.frame_dirty = True


def _tag_redraw() -> None:
    wm = bpy.context.window_manager
    if wm is None:
        return
    for win in wm.windows:
        for area in win.screen.areas:
            if area.type in {"VIEW_3D", "PROPERTIES"}:
                area.tag_redraw()


def shutdown() -> None:
    global _runtime
    if _runtime is not None:
        _runtime.disconnect()
    if _depsgraph_post in bpy.app.handlers.depsgraph_update_post:
        bpy.app.handlers.depsgraph_update_post.remove(_depsgraph_post)
    if _frame_change in bpy.app.handlers.frame_change_post:
        bpy.app.handlers.frame_change_post.remove(_frame_change)
    _runtime = None
