# Rust node search and viewport stream insertion

## What changed

The native Tab menu uses the web editor's `nodemenu.js` token scoring: lowercase tokens split on whitespace, comma and underscore; every token must match; exact label/name (0), prefix (1), word start (2), substring (3), exact keyword (4), keyword prefix (5), menu/category (6), and description (7). Scores add, then the shorter label and alphabetical label break ties. Search results are flat and the first result is selected. Up/Down wrap, Enter adds, Escape closes, and Tab while the menu is open is consumed. The empty query shows the category browser with Create and Polygon initially expanded. Recently used and starred nodes appear above categories, with right-click on a row toggling its star. Category and description hints, missing-Rust dots and the keyboard footer remain visible.

Search metadata was exported from the Python node registry through `NodeType.describe()` into `rust/crates/minihou-app/src/ui/network_menu_web.json` (170 types). The Rust shelf also contains types absent from that Python catalogue; those use their Rust label and rank after a legacy type with the same match quality. The old menu persisted favourites and recents in browser local storage. The native menu currently keeps these for the app process only; it does not persist them across launches. The legacy description, group and alias scoring is available for the exported 170 types.

`graph_edit::add` is the shared creation path for viewport transforms, model modals (Extrude, Inset, Bevel, Push/Pull), Mirror, Grow/selection nodes, viewport cuts and other shelf actions. It now places a node under its source (or in the output's slot), uses the nearest existing stream pitch of at least 96 px rounded to 32 px, falling back to 128 px, and pushes downstream cards by one pitch in the same document edit as the wiring. When no positions were authored, it materializes positions from graph depth. It splices every scalar and multi-input consumer of the source. The explicit noodle-add path still branches from its source; terminal output noodle adds insert before output. `repair_output_hangs` rewires legacy `source -> output -> operator` chains to `source -> operator -> output` as part of the next viewport add or transform gesture. Undo restores both original topology and positions in one step.

The network canvas uses its live cached positions when they differ from the document, preserving manual drags when a Tab add is committed. The generic add path uses the document's persisted stream positions.

## Checks

- Web JS `NodeMenu.search` was executed with the exported Python catalogue to capture top results for box, extrude, bevel, move, rotate, scale, loop, python, donut and image; a Rust ranking table asserts those results.
- Headless egui menu test checks Enter dispatch; keyboard tests check selection wrap and query reset. Network menu, keymap, graph edit, transform, and stream layout targeted tests passed.
- Graph tests cover output, middle and source insertion; branching, multi-display, pitch, unpositioned graphs, repair, one-step undo and several viewport node types.
- Final `cargo test --workspace --no-fail-fast` passed with `CARGO_TARGET_DIR=target_ui2`, `CARGO_PROFILE_TEST_DEBUG=0`, `CARGO_INCREMENTAL=0` and one build job: 922 passed, 3 ignored, 0 failed. A prior full attempt ran out of drive space while compiling; Cargo's clean command removed only generated `target_ui2` files before the successful retry.
- A real native window smoke test was not run in this headless job. Starred and recent choices are not yet saved between app launches.

## Try it

Open MiniHou and press Tab over the viewport or network. Type `extrude`, `bevel`, `move`, or `donut`; use Up/Down and Enter. Right-click a menu row to star it. Select an output and use a transform gizmo or start an Extrude: the new node should appear directly before the output, with the output shifted down. Undo once to restore the graph and positions.

## Shared-file edits

`main.rs`: repair legacy output hangs before choosing a transform target. `graph_edit.rs`: central stream placement, topology repair and tests. `ui/network.rs`: live canvas position reconciliation, idempotent Tab opening and adjusted UI tests. No changes to `keymap.rs`.
