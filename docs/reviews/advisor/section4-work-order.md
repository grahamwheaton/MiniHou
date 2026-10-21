x (~15 lines in one function) than threading a cached `Graph` across [`app_frame.rs`](file:///C:/Users/Grayw/Documents/Minihou_Rust/.claude/worktrees/native-ui-cont/rust/crates/minihou-app/src/app_frame.rs#L109) (note: line 109 and subnet line 159, not line 135).
- **#6 GPU instancing vs #5 Blender transport → Swap (#6 before #5):** [`Scene::new`](file:///C:/Users/Grayw/Documents/Minihou_Rust/.claude/worktrees/native-ui-cont/rust/crates/minihou-app/src/viewport/scene.rs#L52-L56) unpacks instances via `display_geometry(g)` for every native viewport cook, whereas [`encode_message`](file:///C:/Users/Grayw/Documents/Minihou_Rust/.claude/worktrees/native-ui-cont/rust/crates/minihou-app/src/link_protocol.rs#L26-L48) and [`geometry_arrays`](file:///C:/Users/Grayw/Documents/Minihou_Rust/.claude/worktrees/native-ui-cont/rust/crates/minihou-app/src/link_protocol.rs#L92-L122) only affect active Blender link sessions. Section 2 rates viewport instancing **High** and Blender transport **Medium**, yet Section 4 reverses them.
- **#7 Docs → Split; move `README.md` fix to #1/#2:** [README.md](file:///C:/Users/Grayw/Documents/Minihou_Rust/.claude/worktrees/native-ui-cont/rust/crates/minihou-app/README.md#L97-L101) falsely states there is no spatial node canvas, picking, or textures. Leaving false architectural docs until last misleads contributors and AI agents during steps 1–6.

**(2) Real risk and size**
Medium process/prioritization risk: following the current order leaves a trivial-to-fix file truncation hazard ([`Document::save`](file:///C:/Users/Grayw/Documents/Minihou_Rust/.claude/worktrees/native-ui-cont/rust/crates/minihou-app/src/document.rs#L22)) and misleading agent docs ([README.md](file:///C:/Users/Grayw/Documents/Minihou_Rust/.claude/worktrees/native-ui-cont/rust/crates/minihou-app/README.md#L97-L101)) open while tackling broader UI/transport refactors, and prioritizes optional IPC micro-optimizations over core viewport scalability.

**(3) Smallest safe fix**
Reorder Section 4 to: **1. Native CI + `README.md` truth pass → 2. Skip empty-edge `nedges()` in `Geometry::validate` → 3. Atomic project saves → 4. UI `Graph` cache → 5. Viewport GPU instancing → 6. Blender transport optimization → 7. Node metadata consolidation.**

**(4) Test that would prove it**
- Run `cargo test --workspace` and `cargo clippy --workspace --all-targets` in CI.
- Benchmark `Geometry::validate()` on a 1M-polygon mesh with `edge.is_empty()` and assert `edge_pairs()` is not called.
- Test [`Document::save`](file:///C:/Users/Grayw/Documents/Minihou_Rust/.claude/worktrees/native-ui-cont/rust/crates/minihou-app/src/document.rs#L22) against a failing temp rename/write and verify the original file bytes remain intact.

**(5) What could not be verified**
Live GitHub Actions run history (requires GitHub API access) and actual runtime milliseconds for `Graph::from_value`, `nedges()`, GPU instancing, and Blender TCP framing (no profiling/benchmarks executed in this review-only check).

