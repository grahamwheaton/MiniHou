# MiniHou Rust Performance Plan

**Status:** Proposal — benchmark first, migrate incrementally  
**Date:** 2026-10-14  
**Primary objective:** Make MiniHou a high-performance geometry-processing engine for Blender, not merely an alternative node editor.

## 1. Product goal

MiniHou should be useful when an artist sends a *heavy* Blender mesh into MiniHou, performs procedural operations, and receives the result back. If the whole workflow is consistently slower than Blender Geometry Nodes for equivalent work, there is little reason to use it.

Performance must therefore be judged **end to end**: Blender mesh extraction → transfer → cook → transfer back → Blender mesh update. Improving the node editor's appearance or removing Chromium does not advance this goal directly.

**Direction:** Keep the current UI, graph documents, Blender add-on, MCP interface, and Python node ecosystem working while introducing a Rust compute backend for demonstrated hotspots. Consider a Rust-native UI only after measurable compute improvements.

## 2. Existing architecture to preserve

The repository already contains useful foundations:

- **Geometry:** `minihou/geometry.py` — columnar NumPy attributes by point/vertex/primitive, separate topology, immutable cooked results and shallow array sharing.
- **Incremental evaluator:** `minihou/evaluator.py` — content-addressed node keys, pull-based evaluation and a memory-budgeted cache.
- **Cook orchestration:** `minihou/link/session.py` — one evaluator/cook thread per session, edit coalescing, cancellation and live updates.
- **Acceleration:** `minihou/accel.py` and `minihou/nodes/_kernels.py` — optional Numba fast paths; the default PyInstaller build currently omits Numba and SciPy unless `--with-accel` is used.
- **Blender transport:** `minihou/link/protocol.py` — binary array messages, changed-array updates and unchanged-topology elision.
- **Editor and integrations:** `minihou/web/`, `blender_addon/minihou_link/`, MCP and custom Python nodes.

Do **not** rewrite or discard these subsystems before benchmarks identify an actual bottleneck. In particular, preserve graph JSON compatibility and the existing live Blender workflow.

## 3. First milestone: reliable Blender-vs-MiniHou benchmarks

Create a repeatable benchmark harness and store results in machine-readable form (JSON/CSV) with hardware, software versions and test parameters.

### Datasets

- Mesh scales: approximately **1 million**, **5 million**, and **10 million** polygons, within available RAM.
- Both clean regular topology and realistic irregular meshes.
- Attribute-light versus attribute-heavy geometry (UVs, normals, material IDs, groups and custom attributes).
- Repeated modifier edits, cold startup, warm caches and structural/topology changes.
- Include representative production-like graphs, not only synthetic kernels.

### Comparable operations

Test equivalent settings, resolution, precision and output quality for:

1. Transform, normal generation and attribute processing.
2. Noise / displacement on millions of points.
3. Scattering and instancing; distinguish preserved instances from realised geometry.
4. Subdivision, topology editing and mesh filtering.
5. Booleans, proximity queries, raycasts and complex selection.
6. Fracturing or other heavy specialised processing when a genuinely equivalent Blender setup exists.

### Time every stage separately

- **Blender input:** evaluated-mesh extraction, attributes and topology serialisation.
- **Input transport:** bytes, copies and transfer time.
- **Cooking:** each node's time, graph scheduling overhead, cache-hit behaviour and peak resident memory.
- **Output transport:** bytes and transfer time.
- **Blender output:** decoding, mesh creation/update, depsgraph impact.
- **Total artist-visible latency:** elapsed time from changing a control to correct visible output.

Collect warm/cold runs, median and spread across several repetitions. Test the **current default packaged NumPy build** and a **Numba-accelerated build** before declaring Python slow. Warm JIT ahead of measured runs, but report startup/JIT costs separately. Record Blender's standalone timings with equivalent operations and make clear when one side benefits from caching or instancing and the other does not.

**Baseline note:** `docs/PLAN.md` reports a historical test in which Blender edge computation for 1M quads took about 120 ms and an engine-side NumPy calculation took about 1.9 s. Reproduce on current builds before treating those figures as current performance.

### Initial targets (targets, not existing claims)

- Achieve **at least 2× faster end-to-end** than an equivalent Blender workflow for *selected* heavy operations, not every operation.
- Maintain equivalent geometry quality, valid topology, required attributes, deterministic results where expected, and compatibility with Blender.
- Keep cached slider edits responsive and minimise needless transfers.
- Demonstrate memory usage that remains practical on multi-million-element meshes.

If benchmarks show Blender is already faster for an operation, preserve or delegate to Blender rather than porting it blindly.

## 4. Migration strategy: Rust compute modules behind Python

### Phase A — Instrument and remove avoidable overhead

- Add end-to-end timings to the Blender add-on and engine; display input/cook/output times separately.
- Profile actual memory allocations and copies, not just CPU time.
- Enable and benchmark acceleration-friendly packaging; test both default and `--with-accel`.
- Identify slow Python loops, repeated topology construction, excessive dtype conversions, serialisation and cache misses.
- Verify that input meshes are only re-sent when changed and output updates send only changed arrays where possible.
- Report both full-resolution and proxy-level performance explicitly.

**Exit criterion:** ranked hotspots, reproducible benchmark results and a clear Rust pilot candidate.

### Phase B — Build one Rust kernel

Create a small Rust crate exposed to Python using **PyO3 + maturin** (or another measured low-overhead binding). Start with a proven bottleneck, ideally one of:

