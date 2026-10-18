# Performance audit: cooking, caching, geometry sharing (Stage 1)

Date: 2026-10-18. Branch `opus/perf-audit` (from `native-ui-cont`). Scope: `rust/crates/minihou-core` (graph.rs, session.rs,
geometry.rs) and the app cook worker (`rust/crates/minihou-app/src/lib.rs`).

## How evaluation works

- **Two evaluators share one node runner.** `Graph::cook` (graph.rs) is a one-shot pull evaluator: it recursively cooks a
  node's inputs, memoising each node once per cook (`done` map), detecting cycles (`active` set). `Session::cook`
  (session.rs) is the persistent, incremental evaluator the app uses. Both call `graph::run_node`, so results cannot drift.
- **Dependency tracking is content keyed, not dirty flags.** Before cooking, `key_of` hashes per node: type, bypass flag,
  canonical (sorted-key) JSON of the *resolved* parameters (channels/expressions applied), the keys of every input in port
  order, plus extras: frame for time-dependent nodes, file size/mtime for file readers (usd_import, image_file), the external
  input revision for Blender inputs, the python code key, the copy-to-points byte limit. A changed parameter changes that
  node's key and, through the input keys, everything downstream - and nothing upstream.
- **Cook path.** `Run::get` pulls from the target: a cache hit returns the stored `Arc<Geometry>` without visiting inputs.
  `usd_export` always runs (side effect). `loop_end` cooks its loop body through `nodes::loop::Plan`.
- **Cache.** `HashMap<u64 key, Entry{node, Arc<Geometry>, side payload, bytes, used}>`. Eviction happens only after a cook:
  newest `keep_versions` (default 2) per node, and an optional LRU byte budget. Identical nodes share one entry.
- **Stats.** `CookStats{cooked, cached, node_ms, waiting, param_diags}` per cook: this is already a per-node cook counter.

## Does a downstream change recook upstream?

Not for parameter, channel, bypass and time edits (measured below). **But structural edits did**: `Session::replace_graph`
cleared the whole cache, and the app sends `Edit::Replace` for every node add/delete/connect, every undo and redo, and
modifier-input edits. So each of those recooked the entire network.

## Geometry storage and copying

- `Geometry { npoints, topology: Arc<Topology>, point/vertex/prim: BTreeMap<String, Arc<Attr>>, detail: serde Map }`.
  Node outputs are `Arc<Geometry>` in the cache and passed to nodes as `&[Vec<Arc<Geometry>>]` - downstream nodes get
  references, never owned copies.
- `Geometry::clone()` is shallow: it copies the attribute maps of `Arc` pointers and the (small) detail map; the arrays
  are shared. A node that changes P rebuilds only P (`set_p`), topology and other attributes stay shared - copy on write at
  array granularity, already in place. ~140 `(*geo).clone()` call sites across nodes are of this shallow kind.
- A **bypassed node** made a fresh shallow clone of its input (new `Arc<Geometry>`) and cached it separately.
- Cache byte accounting counts shared arrays once per entry (documented: errs toward evicting early).

## Viewport, threading, cancellation

- The app runs one cook thread (`CookWorker`, `minihou-cook`): it drains all queued edits into one batch, applies them in
  order and cooks only the final state (playback ticks are folded with a 16 ms quiet period). The UI accepts a response only
  if its revision is the newest requested; stale results are dropped. Selection and viewport navigation never send cook
  requests; `Edit::Exposed` is a no-op for the core graph.
- The viewport receives `Arc<Geometry>` plus a `prepare` closure output (GPU-ready buffers) built on the worker thread.
- **No mid-cook cancellation existed**: a superseded batch ran to completion (including extra outputs and scene sinks)
  before the newer edit was looked at.
- Parallelism is inside nodes (rayon in validation and several kernels); nodes are cooked sequentially.

## Node registration

`nodes::get(type) -> NodeSpec{name, inputs: &[InputSpec{name, multi, optional}], params: params!(...), cook: fn}`; cook
is a pure function of inputs and resolved `Params`. Special cases are keyed by node type string in session.rs (external
inputs, lod_bake side payload, loop_end, usd_export).

## Top findings

1. Incremental, content-keyed cooking already exists and is correct for parameter/channel/time edits.
2. `replace_graph` threw the cache away: undo, redo and every structural edit recooked the whole network.
3. No cancellation: a superseded cook finished in full before the next edit was applied.
4. Geometry is already shared immutable `Arc` with array-level copy on write; the remaining gap was bypass making a copy.
5. Nodes cook sequentially and every cooked result is re-validated (O(n), parallel); no cross-node parallelism yet.

## Benchmark (debug build, `cargo test -p minihou-core --test incremental bench -- --nocapture`)

Chain: Grid 60x60 -> Scatter 2000 -> Copy to Points (box) -> Transform.

| step | before | after |
| --- | --- | --- |
| full cook | ~44 ms | ~44 ms |
| Transform edit | ~3 ms (only Transform runs) | ~3 ms |
| undo / structural replace | full recook (~44 ms, cache cleared) | ~1.3 ms (nothing cooks) |

## Changes made (Stages 2 and 3)

- Stage 2: a bypassed node returns its input's `Arc<Geometry>` itself (no copy). Test
  `branches_share_the_upstream_result_without_copying` checks topology and non-P arrays are pointer-shared by two branches,
  the upstream result is untouched, and a bypass is the same `Arc`.
- Stage 3: `replace_graph` keeps the content-keyed cache and drops only entries of removed nodes. `Session::set_cancel`
  installs a check asked before each node that has to run; the cook worker uses it to abandon a cook once a newer revision
  is sent (error `SUPERSEDED`, response discarded by revision as before). Finished nodes stay cached, so the next cook
  resumes. Tests in `rust/crates/minihou-core/tests/incremental.rs` and
  `minihou-app` `cook_ui_only_and_undo_edits_cook_nothing`.

## Not done (later stages)

Pack/Unpack, native instances, Cache node, diagnostics panel, cross-node parallel cooking, cancellation inside long nodes.
