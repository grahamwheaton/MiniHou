# Native opaque texture rendering: stage 4 and stage 5 controls

Implemented on `bob/tex-render` on 2026-10-17. This is the local document/image path. Blender material extraction and link asset transport remain separate stages.

## What the editor does

- A graph may have a top-level `materials` object mapping a slot name (or `detail.material_bindings` ID) to a version 1 descriptor. Alternatively `<graph-path>.materials.json` maps names to descriptors; a string value is shorthand for a local PNG/JPEG path. Relative image paths resolve next to the graph. The shipped [textured cube](../rust/crates/minihou-app/src/test_data/textured_cube.minihou) demonstrates the in-document form.
- `Scene::new` expands triangles into corner vertices using `preview::triangulate_geometry`, preserving original point and primitive IDs, named corner UV sets, per-corner normals, and face slots. Point UVs are the core fallback. Invalid or absent UVs become a finite vertex flag and show a checker. Point-indexed mesh buffers still drive picking, outline, wire, edges, points, curves and bounds.
- `Gpu::upload_textures` groups expanded triangles by material slot, builds group(1) factor/image/sampler bindings, and retains group(0) camera/MatCap. Untyped slots use white. Missing or unsupported images use the purple checker. The inspector reports the reason and missing UVs. Shaded and shaded-wire flat/smooth modes sample texture; MatCap keeps its captured colour; Random object colour selects the existing untextured path. X-ray uses the textured blend pass. Ghost geometry keeps its existing subdued pass.
- PNG/JPEG RGBA8 assets use `Rgba8UnormSrgb` for colour or `Rgba8Unorm` for linear values. The GPU decodes sRGB on sample, lighting and factors multiply in linear space, then a piecewise sRGB conversion writes the non-sRGB egui target once. CPU-generated mip levels average RGB in linear light. Samplers support nearest/linear minification and magnification, repeat/clamp/mirror, and mip settings. Bottom-up pixel rows are flipped once in `register_pixels`.
- Any surface with named material slots stays at full triangles and bypasses clustered LOD and proxy reduction. This preserves seams and face materials until a provenance-aware textured LOD exists. The inspector states this. Original indices still supply stable selection and outline masks.
- The persisted **Materials / Textures** switch is in Shading and Overlays. It defaults on when preferences omit it. It is effective in flat/smooth shaded modes; the Shading menu explains inactivity in diagnostic and MatCap modes. **Texture inspector** opens a small panel with each slot's source ID/path, dimensions, colour space, UV set, sampler, resident GPU bytes and state, plus CPU cache occupancy.

## Registry API for the transport stage

`minihou_app::materials::Registry` is device independent. Construct it with a document directory; call `register_descriptor(id, Descriptor)` for an immutable source-scoped binding ID, `register_image_with_id(expected_hash, encoded_png_or_jpeg)` for verified transport bytes, or `register_pixels(width, height, rgba8, bottom_up)` for a decoded snapshot. `register_image(bytes)` returns a canonical `sha256:` ID; `resolve(slot)` returns `MaterialBinding { descriptor, image, state }`. `register_graph(value)` and `register_sidecar(path)` are local adapters. The future link receiver should verify hashes before calling this API and bind IDs from `detail.material_bindings` without recooking geometry for an image revision. Material descriptor changes currently trigger texture upload on recook; a link receiver will need an explicit rebind notification.

The CPU decoded cache starts at 1 GiB and LRU-evicts unreferenced images. The GPU cache starts at 512 MiB, reuses image views by hash and colour interpretation, evicts unreferenced views by LRU when over budget, and downsamples newly visible images when their full mip chain exceeds the remaining budget. These are configurable registry values (`set_budgets` and `gpu_budget`). A visible pinned set can exceed a configured budget; the inspector reports downsampling on newly loaded images. Upload is synchronous on the UI thread and has no per-frame throttle yet.

## Verification and measurements

- Targeted pure tests: descriptor rejection, content hash verification, decoded cache and eviction, sRGB round-trip, bottom-up row flip, cube seam expansion (8 source points, 6 faces, 36 rendered corners), preserved NaN marker.
- Targeted headless tests: asymmetric image top/bottom orientation; two material slots on one mesh; factor multiplication; selected-object visibility; missing image and UV checker; unchanged Normal MatCap and Random object colour. Existing display and geometry render suites passed: 6 and 4 tests respectively. Tests skip with a clear message if no GPU adapter is available; this machine had one.
- Sample graph `--selftest` cooked 4 nodes, 8 points and 6 primitives: first cook 6.16 ms, second 0.64 ms in this run. The 64×64 RGBA8 sample uses 16 KiB decoded CPU pixels and about 21.3 KiB across its mip chain before driver allocation. Headless texture tests took about 0.6–0.8 s each including adapter/device setup and readback. These are test timings, not interactive frame benchmarks.
- Final workspace result: pending the one required end-of-work `cargo test --workspace --no-fail-fast` run.

## Limits and open questions

- Only opaque base colour PNG/JPEG and RGBA8 pixel assets render. High precision PNG/float input is rejected rather than silently reduced to 8-bit. Alpha MASK/BLEND, UDIM, normal/roughness/metallic/emission, Blender node evaluation, saved asset packaging, refresh controls and link transfer are future stages.
- GPU residency accounting counts mip bytes, not driver padding or staging memory. Reused pinned images can exceed a lowered budget until rebound; this needs a strict budget policy and measured frame upload scheduling. CPU payload copies made while decoding are not included in the decoded residency figure.
- The native interactive window and Blender source comparison were not exercised in this stage. Headless renders validate pixels, but not the user's actual window, file browser or Blender material workflow.
- Decide whether local graph material names should remain a convenience alias once versioned binding IDs arrive, and whether a material-only revision should trigger a lightweight rebind without any graph cook.

Shared-file edits were limited to `main.rs` (registry hookup, LOD rule, inspector), `ui/prefs.rs` (one persisted boolean), `ui/editor_state.rs` (two UI booleans/actions), and `ui/viewport_ui.rs` (two menu rows in each popover). The keymap, Blender add-on and link files were untouched.
