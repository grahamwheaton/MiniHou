ocates an unreserved `Vec<(i32, i32)>` of every half-edge, runs `pairs.sort_unstable()`, and calls `pairs.dedup()`.
- [`Geometry::validate()`](file:///C:/Users/Grayw/Documents/Minihou_Rust/.claude/worktrees/native-ui-cont/rust/crates/minihou-core/src/geometry.rs#L240-L272) eagerly evaluates `self.nedges()` inside the array literal at [line 263](file:///C:/Users/Grayw/Documents/Minihou_Rust/.claude/worktrees/native-ui-cont/rust/crates/minihou-core/src/geometry.rs#L262-L264) before checking whether `self.edge` has any attributes.
- Validation runs after every non-bypassed node cook in [`run_node_outputs()`](file:///C:/Users/Grayw/Documents/Minihou_Rust/.claude/worktrees/native-ui-cont/rust/crates/minihou-core/src/graph.rs#L283) and [`Session`](file:///C:/Users/Grayw/Documents/Minihou_Rust/.claude/worktrees/native-ui-cont/rust/crates/minihou-core/src/session/run.rs#L128-L132) (and inside individual nodes like `wrangle`, `array`, `primitives`, etc., often duplicating the check on a single cook).

**(2) Real risk and magnitude**
- **High CPU & allocation overhead on dense meshes:** For a mesh with $V$ vertices/corners (e.g., 1M quads = 4M corners), `edge_pairs()` allocates ~32 MB (with multiple reallocations since `Vec::new()` is unreserved) and performs an $O(V \log V)$ sort on every node evaluation, even for trivial point/attribute operations (`transform`, `wrangle`, `color`) that neither modify topology nor carry edge attributes.
- Note: [`merge.rs`](file:///C:/Users/Grayw/Documents/Minihou_Rust/.claude/worktrees/native-ui-cont/rust/crates/minihou-core/src/nodes/merge.rs#L54) also eagerly calls `g.topology.nedges()` per input even when all `edge` tables are empty.

**(3) Smallest safe fix**
In [`Geometry::validate()`](file:///C:/Users/Grayw/Documents/Minihou_Rust/.claude/worktrees/native-ui-cont/rust/crates/minihou-core/src/geometry.rs#L262-L270), skip computing `self.nedges()` when `self.edge.is_empty()` (or pass a closure `|| self.nedges()` / check `if !table.is_empty()` before computing `n`).
For a complete fix across edge-using nodes and repeated cooks, cache `OnceLock<Vec<(i32, i32)>>` (or `OnceLock<usize>`) on `Topology` (implementing `PartialEq`/`Clone` manually or ignoring the cache cell).

**(4) Test to prove it**
- **Functional/Unit test:** A test in [`geometry.rs`](file:///C:/Users/Grayw/Documents/Minihou_Rust/.claude/worktrees/native-ui-cont/rust/crates/minihou-core/src/geometry.rs#L294-L306) asserting that `validate()` succeeds on empty `edge` tables without populating a lazy edge cache on `Topology` (e.g., `assert!(g.topology.cached_edges.get().is_none())`), and populates it only when an edge attribute is present.
- **Benchmark/Regression test:** Cook a 5-node chain (`subdivide` → 4× `transform`) on a ~100k+ polygon mesh and assert cook time drops significantly.

**(5) What could not be verified**
- Exact wall-clock frame/cook time savings on production scenes, as no benchmarks or profiler runs were executed during this static check.

