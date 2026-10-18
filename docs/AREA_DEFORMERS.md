## Purpose
Geometric deformers, axial bends, twists, tapers, point relax/jitter, and lattice grid deformation.

## Key files
- `rust/crates/minihou-core/src/nodes/deform.rs`: Axial deformers (bend, twist, taper), point jitter, relax, and clean.
- `rust/crates/minihou-core/src/nodes/capture_deform.rs`: Lattice grid deformation, point deform, and soft transform.
- `rust/crates/minihou-app/src/ui/transform_gizmo.rs`: Viewport handle projection and raycasting for interactive adjustments.

## Key functions or types
`BEND`, `POINT_JITTER`, `POINT_RELAX`, `CLEAN`, `LATTICE_GRID`, `LATTICE`, `POINT_DEFORM`, `SOFT_TRANSFORM`, `bend`, `project`, `ray`

## Conventions to follow
- Deformers must preserve existing topology, face vertex counts, and non-spatial attributes.
- Use optional second input for rest geometry; deform displacements relative to rest shape.
- Recalculate or transform normal vector attribute `N` when geometry is deformed.

## Tests to run for changes here
- `cargo test -p minihou-core -- deform`
- `python -m unittest tests.test_deform`
- `python -m unittest tests.test_capture_deform`

## Known gotchas and open TODOs
- Deformer parity tasks in-flight ([TODO_IMAGE_NODES.md](TODO_IMAGE_NODES.md#L202)).
- Houdini Deform shelf parity nodes: Bulge, Fractal, Mountain, Waves, Ripple, Morph ([TODO_IMAGE_NODES.md](TODO_IMAGE_NODES.md#L191)).
