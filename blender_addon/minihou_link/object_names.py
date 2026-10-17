"""Fuzzy Blender object-name search for the MiniHou Blender Object node (add-on 0.5.0, ``object_names_v1``).

The app asks only while its object field is being edited: ``object_names_query`` {request_id, query, limit}.
This module answers ``object_names`` {request_id, query, hash, count, partial, scanned, ms, matches} with at most
``limit`` (<= 50) ranked matches ``{name, type, mesh}``. The full list never crosses the link.

Main-thread cost is bounded: a search is a resumable job that reads ``bpy.data.objects`` through a held
collection iterator, stopping each timer tick after ``BUDGET_MS`` milliseconds; the next tick resumes. The
iterator is revalidated every tick (undo/load generation; the last object read must still be alive) and
restarted otherwise, so it never stands on a freed object. Names are cached per scene signature so repeat
queries scan a plain Python list. A newer query replaces the running job (stale work is dropped); a partial
answer goes out after ``PARTIAL_AFTER_MS`` so the app can show results while the scan continues, and the scan
gives up with ``partial: true`` after ``MAX_TOTAL_MS``.

``score`` mirrors rust/crates/minihou-app/src/object_names.rs::score.
"""
from __future__ import annotations

import heapq
import itertools
import time

CAPABILITY = "object_names_v1"
LIMIT = 50
BUDGET_MS = 4.0
PARTIAL_AFTER_MS = 150.0
MAX_TOTAL_MS = 3000.0
_SEPARATORS = " _-./:"


def _word_start(name, j):
    if j == 0:
        return True
    a, b = name[j - 1], name[j]
    return a in _SEPARATORS or (a.islower() and b.isupper()) or (a.isalpha() and b.isdigit())


def score(query, name, low=None):
    """Score ``name`` for a lower-cased non-empty ``query``; None = no match (same rules as the app)."""
    if not query:
        return 0
    low = name.lower() if low is None else low
    n = len(low)
    if low == query:
        return 1000
    if low.startswith(query):
        return 900 - n
    if len(name) != n:
        return None
    i = low.find(query)
    if i >= 0:
        return 700 - i - n // 4 + (50 if _word_start(name, i) else 0)
    pos, s, prev = 0, 0, -2
    for c in query:
        j = low.find(c, pos)
        if j < 0:
            return None
        if j == prev + 1:
            s += 10
        if _word_start(name, j):
            s += 15
        s -= min(j - pos, 10)
        prev, pos = j, j + 1
    return 300 + s - n // 4


# Bumped by undo/redo/load handlers: a held collection iterator is never resumed across them.
GENERATION = [0]


def _bump(*_args):
    GENERATION[0] += 1


def _install_handlers():
    import bpy
    for name in ("undo_post", "redo_post", "load_post"):
        handlers = getattr(bpy.app.handlers, name)
        if not any(getattr(h, "__name__", "") == "_minihou_object_names_bump" for h in handlers):
            def _minihou_object_names_bump(*args):
                _bump()
            handlers.append(bpy.app.handlers.persistent(_minihou_object_names_bump))
    handlers = bpy.app.handlers.depsgraph_update_post
    if not any(getattr(h, "__name__", "") == "_minihou_object_names_dirty" for h in handlers):
        def _minihou_object_names_dirty(scene, depsgraph):
            if depsgraph.id_type_updated("OBJECT"):
                CACHE_GEN[0] += 1
        handlers.append(bpy.app.handlers.persistent(_minihou_object_names_dirty))


