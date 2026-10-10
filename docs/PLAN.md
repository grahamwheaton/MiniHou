# MiniHou plan and design notes

## Goal

A standalone procedural geometry engine (Python, numpy only for now) that live-links into Blender 5.1 on Windows.
Houdini-flavoured: attribute tables, a DAG of typed nodes stored as JSON, pull-based cooking with a cache, nodes
that are plain Python functions.

## Milestone 1 (this repo state)

1. Engine core: geometry model, node decorator/registry, cached evaluator, JSON graph load/save.
2. Nodes: grid, transform, noise, scatter, copy_to_points, merge, output.
3. Socket server + protocol that sends only changed attributes and skips unchanged topology.
4. Blender add-on: listener thread -> timer -> `foreach_set`, proxy object + Geometry Nodes modifier, parameter forwarding with
   feedback-loop guards.
5. Benchmark: 1M-point position array over the socket, timing the Blender-side write.
6. Unit tests for evaluator, caching, nodes (plus protocol, link, add-on).

## Key design decisions

**Geometry** (`minihou/geometry.py`). Points, vertices, primitives and detail; each attribute a numpy array
(float32 / int32 / bool; shape `(n,)` or `(n, k)`). Topology is `vertex_point` + CSR `prim_offsets`, kept separate from attributes
so "topology unchanged" is one comparison. Cooked geometry is **frozen** (arrays read-only). A node modifies a
shallow `copy()` and *replaces* arrays; untouched arrays remain the same object. That makes the cache safe to share
and lets the link layer detect "unchanged" by identity.

**Caching** (`minihou/evaluator.py`). Merkle keys: `hash(node type, node version, canonical params, input keys)`. Keys are
computed for the whole upstream set first, without cooking. Cooking then pulls from the target and stops at the first cache hit, so an
unchanged branch is never visited. LRU with a byte budget that counts shared arrays once; the live link
prunes entries that are no longer upstream of the display node after each cook. Contract: nodes are pure functions
of (inputs, params).

**Node SDK** (`minihou/sdk.py`). `@node("name", inputs=[...], params=[Float(...), ...])` on a function
`fn(*inputs, **params) -> Geometry`. Signature mismatches fail at import. Param kinds: float, int, bool, str, vec3, enum
with hard `min/max`, soft UI hints, labels. `multi=True` inputs receive a list (merge).

**Wrangle readiness.** The API is already sufficient for `@P.z += sin(@P.x)`: a node with one `Str("snippet", kind="code")`
parameter and one geometry input. The snippet is part of the cache key. The node compiles it to a numpy function (memoised on the
source text) and runs it over the attribute arrays. `tests/test_nodes.py::WrangleReadinessTests` exercises that shape with a
toy implementation. Left for the real thing: the expression parser/compiler, attribute reads/writes metadata on node types (for graph analysis
and UI), and an `Input`/`Output` port-type system beyond `geometry`.

**Live link** (`minihou/link/`). TCP on 127.0.0.1 (portable on Windows; no named pipes / AF_UNIX).
Frame = `MHL1 | header_len | blob_len | JSON header | 8-byte-aligned raw array blob`; the receiver reads into one bytearray and exposes arrays
via `np.frombuffer` (no copies). The `geo` message carries only changed arrays, a manifest of all current arrays (so removals are
explicit), counts, and the two topology arrays only when they changed. Diffing happens **per client at send time**
(against what that client last received) in a writer thread that sends the latest result only, so slider drags coalesce
correctly. Cooking runs on its own thread and also coalesces edits.

**Blender side** (`blender_addon/minihou_link/`). The socket thread only enqueues (merging pending `geo` updates). A
`bpy.app.timers` function drains the queue and does all bpy work; `mesh_writer.assert_main_thread()` guards the bpy entry points.
Output goes to a hidden `MiniHou_Output` mesh; `MiniHou_Proxy` has a Geometry Nodes modifier whose group reads the output
object with *Object Info* (relative transform). The group's inputs are the exposed parameters.

Feedback-loop guards: (1) the depsgraph handler only sets a flag; (2) forwarding is a diff against the values the engine is known
to have, so updates triggered by our own mesh writes forward nothing; (3) the engine never echoes parameter changes to the client that sent them; (4) engine-pushed
values are recorded as known before they are written to the modifier; (5) forwarding is rate limited and a circuit breaker trips at
>120 forwarded values per second; (6) the tick is not re-entrant. (2) has a regression test for the float32 round-trip
(Blender stores 0.4 as 0.4000000059604645; an exact comparison would forward a phantom edit after every connect).

## Findings from measurement

* `mesh.vertices.foreach_set("co")` is ~170x slower than writing the `position` attribute's data; `loops.foreach_set("vertex_index")`
  is ~250x slower than the `.corner_vert` attribute. Both fast paths are used, with fallbacks.
* At 1M points the Blender end costs ~1 ms and the socket ~6 ms; the engine's numpy noise (~276 ms) dominates a slider edit.
  A 100k-point edit is ~28 ms end to end.
* Review findings fixed after the first push: float32 round-trip phantom edits; `ensure_objects()` re-assigning the modifier node group on every update
  (Blender returns a new Python wrapper per access, so `is` checks are always False; it tagged the depsgraph each time); unmeasured
  performance claims in the README; the benchmark ignoring `MINIHOU_ENGINE_PYTHON`; restarting the engine on a busy port.

## Next steps (not started)

1. ~~Noise speed~~ done: optional Numba backend (see README, "Speed"). Remaining: a wrangle compiler can target the same machinery
   (`@njit` on generated code, with the numpy fallback); cheaper topology rebuilds in Blender.
2. Wrangle node (parser + numpy codegen), attribute-flow metadata on node types.
3. More nodes: delete/blast, group/attribute-from-group, attribute transfer / ray, boolean-free modelling basics, instancing packed prims.
4. Multiple outputs / several display nodes; detail attributes and normals to Blender; undo-friendly parameter writes.
5. Time/frame context in the cache key for animated graphs; a `bpy.msgbus`/frame-change hook.
6. Web editor (explicitly deferred).
7. Package the add-on as a Blender extension (manifest + licence choice is yours) and add CI that runs the `bpy`-module tests.

## Acceleration design (added after the benchmark)

Profiling at 1M points showed noise (275 ms), scatter (1.2 s: many numpy temporaries) and copy_to_points (570 ms: per-copy rotation matrices) as the
engine-side costs; the link and the Blender write were already ~8 ms. `minihou/accel.py` + `nodes/_kernels.py` add fused Numba kernels behind the
existing node functions. Rules: optional; backend chosen by availability only; same algorithm and float32 coordinate math as the numpy code (kernel
tests mutation-checked); launches serialised by a lock; any kernel exception disables acceleration and re-runs the numpy path; seeds reduced to 32 bits in
both paths. A thread-pool-over-numpy alternative gave only 2.8x and was not adopted.
