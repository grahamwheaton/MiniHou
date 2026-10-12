"""Pull-based, cached graph evaluation.

Cooking a node asks for its inputs, which ask for theirs, and so on -- but only as far
as needed:

1. Every node's **cache key** is computed first, without cooking anything. It is a hash
   of (node type, node-type version, canonical params, *keys of its inputs*), a Merkle
   chain. Changing one parameter changes that node's key and, transitively, every key
   downstream -- and nothing else.
2. Cooking walks from the target. A node whose key is already cached is returned as-is
   and its inputs are never visited (so an unchanged branch costs nothing, even when a
   sibling branch changed).

Cached geometries are frozen (read-only arrays), so sharing them between cache entries,
downstream nodes and link clients is safe.
"""
from __future__ import annotations

import hashlib
import json
import time
from collections import OrderedDict
from dataclasses import dataclass, field
from typing import Dict, List, Optional, Tuple

from . import proxy, timectx
from .geometry import Geometry
from .inputs import input_cache_scope
from .graph import Graph, GraphError
from .loops import analyze as analyze_loops, cook_budget, consume_iteration
from .sdk import REGISTRY, Registry

DEFAULT_CACHE_BYTES = 2 * 1024 ** 3


class CookCancelled(RuntimeError):
    """A cook was stopped between nodes because its result was no longer wanted (see ``Evaluator.cancel``)."""


class NodeError(RuntimeError):
    """A node function raised, or returned something invalid."""

    def __init__(self, node_id: str, node_type: str, cause: BaseException):
        self.node_id = node_id
        self.node_type = node_type
        self.cause = cause
        where = _library_location(cause)
        super().__init__(f"node {node_id!r} ({node_type}): {type(cause).__name__}: {cause}{where}")


def _library_location(exc: BaseException) -> str:
    """" (nodes/twist_height.py, line 12)" when the error comes from a node-library file: tells a person (or an AI) where to look."""
    import traceback
    from .sdk import REGISTRY
    files = {nt.source for nt in REGISTRY if nt.source}
    for fr in reversed(traceback.extract_tb(exc.__traceback__)):
        if fr.filename in files:
            return f" ({fr.filename}, line {fr.lineno})"
    return ""


@dataclass
class CookStats:
    target: str = ""
    key: str = ""
    cooked: List[str] = field(default_factory=list)     # nodes whose function actually ran, in order
    cached: List[str] = field(default_factory=list)     # nodes served from cache
    node_ms: Dict[str, float] = field(default_factory=dict)
    total_ms: float = 0.0

    def to_dict(self) -> dict:
        return {"target": self.target, "cooked": list(self.cooked), "cached": list(self.cached),
                "node_ms": {k: round(v, 3) for k, v in self.node_ms.items()},
                "total_ms": round(self.total_ms, 3)}


@dataclass
class _Entry:
    geo: Geometry
    arrays: Tuple[int, ...]      # ids of arrays this entry pins (for shared-array byte accounting)
    node_type: str = ""


class Cache:
    """LRU cache of frozen geometries, bounded by (de-duplicated) array bytes."""

    def __init__(self, max_bytes: int = DEFAULT_CACHE_BYTES):
        self.max_bytes = max_bytes
        self._entries: "OrderedDict[str, _Entry]" = OrderedDict()
        self._array_refs: Dict[int, List[int]] = {}   # id(array) -> [refcount, nbytes]
        self.total_bytes = 0

    def __len__(self) -> int:
        return len(self._entries)

    def __contains__(self, key: str) -> bool:
        return key in self._entries

    def keys(self) -> List[str]:
        return list(self._entries)

    def get(self, key: str) -> Optional[Geometry]:
        e = self._entries.get(key)
        if e is None:
            return None
        self._entries.move_to_end(key)
        return e.geo

    def put(self, key: str, geo: Geometry, node_type: str = "") -> None:
        if key in self._entries:
            self._entries.move_to_end(key)
            return
        ids = []
        for a in geo.unique_arrays():
            ref = self._array_refs.get(id(a))
            if ref is None:
                self._array_refs[id(a)] = [1, a.nbytes]
                self.total_bytes += a.nbytes
            else:
                ref[0] += 1
            ids.append(id(a))
        self._entries[key] = _Entry(geo, tuple(ids), node_type)
        self._evict_to_budget(keep=key)

    def _drop(self, key: str) -> None:
        e = self._entries.pop(key)
        for aid in e.arrays:
            ref = self._array_refs[aid]
            ref[0] -= 1
            if ref[0] == 0:
                self.total_bytes -= ref[1]
                del self._array_refs[aid]

    def _evict_to_budget(self, keep: Optional[str] = None) -> None:
        while self.total_bytes > self.max_bytes and len(self._entries) > 1:
            oldest = next(iter(self._entries))
            if oldest == keep:
                break
            self._drop(oldest)

    def retain(self, live_keys) -> int:
        """Drop every entry not in ``live_keys``. Returns the number dropped."""
        live = set(live_keys)
        dead = [k for k in self._entries if k not in live]
        for k in dead:
            self._drop(k)
        return len(dead)

    def clear(self) -> None:
        for k in list(self._entries):
            self._drop(k)


