# Finding 3: packed instances in the native viewport

Status: design only. No viewport behavior changes in this pass.

## Current path and risk

`Scene::new` in `rust/crates/minihou-app/src/viewport/scene.rs` calls `display_geometry` when it sees packed instances. That creates expanded positions, normals, triangles, textured triangle records, and a triangle BVH before `Gpu::upload` copies the data to GPU buffers. `draw.rs` then draws those buffers with an instance range of `0..1`. Repeated detailed sources therefore consume CPU RAM, build time, and GPU memory in proportion to source size times instance count.

The packed representation in `minihou-core/src/nodes/packed.rs` already stores each source once in `Geometry::packed`. Each loose point identifies a source through `packed_source`, provides translation through `P`, and may provide a 3×3 `packed_xform`. Other point attributes, including `Cd`, override source attributes during unpacking. The viewport path must preserve that visible result.

## Proposed implementation sequence

1. Measure the current path first: scene construction, BVH construction, upload time, frame time, peak Rust RSS and GPU bytes for one source repeated 1, 1,000 and 10,000 times. Use both an untextured cube and a textured/material-split asset. Record exact source triangles and final expanded counts.
2. Add a bounded viewport fallback before attempting GPU instancing. Estimate expanded point/triangle counts with checked arithmetic before calling `display_geometry`; above a documented viewport budget, draw transformed `instance_bounds` proxies. Keep the full cooked geometry and Blender export semantics intact. This guard should be a separate implementation commit with tests.
3. Define a scene representation with one immutable mesh per unique packed source and a compact list of per-instance translation, 3×3 transform, color and source id. Deduplicate by source identity (`Arc` pointer) while retaining the current non-instanced scene path for ordinary geometry. Resolve nested instances by composing transforms and retaining shared leaf meshes; detect excessive nesting/cycles and use the bounded fallback.
4. Add a GPU instance buffer and shader inputs for world position, inverse-transpose normal transform, color override, and object/instance id. Draw each source/material range with `draw_indexed(..., 0..instance_count)`. Preserve source UV seams, material slots, texture binding, back-face/winding behavior under negative scale, and the existing object visibility/depth/outline passes. Budget and chunk both source and instance buffers against wgpu limits.
5. Replace the expanded triangle BVH for instances with a top-level BVH of transformed instance bounds plus reusable source BVHs. Transform a pick ray into source space, return the same world hit and point/face identity expected by object and element selection. Keep edit previews and unsupported per-instance attributes on the CPU fallback until equivalent behavior is proven.
6. Route scene upload, proxies, normals, wire overlays, bounds, selected-object highlighting, and append-selected previews through the new representation. Preserve ordinary geometry rendering and Blender transport, which may still require expansion.

## Acceptance and tests for the implementation pass

- A scene test with 1,000 copies of one cube must retain one 8-point/12-triangle source and 1,000 transforms, rather than 8,000 points/12,000 triangles. Test mixed sources, nested transforms, negative/nonuniform scale, `Cd` overrides, and ordinary geometry in the same scene.
- A headless wgpu image comparison against the current expanded path must cover untextured shading, textured UV seams/materials, wire/outline and depth. Picking tests must compare hits and selections with the expanded path. Keep the CPU fallback tests for unsupported features.
- Benchmark 1, 1,000 and 10,000 instances on target Windows and Linux GPUs. Report scene/BVH/upload latency, median and p95 frame time, peak RSS and GPU bytes. Acceptance requires memory scaling primarily with unique source geometry plus instance count, with no visible or selection regression. Set the fallback budget from those measurements rather than guessing a production threshold.

The GPU path, fallback guard, visual equivalence and hardware limits remain unverified; this document is the implementation plan only.
