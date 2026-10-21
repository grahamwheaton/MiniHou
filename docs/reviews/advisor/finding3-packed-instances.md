rt/scene.rs#L84)), `SurfaceTri` list, and full triangle BVH ([`Bvh::new`](file:///C:/Users/Grayw/Documents/Minihou_Rust/.claude/worktrees/native-ui-cont/rust/crates/minihou-app/src/viewport/scene.rs#L102)).
- Scattering 10,000 copies of a 10k-vertex mesh expands to 100M vertices, Either hitting [`MAX_ELEMENTS`](file:///C:/Users/Grayw/Documents/Minihou_Rust/.claude/worktrees/native-ui-cont/rust/crates/minihou-core/src/geometry.rs#L20) / OOM or stalling scene build, BVH construction, and GPU uploads.

### 3. Smallest Safe Fix

1. **Immediate safety guard:** Cap or fallback-box unpack in [`Scene::new`](file:///C:/Users/Grayw/Documents/Minihou_Rust/.claude/worktrees/native-ui-cont/rust/crates/minihou-app/src/viewport/scene.rs#L52-L56) when total expanded vertices/primitives (`sum(source.npoints)` across instances) exceed a viewport budget, substituting [`instance_bounds`](file:///C:/Users/Grayw/Documents/Minihou_Rust/.claude/worktrees/native-ui-cont/rust/crates/minihou-core/src/nodes/packed.rs#L51-L68) proxy boxes above the threshold.
2. **Proper architectural fix:** Build one [`Scene`](file:///C:/Users/Grayw/Documents/Minihou_Rust/.claude/worktrees/native-ui-cont/rust/crates/minihou-app/src/viewport.rs#L75-L86) mesh chunk per unique [`Geometry::packed`](file:///C:/Users/Grayw/Documents/Minihou_Rust/.claude/worktrees/native-ui-cont/rust/crates/minihou-core/src/geometry.rs#L185) source (flattening only nested source transforms) plus an instance buffer of `(P, packed_xform, Cd)` per source, and issue instanced `draw_indexed(..., 0..instance_count)` calls in [draw.rs](file:///C:/Users/Grayw/Documents/Minihou_Rust/.claude/worktrees/native-ui-cont/rust/crates/minihou-app/src/viewport/draw.rs#L19).

### 4. Test That Would Prove It

Add a unit test in [scene.rs](file:///C:/Users/Grayw/Documents/Minihou_Rust/.claude/worktrees/native-ui-cont/rust/crates/minihou-app/src/viewport/scene.rs#L107) creating `N = 1,000` instances of a cube via [`instance_geometry`](file:///C:/Users/Grayw/Documents/Minihou_Rust/.claude/worktrees/native-ui-cont/rust/crates/minihou-core/src/nodes/packed.rs#L72-L96) (mirroring [`thousand_instances_share_one_source`](file:///C:/Users/Grayw/Documents/Minihou_Rust/.claude/worktrees/native-ui-cont/rust/crates/minihou-core/src/nodes/packed_tests.rs#L101-L112)):
- Assert that `Scene::new(&inst)` stores only `1` unique source mesh (`8` vertex positions, `12` triangles) and `1,000` instance transforms, rather than `8,000` vertices and `12,000` triangles.
- Add a headless wgpu render test (like [`asymmetric_f_texture_uses_blender_uv_origin`](file:///C:/Users/Grayw/Documents/Minihou_Rust/.claude/worktrees/native-ui-cont/rust/crates/minihou-app/src/viewport.rs#L219-L250)) verifying instanced draw output matches unpacked draw output.

### 5. What Could Not Be Verified

- Actual runtime frame times, BVH build latency, and peak RAM/VRAM thresholds on target hardware, as no benchmarks or application runs were executed during this static verification.