class Evaluator:
    """Cooks graphs against a shared cache. Not thread-safe: use one thread per evaluator."""

    def __init__(self, registry: Optional[Registry] = None, cache: Optional[Cache] = None):
        self.registry = registry or REGISTRY
        self.cache = cache if cache is not None else Cache()
        self.last_stats = CookStats()
        self.proxy_level: Optional[float] = None           # cook at this proxy level (None: whatever surrounds the cook)
        self.cancel = None                                 # callable -> True stops the cook before its next node (CookCancelled)
        from .rbd import Manager
        self.rbd = Manager()

    def close(self):
        """Release optional physics worlds and all geometry cached by this evaluator."""
        self.rbd.clear()
        self.cache.clear()

    # -- keys --------------------------------------------------------------
    def compute_keys(self, graph: Graph, order: List[str], fixed: Optional[Dict[str, str]] = None) -> Dict[str, str]:
        """Cache key for each node in ``order`` (dependencies first). Cooks nothing. ``fixed`` pins some keys."""
        keys: Dict[str, str] = {}
        for nid in order:
            if fixed and nid in fixed:
                keys[nid] = fixed[nid]
                continue
            inst = graph.nodes[nid]
            nt = self.registry.get(inst.type)
            params = proxy.scale_params(nt, nt.coerce_params(inst.params))
            in_keys = []
            for spec in nt.inputs:
                ref = inst.inputs.get(spec.name)
                if ref is None:
                    in_keys.append(None)
                elif isinstance(ref, list):
                    in_keys.append([keys[r] for r in ref])
                else:
                    in_keys.append(keys[ref])
            payload = json.dumps(
                [nt.name, nt.version, [[p.name, p.to_json_value(params[p.name])] for p in nt.params], in_keys]
                + ([nt.code_hash] if nt.code_hash else []) + ([nt.key_extra(params)] if nt.key_extra else [])
                + ([timectx.key()] if timectx.params_use_time(params) else [])
                + ([["proxy", proxy.key_part(nt)]] if proxy.key_part(nt) is not None else [])
                + ([input_cache_scope()] if input_cache_scope() else []),
                separators=(",", ":"), allow_nan=False)
            keys[nid] = hashlib.blake2b(payload.encode("utf-8"), digest_size=16).hexdigest()
        return keys

    # -- cooking -----------------------------------------------------------
    def cook(self, graph: Graph, target: Optional[str] = None,
             feeds: Optional[Dict[str, Tuple[str, Geometry]]] = None, frame: Optional[float] = None,
             fps: Optional[float] = None) -> Geometry:
        """Cook ``target`` (default: the display node). ``feeds`` supplies some nodes' results from outside as
        ``{node_id: (cache_key, geometry)}``; their upstream is not cooked (how systems receive their inputs).
        ``frame``/``fps`` set the time for ``@Frame``/``@Time`` (default: the surrounding cook's, else frame 1)."""
        with timectx.at(frame, fps), proxy.using(self.proxy_level), cook_budget():
            return self._cook(graph, target, feeds)

    def _cook(self, graph: Graph, target: Optional[str], feeds, *, loop_body=False) -> Geometry:
        t_start = time.perf_counter()
        feeds = feeds or {}
        target = target or graph.output_node()
        if target not in graph.nodes:
            raise GraphError(f"display node {target!r} does not exist")
        order = graph.upstream_order(target, stop=feeds)
        probs = graph.problems(only=[n for n in order if n not in feeds])   # a half-built branch elsewhere is fine
        if probs:
            raise GraphError([m for _, m in probs], probs[0][0])
        if not loop_body and any(graph.nodes[n].type in {"loop_begin", "loop_end"} for n in order):
            # Validate the logical region before looking in the cache (including zero repeats).
            analyze_loops(graph, order, target)
        for nid in order:
            if graph.nodes[nid].type == 'rbd_solver':
                for upstream in graph.upstream_order(nid, stop=feeds):
                    if upstream == nid:
                        continue
                    n = graph.nodes[upstream]
                    nt = self.registry.get(n.type)
                    values = nt.coerce_params(n.params)
                    extra_timed = False
                    if nt.key_extra:
                        with timectx.at(1):
                            first = nt.key_extra(values)
                        with timectx.at(2):
                            extra_timed = first != nt.key_extra(values)
                    if timectx.params_use_time(values) or extra_timed or n.type in ('solver', 'rbd_solver'):
                        raise NodeError(nid, 'rbd_solver', ValueError(
                            'RBD inputs must be static: animated/deforming inputs and nested solvers are unsupported'))
        for nid in list(self.rbd.worlds):
            if nid not in graph.nodes or graph.nodes[nid].type != 'rbd_solver':
                self.rbd.worlds.pop(nid)[1].close()
        keys = self.compute_keys(graph, order, {n: k for n, (k, _) in feeds.items()})
        stats = CookStats(target=target, key=keys[target])
        results: Dict[str, Geometry] = {n: g for n, (_, g) in feeds.items() if n in keys}
        stack = [target]
        while stack:
            nid = stack[-1]
            if nid in results:
                stack.pop()
                continue
            if self.cancel is not None and self.cancel():
                raise CookCancelled("cook cancelled: a newer edit replaced it")
            hit = self.cache.get(keys[nid])
            if hit is not None:
                results[nid] = hit
                stats.cached.append(nid)
                stack.pop()
                continue
            inst = graph.nodes[nid]
            if inst.type == "loop_end":
                begin = inst.inputs["begin"]
                # The body is a recipe, not an ordinary dependency to cook once up front.
                deps = [graph.nodes[begin].inputs["geo"]]
            else:
                deps = graph.input_ids(nid)
            pending = [d for d in deps if d not in results]
            if pending:
                stack.extend(pending)
                continue
            if inst.type == "loop_end":
                results[nid] = self._cook_loop(graph, nid, keys[nid], results, feeds, stats)
            else:
                results[nid] = self._cook_node(graph, nid, keys[nid], results, stats)
            stack.pop()
        stats.total_ms = (time.perf_counter() - t_start) * 1000.0
        self.last_stats = stats
        return results[target]

    def _cook_loop(self, graph, nid, key, results, feeds, stats):
        inst = graph.nodes[nid]
        begin = inst.inputs["begin"]
        initial = results[graph.nodes[begin].inputs["geo"]]
        count = graph.get_param(nid, "iterations")
        state = initial
        t0 = time.perf_counter()
        for iteration in range(count):
            consume_iteration(nid)
            incoming = state.copy()
            incoming.detail.update(iteration=iteration, numiterations=count)
            incoming.freeze()
            # The End key includes the whole body recipe, external inputs, initial
            # geometry and count. Per-iteration keys cannot alias ordinary body cooks.
            seed = hashlib.blake2b(f"repeat-v1:{key}:{iteration}".encode("utf-8"), digest_size=16).hexdigest()
            state = self._cook(graph, inst.inputs["geo"], {**feeds, begin: (seed, incoming)}, loop_body=True)
            body_stats = self.last_stats
            stats.cooked.extend(body_stats.cooked)
            stats.cached.extend(body_stats.cached)
            for body_id, ms in body_stats.node_ms.items():
                stats.node_ms[body_id] = stats.node_ms.get(body_id, 0.0) + ms
        if count:
            state = state.copy()
            # Restore the containing block's metadata after nested loops, and keep
            # per-iteration bookkeeping from leaking into normal downstream geometry.
            for name in ("iteration", "numiterations"):
                if name in initial.detail:
                    state.detail[name] = initial.detail[name]
                else:
                    state.detail.pop(name, None)
            state.freeze()
        if not state.frozen:
            state.validate()
            state.freeze()
        self.cache.put(key, state, "loop_end")
        stats.cooked.append(nid)
        stats.node_ms[nid] = (time.perf_counter() - t0) * 1000.0
        return state

    def _cook_node(self, graph: Graph, nid: str, key: str, results: Dict[str, Geometry],
                   stats: CookStats) -> Geometry:
        inst = graph.nodes[nid]
        nt = self.registry.get(inst.type)
        args = []
        for spec in nt.inputs:
            ref = inst.inputs.get(spec.name)
            if ref is None:
                args.append(None)
            elif isinstance(ref, list):
                args.append([results[r] for r in ref])
            else:
                args.append(results[ref])
        params = proxy.scale_params(nt, nt.coerce_params(inst.params))
        t0 = time.perf_counter()
        try:
            from .rbd import owned
            with owned(self.rbd, nid):
                out = nt.fn(*args, **params)
            if not isinstance(out, Geometry):
                raise TypeError(f"node function returned {type(out).__name__}, expected Geometry")
            if not out.frozen:
                out.validate()
                out.freeze()
        except Exception as e:  # noqa: BLE001 - re-raised with node context
            raise NodeError(nid, inst.type, e) from e
        stats.node_ms[nid] = (time.perf_counter() - t0) * 1000.0
        stats.cooked.append(nid)
        self.cache.put(key, out, nt.name)
        return out

    def live_keys(self, graph: Graph, target: Optional[str] = None) -> List[str]:
        """Keys of every node upstream of ``target`` (what a cook of it would use)."""
        target = target or graph.output_node()
        order = graph.upstream_order(target)
        return list(self.compute_keys(graph, order).values())
