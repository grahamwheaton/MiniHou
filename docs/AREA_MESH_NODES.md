## Purpose
Core procedural mesh generation, polygon modeling nodes, and boolean CSG operations.

## Key files
- `rust/crates/minihou-core/src/nodes/mod.rs`: Node registry, dispatch tables, and input resolution.
- `rust/crates/minihou-core/src/nodes/model.rs`: Polygonal modeling nodes (extrude, bevel, inset, mirror).
- `rust/crates/minihou-core/src/nodes/polyfill.rs`: Boundary capping, quad/triangle hole filling, and patch construction.
- `rust/crates/minihou-core/src/nodes/edge_tools.rs`: Edge manipulation nodes (collapse, flip, cusp, divide).
- `rust/crates/minihou-core/src/nodes/boolean.rs`: Mesh boolean operations (union, intersection, difference).

## Key functions or types
`NodeSpec`, `registry`, `get`, `POLYFILL`, `POLYEXTRUDE`, `BOOLEAN`, `REVERSE`, `FACET`, `DIVIDE`, `CLIP`

## Conventions to follow
- Nodes take immutable input geometries and return newly allocated or modified `Geometry`.
- Declare all Python node parameters with identical defaults; mark unsupported params with `unsupported`.
- Refuse unhandled options with explicit errors naming the Rust backend.

## Tests to run for changes here
- `cargo test -p minihou-core -- polyfill`
- `python -m unittest tests.test_rust_parity`

## Known gotchas and open TODOs
- Advanced Flood Fill node (`flood_fill`) ([TODO_IMAGE_NODES.md](TODO_IMAGE_NODES.md#L57)).
- Full Array modifier parity with line/circle/curve shapes ([TODO_IMAGE_NODES.md](TODO_IMAGE_NODES.md#L85)).
- Houdini parity for Copy to Curves, Orientation along Curve, Scatter and Align ([TODO_IMAGE_NODES.md](TODO_IMAGE_NODES.md#L116)).
- Polyfill full parameter parity with Houdini ([TODO_IMAGE_NODES.md](TODO_IMAGE_NODES.md#L189)).
