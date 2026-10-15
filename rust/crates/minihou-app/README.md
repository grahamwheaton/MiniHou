# MiniHou native editor slice

Native Rust executable; no Python process, web server, or browser.

From the repository root:

    cargo build --release --manifest-path rust/Cargo.toml -p minihou-app
    rust/target/release/minihou-app.exe rust/crates/minihou-app/tests/fixtures/small.json
    rust/target/release/minihou-app.exe --selftest rust/crates/minihou-app/tests/fixtures/small.json

Use any graph containing Rust-supported grid, box, transform, normal, noise,
merge, null and output nodes. The graph must specify display or have an output.
The selftest cooks, bypasses the second node in file order (the only node when
there is one), cooks again, and emits one JSON line. Counts describe the second
cook. It never creates a window or saves the graph.

## Controls and state

- Click a node's label to edit its declared parameters. The radio selects the
  display node independently. Unsupported parameters are disabled with a tooltip.
- Bypass passes the first input through, or produces empty geometry for a source.
- Left drag: orbit. Middle/right drag or Shift+left drag: pan. Wheel: zoom.
- F or Frame: frame the current mesh. F is suppressed while editing text.
- Top bar: Solid, Shaded, Wire, Shaded+Wire.
- Save or Ctrl+S writes to the opened path. The asterisk indicates unsaved edits.
- Bottom bar reports Session cook/cache node ids, elapsed cook time and errors.
  A failed cook clears the viewport; correcting a parameter recovers it.

The document retains the original serde_json::Value and original text. Saves
without edits are byte-exact. Existing edits replace only their JSON token;
omitted parameters/flags are inserted into their enclosing object. Unknown
fields, object order, whitespace, comments stored as JSON strings, positions,
exposed lists and unrelated flags remain intact. Display selection is saved too.
The Graph is never used to serialize the file.

## Dependencies and rendering

eframe, egui and egui-wgpu are pinned/resolved to 0.33.3; wgpu is 27.0.1,
as required by eframe 0.33.3. The app imports wgpu through egui-wgpu's re-export.
The existing standalone spike uses wgpu 30.0.1 and is unchanged. The app's
dependency graph contains only wgpu 27; it shares eframe's device and queue.

The spike's shaders and draw setup are ported to wgpu 27. An offscreen
color/depth target is displayed as an egui native texture. Mesh changes upload
buffers while reusing pipelines; ordinary UI frames do not recook or rebuild
layout. Uses core preview::layout, all triangles/edges, two-sided rendering,
and point N or point_normals. No culling or extra geometry optimizations.
No new minihou-core APIs were needed.

## Checks

    cargo test --manifest-path rust/Cargo.toml -p minihou-app
    cargo clippy --release --manifest-path rust/Cargo.toml -p minihou-app --all-targets -- -D warnings

Unit tests cover parameter kinds, lossless saves/edits, insertion and bypass,
escaped strings/keys, display selection, invalid edits, cook recovery,
camera math and selftest. CLI tests check one-line JSON, unchanged input and
nonzero failures. Workspace default-members excludes both GUI crates.

## Not done / next session

- No interactive window or GPU rendering validation was performed in this
  session. Check all shading modes, resize/DPI, camera controls, disabled
  tooltips and Save in a desktop session.
- Interactive 1M-quad responsiveness is unmeasured. Cooking, layout, normals
  and uploads are synchronous, batched once per UI frame; cook statistics
  exclude layout/normal generation/GPU upload time. No worker/cancellation.
- No node creation/deletion, rewiring, spatial node canvas, picking, undo/redo,
  multi-document handling, file picker or Save As.
- No unsaved-close prompt, external-file conflict detection or atomic save.
- No textures/materials, point-cloud drawing, culling, multi-object scene
  management or additional Rust node implementations.
