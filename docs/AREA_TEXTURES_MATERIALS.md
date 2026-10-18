## Purpose
Texture asset registry, material descriptors, image transfer service, and GPU texture uploading.

## Key files
- `rust/crates/minihou-app/src/materials.rs`: Texture registry, material descriptors, sRGB conversions, and CPU mips.
- `rust/crates/minihou-app/src/image_assets.rs`: Hash-addressed image streaming and chunked transport service.
- `docs/RUST_TEXTURE_RENDERING.md`: Architecture specification for native opaque texture rendering.

## Key functions or types
`Registry`, `Descriptor`, `ImageAsset`, `MaterialBinding`, `Sampler`, `Service`, `srgb_to_linear`, `linear_to_srgb`

## Conventions to follow
- Deduplicate texture assets across sessions using SHA-256 content hashes.
- Upload sRGB color textures as `Rgba8UnormSrgb` and linear maps as `Rgba8Unorm`.
- Flip bottom-up image rows once upon registration; render purple checker for missing textures.

## Tests to run for changes here
- `cargo test -p minihou-app -- materials`
- `cargo test -p minihou-app -- image_assets`

## Known gotchas and open TODOs
- Image-based procedural nodes: Image Density, Sample, Mask, Displace, to Curves ([TODO_IMAGE_NODES.md](TODO_IMAGE_NODES.md#L12)).
- Texture transport limits: 16-bit and float image precision ([TODO_IMAGE_NODES.md](TODO_IMAGE_NODES.md#L176)).
