## Purpose
Node graph canvas rendering, node card drawing, wire routing, spring physics, and graph layout.

## Key files
- `rust/crates/minihou-app/src/ui/network.rs`: Node graph canvas UI, node cards, and user interaction.
- `rust/crates/minihou-app/src/ui/network_layout.rs`: Automatic graph layout, placement, and overlap avoidance.
- `rust/crates/minihou-app/src/ui/network_physics.rs`: Spring physics simulation for node layout dynamics.
- `rust/crates/minihou-app/src/ui/network_wires.rs`: Wire routing, bezier curves, and interactive wire cutting.

## Key functions or types
`State`, `Wiggle`, `show`, `tidy`, `frame`, `focus`, `insert_added`, `position_add`, `layout`, `free_position`, `Springs`, `Cut`, `curve`, `crosses`

## Conventions to follow
- Render immediate-mode elements in canvas coordinate space transformed by zoom and pan.
- Keep layout calculations deterministic and pure; test with headless graph data.
- Never block the UI frame thread on graph evaluations.

## Tests to run for changes here
- `cargo test -p minihou-app -- network_layout`

## Known gotchas and open TODOs
- Mouse-position node insertion on Tab with empty selection ([TODO_IMAGE_NODES.md](TODO_IMAGE_NODES.md#L40)).
- Node input port hover tooltips and wide multi-input ports ([TODO_IMAGE_NODES.md](TODO_IMAGE_NODES.md#L141)).
- Subnetwork grouping via Ctrl+G ([TODO_IMAGE_NODES.md](TODO_IMAGE_NODES.md#L169)).
