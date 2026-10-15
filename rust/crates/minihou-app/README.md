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
- Registry metadata selects numeric sliders, drag values, enum combos, booleans,
  text and vec3 controls. Hover a parameter for its documentation. Hard bounds
  clamp edits; soft maxima only limit the slider, and the adjacent drag value
  can exceed them. Parameters absent from metadata retain generic controls.
- Save or Ctrl+S writes to the opened path. File > Open and Save As use native
  dialogs. Cancel leaves the document unchanged; failed saves retain the old
  path/save point. The window title shows the filename and an unsaved `*`.
- Edit > Undo / Redo or Ctrl+Z / Ctrl+Y restores parameter edits, bypass toggles
  and display changes. A numeric pointer drag commits one step on release.
  New edits clear redo. Undo preserves original JSON values, absent members
  and token spelling. Opening a graph replaces the document, worker and history.
- Bottom bar reports Session cook/cache node ids, elapsed cook time and errors.
  `cooking...` appears while awaiting the latest revision. Failed cooks keep the
  previous good mesh visible; correcting the parameter recovers it.

The document retains the original serde_json::Value and original text. Saves
without edits are byte-exact except that an input UTF-8 BOM is removed. Saves
never add a BOM and preserve existing line endings (including CRLF). Existing edits replace only their JSON token;
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
No new minihou-core APIs were needed. Native dialogs add rfd 0.15.4; eframe
continues to use its explicitly named features with defaults disabled.

A dedicated worker owns the Session. Parameter, bypass and display commands
are drained in order, then the latest queued state is cooked once. Responses
carry a revision, Arc<Geometry>, statistics and errors. Stale responses are
ignored; a single response slot bounds queued result memory. CPU viewport
layout and normal generation also run on the worker; GPU uploads remain on
the UI thread. The renderer and shaders are unchanged. Undo/redo sends a
replacement graph to the worker, resetting its cache to restore omitted values
exactly. In-flight cooks are not cancelled. Closing/replacing a worker lets its
current cook finish without blocking the UI.

## Checks

    cargo test --manifest-path rust/Cargo.toml -p minihou-app
    cargo clippy --release --manifest-path rust/Cargo.toml -p minihou-app --all-targets -- -D warnings

Unit tests cover BOM/CRLF I/O, metadata coverage for all eight node types,
range/clamp rules, lossless edits, coalesced cooks, stale results, failure/recovery,
worker channel delivery, exact undo/redo, drag grouping, save points/Save As,
file replacement state, camera math and selftest. CLI tests check one-line JSON,
unchanged input and nonzero failures; the backend test guards enabled wgpu
backends. Workspace default-members excludes both GUI crates.

## Not done / next session

- No interactive window or GPU rendering validation was performed in this
  session. Check all shading modes, resize/DPI, camera controls, disabled
  tooltips and Save in a desktop session.
- Interactive 4M-quad responsiveness remains unmeasured. Cooking, layout and
  normals are off-thread, but large GPU uploads can still stall a UI frame.
  Cook statistics exclude layout/normal generation/GPU upload time.
- In a desktop session, drag a numeric slider and undo once; type a value above
  a soft maximum; test enums/docs, Ctrl+Z/Y, bypass undo, cooking/error recovery,
  Open/Save As/cancel and filename/dirty title updates. Open while a cook is
  pending to confirm old results cannot enter the replacement document.
- No node creation/deletion, rewiring, spatial node canvas, picking or
  simultaneous multi-document handling.
- No unsaved-close prompt, external-file conflict detection or atomic save.
- No textures/materials, point-cloud drawing, culling, multi-object scene
  management or additional Rust node implementations.