- Topology adjacency / edge indexing / connectivity;
- Boolean broad-phase or another profiled mesh-intersection stage;
- Nearest-point / spatial-query construction;
- High-volume attribute operations where NumPy/Numba cannot already match the performance.

Select based on Phase A numbers. Do not start with an easy but already-fast transform just to prove Rust works.

**Interop requirements:**

- Read contiguous NumPy buffers without copies when safely possible; document ownership, lifetime and mutability.
- Preserve MiniHou's point/vertex/primitive attributes, material IDs and topology semantics.
- Release Python's GIL during substantial safe native computation where supported.
- Return correct errors and support cancellation at reasonable boundaries.
- Keep the existing Python implementation as a fallback and test oracle.
- Add parity tests covering empty, degenerate, large and malformed meshes.

**Exit criterion:** a demonstrable performance win for the chosen complete operation (including Python↔Rust overhead), with passing parity tests.

### Phase C — Parallel CPU execution

Use **Rayon** or another profiled Rust parallelism mechanism where it helps:

- Parallelise within expensive geometry kernels.
- Investigate concurrent evaluation of independent DAG branches.
- Preserve deterministic outputs where required, cache consistency, error handling and cancellation.
- Avoid oversubscription between Rayon, NumPy, Numba, Blender and physics libraries.
- Honour an explicit worker-count/memory-budget setting; benchmark single-thread vs multi-thread scaling.

**Important:** the current `Evaluator` is explicitly single-threaded and `Session` owns a cook lock. A parallel graph scheduler requires a deliberate concurrency design, not merely replacing a `for` loop.

### Phase D — Native shared geometry core, only if justified

If multiple Rust kernels provide consistent gains, evaluate moving hot geometry storage and graph scheduling into an internal Rust library:

- Reference-counted, immutable buffers; selective copy-on-write for modified attributes.
- Attribute-preserving topology operations, compact indexes where valid, explicit large-mesh index limits.
- Bounded caches and intermediate-memory pressure control.
- Stable Python-facing adapter so existing Python custom nodes can continue to run.
- Existing JSON graphs, MCP, Blender add-on and editor remain unchanged externally.

Avoid an all-at-once rewrite of the current node registry or extensive built-in node catalogue.

### Phase E — Optional GPU compute

Investigate **wgpu compute** (or another suitable backend) only for profiled, highly parallel workloads such as noise, deformation or large attribute passes. Benchmark GPU upload/download, resident buffer reuse, dispatch overhead and device-memory limits.

For workflows that immediately return data to Blender, GPU compute is only beneficial if the transfer and synchronisation costs do not cancel the kernel speedup. Keep a CPU fallback. RTX 4090 machines are a useful test environment, but not a reason to assume GPU acceleration wins every task.

### Phase F — Desktop UI later

MiniHou already has a packaged desktop mode using pywebview/WebView2. Replacing Chromium with a Rust-native editor (for example, egui + wgpu) may be worthwhile for UX or viewport scaling, **but it is not the performance priority** unless profiling identifies the current UI as the bottleneck.

## 5. Blender integration is part of the performance product

A fast Rust kernel is not sufficient if extracting and rebuilding a 10M-polygon Blender mesh dominates the round trip.

Explore, without prematurely breaking compatibility:

- Persistent engine-side mesh identities and versioned buffers.
- Position/attribute-only deltas when topology is unchanged.
- Incremental dirty-range updates where correctness and Blender APIs allow them.
- Instancing retained as long as possible; avoid accidental realisation.
- Asynchronous compute with safe Blender main-thread updates.
- Explicit render/bake synchronisation: never claim a fresh result until cooking and applying it have completed.
- Time and memory diagnostics visible to users.

The existing TCP binary array protocol is a strong starting point. Benchmark it before replacing it with shared memory or an alternative transport.

## 6. Correctness and release gates

Every new backend must:

- Pass existing engine, Blender, graph and integration tests.
- Match reference output within clearly defined numeric tolerances, or document a justified difference.
- Preserve selections/groups, UVs, material identity, custom attributes and mesh orientation where the operation promises them.
- Handle empty, non-manifold and degenerate inputs according to the node's existing contract.
- Avoid crashes, leaked memory and unbounded allocations on heavy scenes.
- Build and package reliably on supported operating systems; confirm the accelerated backend is actually present in release builds.
- Allow rollback/fallback to existing implementations.

Track performance regressions in CI where stable, and run controlled hardware benchmarks separately where CI hardware is too variable.

## 7. Proposed implementation order

- [ ] Add a Blender-vs-MiniHou benchmark suite and performance report.
- [ ] Measure current default build against NumPy+Numba accelerated build.
- [ ] Instrument extraction, transfer, cook, output and memory timings.
- [ ] Profile representative heavy-model graphs; rank the top five hotspots.
- [ ] Choose and implement **one** Rust/PyO3 pilot kernel, with Python fallback.
- [ ] Prove correctness and complete-operation performance improvement.
- [ ] Optimise Blender round-trip and geometry residency based on measurements.
- [ ] Add measured CPU parallelism to suitable kernels and/or independent graph branches.
- [ ] Port more high-value operations one by one.
- [ ] Evaluate GPU compute only for operations with clear end-to-end benefit.
- [ ] Reconsider a Rust-native interface after the geometry performance milestones.

## 8. Decision rule

**Do not measure success by the percentage of MiniHou rewritten in Rust.**

Measure it by whether artists can feed in heavy Blender geometry, perform useful operations **faster than Blender at comparable quality**, and receive the correct result without disruptive waiting or excessive memory usage. Preserve Python wherever it already performs well. Port only where data proves the benefit.
