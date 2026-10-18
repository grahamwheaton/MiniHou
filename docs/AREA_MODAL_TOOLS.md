## Purpose
Interactive viewport modeling tools, edge cutting/sliding, modal transformations, and plane picking.

## Key files
- `rust/crates/minihou-app/src/ui/mesh_cut.rs`: Loop cut, knife cutting, and edge slide operations.
- `rust/crates/minihou-app/src/ui/modal_model.rs`: Interactive extrude, inset, bevel, and pushpull state machine.
- `rust/crates/minihou-app/src/ui/modal_transform.rs`: Modal move, rotate, and scale gestures with axis constraints.
- `rust/crates/minihou-app/src/ui/mirror_tool.rs`: Interactive mirror plane placement and bounding cube selection.
- `rust/crates/minihou-core/src/nodes/pushpull.rs`: Core pushpull geometry evaluation and face offsetting.

## Key functions or types
`State`, `Kind`, `Outcome`, `Modal`, `Cube`, `insert_graph`, `show`, `start_edge_slide`, `view_plane_motion`, `hit_cube`, `ray_hit`

## Conventions to follow
- Modal states capture event focus and intercept navigation during active manipulations.
- Live previews update transient UI state; committing applies structural edits to document graph.
- Esc cancels the modal operation and restores original document state without mutations.

## Tests to run for changes here
- `cargo test -p minihou-app -- modal`
- `python -m unittest tests.test_pushpull`

## Known gotchas and open TODOs
- Loop Cut extras: numeric input, E (even), F (flip), and redo strip ([TODO_IMAGE_NODES.md](TODO_IMAGE_NODES.md#L170)).
- Inset on non-planar regions / spheres ([TODO_IMAGE_NODES.md](TODO_IMAGE_NODES.md#L174)).
- Knife tool live cooked preview and edge-axis constraints ([TODO_IMAGE_NODES.md](TODO_IMAGE_NODES.md#L178)).
