## Purpose
Interactive 2D/3D sketching, geometric constraint solving, guide alignment, and smart snapping.

## Key files
- `rust/crates/minihou-app/src/ui/sketch.rs`: Sketching interaction, stroke assembly, and measurement overlays.
- `rust/crates/minihou-app/src/inference/mod.rs`: Inference engine coordinator and constraint solving.
- `rust/crates/minihou-app/src/inference/candidates.rs`: Snapping candidate generation from geometric features.
- `rust/crates/minihou-app/src/inference/constraints.rs`: Working planes, numeric typed inputs, and lock axes.

## Key functions or types
`SketchMode`, `SketchStroke`, `InferenceEngine`, `Candidate`, `Guide`, `WorkingPlane`, `parse_typed`, `apply_typed`, `grid_snap`, `axis_lock`

## Conventions to follow
- Inference queries run on spatial acceleration structures to maintain 60 FPS viewport response.
- Snap candidates are ranked by score; strongest candidate locks current position unless overridden.
- Typed numeric dimensions take precedence over mouse cursor position.

## Tests to run for changes here
- `cargo test -p minihou-app -- inference`
- `python -m unittest tests.test_sketch_tools`

## Known gotchas and open TODOs
- Rectangle and Polygon are viewport sketch tools, not procedural graph nodes ([TODO_IMAGE_NODES.md](TODO_IMAGE_NODES.md#L133)).
- Inference Phase 2 & Move-tool integration with multi-reference snapping ([TODO_IMAGE_NODES.md](TODO_IMAGE_NODES.md#L171)).
