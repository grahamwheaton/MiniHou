## Purpose
GPU 3D viewport rendering, camera navigation, display shading modes, and WGSL shaders.

## Key files
- `rust/crates/minihou-app/src/viewport.rs`: WGPU pipeline state, scene geometry buffers, and draw calls.
- `rust/crates/minihou-app/src/shader.wgsl`: WGSL shader for flat/smooth shading, matcaps, and overlays.
- `rust/crates/minihou-app/src/display_modes.rs`: Viewport shading modes, matcap textures, and visibility filters.
- `rust/crates/minihou-app/src/lib.rs`: Camera definition, projection matrices, framing, and ray queries.

## Key functions or types
`Scene`, `Gpu`, `Overlay`, `Target`, `Mode`, `ObjectVisibility`, `Camera`, `frame`, `matrices`, `ray`, `draw`, `draw_raw`

## Conventions to follow
- Preserve right-handed Y-up coordinate system for all view and projection transforms.
- Sanitize near and far clip planes to prevent depth buffer division by zero.
- Keep headless rendering paths functioning for CI and automated test verification.

## Tests to run for changes here
- `cargo test -p minihou-app -- viewport`
- `python -m unittest tests.test_viewport_followups`

## Known gotchas and open TODOs
- Viewport select arrow highlight in purple for active picks ([TODO_IMAGE_NODES.md](TODO_IMAGE_NODES.md#L106)).
- Blender-style viewport gizmos for Array parameters ([TODO_IMAGE_NODES.md](TODO_IMAGE_NODES.md#L97)).
- Texture transport precision limits and live smoke tests ([TODO_IMAGE_NODES.md](TODO_IMAGE_NODES.md#L176)).