class Job:
    """One resumable search over ``objects`` (a sliceable collection of objects with ``.name``/``.type``)."""

    def __init__(self, request_id, query, limit, objects, cache=None):
        self.request_id = request_id
        self.query = (query or "").strip()
        self.q = self.query.lower()
        self.limit = max(1, min(int(limit or LIMIT), LIMIT))
        self.objects = objects
        # Parallel string lists (names, lower names, types, mesh flags): no per-object GC-tracked tuples.
        self.cache = cache
        self.count = len(cache[0]) if cache is not None else None   # unknown until the iterator ends
        self.building = ([], [], [], []) if cache is None else None
        self.pos = 0
        self.heap = []                         # (score, -order, name, type, mesh); smallest = worst kept
        self.started = time.perf_counter()
        self.partial_sent = False
        self.max_slice_ms = 0.0
        self.iter = None
        self.last = None                       # the last object read (the iterator's position)
        self.generation = GENERATION[0]
        self.exact = True

    def _offer(self, sc, name, typ, mesh):
        item = (sc, _neg(name), name, typ, mesh)
        if len(self.heap) < self.limit:
            heapq.heappush(self.heap, item)
        elif item > self.heap[0]:
            heapq.heapreplace(self.heap, item)

    def _validate(self):
        """Keep the held collection iterator only while the object it stands on is alive.

        Blender invalidates an ID's Python wrapper when the ID is freed (``ReferenceError`` on access); the
        iterator's position is that object, so neighbours being added or removed are harmless. Undo/redo/load
        replace the database: their handlers bump GENERATION. Otherwise restart and skip ``pos`` objects."""
        ok = self.iter is not None and GENERATION[0] == self.generation
        if ok and self.last is not None:
            try:
                self.last.name
            except ReferenceError:
                ok = False
        if not ok:
            self.iter = iter(self.objects)
            self.generation = GENERATION[0]
            for _ in itertools.islice(self.iter, self.pos):
                pass

    def _rows(self, n=256):
        names, lows, types, meshes = [], [], [], []
        obj = None
        for obj in itertools.islice(self.iter, n):
            name = obj.name
            typ = obj.type
            names.append(name); lows.append(name.lower()); types.append(typ); meshes.append(typ == "MESH")
        if obj is not None:
            self.last = obj
        return names, lows, types, meshes

    def step(self, budget_ms=BUDGET_MS):
        """Scan until the budget is used. Returns True when the scan is complete."""
        t0 = time.perf_counter()
        deadline = t0 + budget_ms / 1000.0
        q, q0 = self.q, self.q[:1]
        empty = not q
        if self.cache is None:
            self._validate()
        while not self.done():
            if self.cache is not None:
                end = min(self.pos + 256, self.count)
                cols = tuple(col[self.pos:end] for col in self.cache)
            else:
                cols = self._rows()
                if not cols[0]:
                    self.count = self.pos          # end of the collection: done
                    break
                for into, col in zip(self.building, cols):
                    into.extend(col)
            names, lows, types, meshes = cols
            for i, low in enumerate(lows):
                if empty:
                    self._offer(0, names[i], types[i], meshes[i])
                elif q0 in low:
                    sc = score(q, names[i], low)
                    if sc is not None:
                        self._offer(sc, names[i], types[i], meshes[i])
            self.pos += len(names)
            if empty and len(self.heap) >= self.limit and self.cache is None:
                # ID collections are sorted by name: the first `limit` names are the answer.
                self.building = None
                self.count = self.pos
                self.exact = False
                break
            if time.perf_counter() >= deadline:
                break
        self.max_slice_ms = max(self.max_slice_ms, (time.perf_counter() - t0) * 1000.0)
        return self.done()

    def done(self):
        return self.count is not None and self.pos >= self.count

    def elapsed_ms(self):
        return (time.perf_counter() - self.started) * 1000.0

    def response(self, signature, partial):
        best = sorted(self.heap, key=lambda it: (-it[0], it[2]))
        return {"type": "object_names", "request_id": self.request_id, "query": self.query, "hash": signature,
                "count": self.count if self.count is not None else self.pos,
                "count_exact": self.count is not None and self.exact, "partial": bool(partial), "scanned": self.pos,
                "ms": round(self.elapsed_ms(), 3), "max_slice_ms": round(self.max_slice_ms, 3),
                "matches": [{"name": n, "type": t, "mesh": m} for _, _, n, t, m in best]}


def _neg(name):
    # Ties keep the alphabetically first name: invert so heap order prefers it.
    return tuple(-ord(c) for c in name[:16])


# Bumped when Blender reports object changes (depsgraph handler): invalidates the name cache.
CACHE_GEN = [0]


def signature(objects=None):
    """Scene signature for the name cache and the app's (hash, query) cache: O(1), no collection walk."""
    return f"{GENERATION[0]}.{CACHE_GEN[0]}"


CACHE_SECONDS = 5.0


class Searcher:
    """Owns the running job and the name cache (main thread only)."""

    def __init__(self):
        self.job = None
        self.cache = None
        self.cache_sig = None
        self.cache_time = 0.0

    def start(self, msg, objects):
        sig = signature(objects)
        fresh = self.cache is not None and self.cache_sig == sig and time.monotonic() - self.cache_time < CACHE_SECONDS
        self.job = Job(msg.get("request_id", 0), msg.get("query", ""), msg.get("limit", LIMIT), objects,
                       cache=self.cache if fresh else None)
        self.job.signature = sig

    @property
    def active(self):
        return self.job is not None

    def step(self, send, budget_ms=BUDGET_MS):
        """Advance the job one slice; ``send(dict)`` delivers answers. Returns True while work remains."""
        job = self.job
        if job is None:
            return False
        done = job.step(budget_ms)
        if done:
            if job.building is not None:
                self.cache, self.cache_sig, self.cache_time = job.building, job.signature, time.monotonic()
            send(job.response(job.signature, False))
            self.job = None
            return False
        if job.elapsed_ms() > MAX_TOTAL_MS:
            send(job.response(job.signature, True))
            self.job = None
            return False
        if not job.partial_sent and job.elapsed_ms() > PARTIAL_AFTER_MS:
            job.partial_sent = True
            send(job.response(job.signature, True))
        return True


SEARCHER = Searcher()


def handle_query(msg):
    """Runtime hook: a new query replaces the running one (stale work dropped)."""
    import bpy
    _install_handlers()
    SEARCHER.start(msg, bpy.data.objects)


def run_blocking(query, objects, budget_ms=BUDGET_MS, request_id=1):
    """Test helper: drive a search to completion tick by tick; returns (final, partials, ticks, max_slice_ms)."""
    s = Searcher()
    out = []
    s.start({"request_id": request_id, "query": query}, objects)
    ticks = 0
    job = s.job
    while s.step(out.append, budget_ms):
        ticks += 1
    return out[-1], out[:-1], ticks + 1, job.max_slice_ms
