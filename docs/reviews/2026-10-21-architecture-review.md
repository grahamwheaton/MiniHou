# MiniHou — Architecture & Code Review (ChatGPT, 2026-10-21)

Branch: `native-ui` · Review date: 21 October 2026

## 1. Overall architecture

MiniHou has grown into a substantial application. The `native-ui` branch contains 1,584 tracked files, including a native Rust editor, an independent geometry engine, a Python implementation, Blender integration and extensive tests.

The most important architectural decision is already in place: the geometry-processing engine is separate from the user interface. That gives MiniHou a much better foundation for handling complex geometry than tying computation directly to a browser or UI event loop.

Layers: Native MiniHou application (Rust, egui, wgpu: node editor, parameters, 3D viewport, modelling tools) → document state / cook requests / results → Rust geometry engine (`minihou-core`: graph evaluator, node registry, incremental cache, geometry data, modelling kernels, USD/attributes) → Blender integration (add-on, geometry exchange, live modifiers) and Python compatibility (legacy nodes, PyO3 bridge, Python execution).

### What looks good

- Independent engine: `minihou-core` can operate without Blender or the editor.
- Incremental evaluation: changing a downstream parameter should not recalculate every upstream node.
- Shared geometry: attributes and topology use reference-counted storage, reducing unnecessary copies.
- Background cooking: expensive geometry computation is separated from the UI thread.
- Modular node implementations: individual geometry operations are increasingly isolated rather than living in one giant execution function.

## 2. Main findings

| Priority | Finding | Why it matters |
| --- | --- | --- |
| High | Graph rebuilt every UI frame | Increasingly expensive with large node networks |
| High | Edge topology repeatedly recalculated | Unnecessary computation on dense meshes |
| High | Packed instances expanded for viewport rendering | Large memory and performance penalty for repeated geometry |
| High | Native branch lacks automatic CI coverage | Regressions can reach the branch without tests |
| Medium | Saves aren't atomic | A failed write can damage the original project file |
| Medium | Blender geometry transfer makes large copies | Potential CPU and memory bottleneck |
| Medium | Documentation is inconsistent | Particularly troublesome for AI coding agents |

### Finding 1 — Graph rebuilt during rendering

Performance · Native UI

In `rust/crates/minihou-app/src/app_frame.rs` (line 135), the UI's `update()` reconstructs the graph model with `Graph::from_value(self.editor.doc.value())` on every frame, even when the user is only moving the camera or cursor. Subnet editing can trigger additional reconstruction.

Recommendation: Maintain a persistent graph snapshot, invalidate it only when the document changes, and reuse it for rendering and input handling.

### Finding 2 — Expensive edge counting

Performance · Geometry engine

In `rust/crates/minihou-core/src/geometry.rs` (lines 108–132), `nedges()` calls `edge_pairs()`, which builds, sorts and deduplicates edge pairs. Geometry validation requests the edge count even when the geometry has no edge attributes, and validation runs after node execution. For a mesh with millions of polygons this can repeat expensive operations along the node chain.

Recommendation: Cache edge topology against immutable topology data. At minimum, avoid calculating it when the edge attribute table is empty.

### Finding 3 — Packed geometry loses its advantage in the viewport

Performance · Instances / Archviz

The geometry engine supports packed geometry and shared source objects. However, `rust/crates/minihou-app/src/viewport/scene.rs` (lines 52–55) calls `display_geometry()`, which recursively unpacks instances into ordinary geometry. Scattering 10,000 copies of a detailed tree means individual expanded mesh data for each copy, and higher memory use.

Recommendation: Add a GPU-instanced rendering path using one source mesh plus per-instance transform buffers. Keep CPU expansion as a fallback for unsupported geometry features.

### Finding 4 — Native app isn't automatically tested on branch pushes

Reliability · Build system

The CI workflow (`.github/workflows/ci.yml`, lines 3–7) doesn't include `native-ui` in its push triggers. The Rust workspace's default members exclude `minihou-app`, and the CI commands don't explicitly select the full workspace. No GitHub Actions runs were found for the `native-ui` branch.

Recommendation: Add a native-app CI job that runs tests, compilation and Clippy checks on every push, with at least Windows and Linux coverage.

### Finding 5 — Project files are written directly

Reliability · Persistence

`rust/crates/minihou-app/src/document.rs` (line 22) uses `std::fs::write()` directly against the destination. If a write fails halfway through, the previous valid file may already have been truncated.

Recommendation: Write a temporary file, flush it, then replace the destination safely. Add backup/recovery handling and check for external file modifications.

### Finding 6 — Blender transfers involve avoidable copying

Performance · Blender integration

The link protocol (`rust/crates/minihou-app/src/link_protocol.rs`) already supports attribute-level differences. However, constructing geometry arrays, packing them into a blob and assembling the final network frame involves several large allocations and copies.

Recommendation: Profile peak memory and transfer latency, then consider streaming or vectored writes. Shared-memory transport could be worthwhile for local Blender-to-MiniHou transfers, but should be benchmarked before replacing the current protocol.

## 3. Architecture recommendations

Preserve the current separation between Rust core, editor and Blender rather than undertaking another rewrite.

- **Keep Rust as the primary geometry engine.** The independent core, immutable/shared geometry and incremental caching are strong foundations.
- **Make node packages the standard extension mechanism.** Adding a node should require minimal changes to central dispatch, menus and metadata. Add contract tests for every node package.
- **Reduce duplicated Python/Rust responsibilities.** Define a clear source of truth for node parameters, documentation and graph compatibility. Automate validation of the generated metadata as part of CI.
- **Continue splitting UI features into focused modules.** Keep extracting behaviour from large orchestration files such as `app_frame.rs`. Aim for cohesive responsibilities rather than an arbitrary 1,000-line limit.
- **Housekeeping:** `rust/crates/minihou-app/README.md` still describes the editor as a limited prototype without a spatial node canvas. Update it, because AI agents may use it to understand the project.

## 4. Recommended work order

| Order | Work | Benefit |
| --- | --- | --- |
| 1 | Add native CI and regression testing | Protect ongoing development |
| 2 | Remove unnecessary topology validation work | Faster dense-mesh cooking |
| 3 | Cache the UI's graph model | Better node-editor responsiveness |
| 4 | Implement atomic project saves | Protect project files |
| 5 | Benchmark and optimise Blender transport | Faster geometry exchange |
| 6 | Introduce GPU instancing | Scale to large archviz scenes |
| 7 | Consolidate documentation and node metadata | Easier maintenance and AI development |

## 5. Assessment

- Architecture foundation: Strong
- Modularity: Good, improving
- Large-geometry scalability: Promising, needs profiling
- Automated quality assurance: Needs attention
- Maintainability: Manageable, growing complexity

This was a static review of the branch's architecture and representative implementation paths, not a line-by-line audit of all 1,584 files. The Rust tests were not executed and the application was not benchmarked, so the performance findings are code-level risks rather than measured slowdowns.

Main conclusion: MiniHou doesn't need another architectural overhaul. The next step is to tighten testing and remove specific hot-path inefficiencies before adding more modelling features.
