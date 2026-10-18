## Purpose
Parameter inspection UI, numeric stepping/dragging, unit conversion, and core parameter resolution.

## Key files
- `rust/crates/minihou-app/src/ui/params.rs`: Node parameter inspector widgets and row layouts.
- `rust/crates/minihou-app/src/ui/number_step.rs`: Numeric field stepping, scrubbing, and mouse wheel handling.
- `rust/crates/minihou-core/src/params.rs`: Core parameter definitions, specs, and type coercion.

## Key functions or types
`show`, `widget`, `widget_with_units`, `ParamRowLayout`, `RampData`, `step_size`, `stepper`, `wheel`, `Params`, `ParamSpec`

## Conventions to follow
- Match parameter names and defaults to Python node specs generated in `param_meta.json`.
- Parameter edits emit `UiAction` or mutate JSON document values directly.
- Support modifier key scaling (Shift for fine, Ctrl for coarse) on numeric fields.

## Tests to run for changes here
- `cargo test -p minihou-app -- params`
- `python -m unittest tests.test_rust_param_meta`

## Known gotchas and open TODOs
- Viewport select arrow button on group and element parameter fields ([TODO_IMAGE_NODES.md](TODO_IMAGE_NODES.md#L3), [TODO_IMAGE_NODES.md](TODO_IMAGE_NODES.md#L106)).
- Dedicated Controller node for top-level driver parameters ([TODO_IMAGE_NODES.md](TODO_IMAGE_NODES.md#L44)).
