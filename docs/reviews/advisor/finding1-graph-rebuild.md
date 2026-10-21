e scoped `Value`, and calling [`Graph::from_value`](file:///C:/Users/Grayw/Documents/Minihou_Rust/.claude/worktrees/native-ui-cont/rust/crates/minihou-core/src/graph.rs#L64-L139) at line 159) and in [`ui::network::show`](file:///C:/Users/Grayw/Documents/Minihou_Rust/.claude/worktrees/native-ui-cont/rust/crates/minihou-app/src/ui/network.rs#L211-L212).

**(2) Real risk and magnitude**
- **Medium-to-High CPU/allocation overhead on large documents:** [`Graph::from_value`](file:///C:/Users/Grayw/Documents/Minihou_Rust/.claude/worktrees/native-ui-cont/rust/crates/minihou-core/src/graph.rs#L64-L139) parses all nodes, clones every node's `inputs`, `endpoints`, `params`, `channels`, and `locked_params`, scans all `nodes` to resolve `modifier_input` links (lines 117–130), validates `controller` nodes by cloning their map (line 78), and deep-clones the entire document `serde_json::Value` into `source: v.clone()` (line 138).
- Because this runs every `egui` repaint—including 60–120 Hz camera orbits, cursor movement, and hover checks—a document with hundreds of nodes or embedded subnets incurs avoidable heap churn and frame-time jitter on the UI thread.

**(3) Smallest safe fix**
- Cache the parsed [`Graph`](file:///C:/Users/Grayw/Documents/Minihou_Rust/.claude/worktrees/native-ui-cont/rust/crates/minihou-core/src/graph.rs#L50-L56) (and a monotonic revision `u64`) inside [`Document`](file:///C:/Users/Grayw/Documents/Minihou_Rust/.claude/worktrees/native-ui-cont/rust/crates/minihou-app/src/document.rs#L10), rebuilding it only in [`Document::parse`](file:///C:/Users/Grayw/Documents/Minihou_Rust/.claude/worktrees/native-ui-cont/rust/crates/minihou-app/src/document.rs#L12-L18) and [`Document::set`](file:///C:/Users/Grayw/Documents/Minihou_Rust/.claude/worktrees/native-ui-cont/rust/crates/minihou-app/src/document.rs#L45-L69) when the document actually mutates.
- Expose `doc.graph() -> &Graph` and pass that borrow into [`View`](file:///C:/Users/Grayw/Documents/Minihou_Rust/.claude/worktrees/native-ui-cont/rust/crates/minihou-app/src/app_frame.rs#L124) instead of calling `Graph::from_value` in [`App::update`](file:///C:/Users/Grayw/Documents/Minihou_Rust/.claude/worktrees/native-ui-cont/rust/crates/minihou-app/src/app_frame.rs#L109).

**(4) Test that would prove it**
- A unit test in [document.rs](file:///C:/Users/Grayw/Documents/Minihou_Rust/.claude/worktrees/native-ui-cont/rust/crates/minihou-app/src/document.rs) asserting that `std::ptr::eq(doc.graph(), doc.graph())` holds across repeated reads without edits, and that `doc.revision()` increments and `doc.graph()` updates only after [`Document::edit`](file:///C:/Users/Grayw/Documents/Minihou_Rust/.claude/worktrees/native-ui-cont/rust/crates/minihou-app/src/document.rs#L150-L235) or replacement.

**(5) What could not be verified**
- Actual frame-time impact (in milliseconds) on production-sized `.minihou` files during interactive viewport navigation, as no runtime benchmark was executed in this review-only check.

