# Project Cheetah: hard-coded performance limits

Source: Argon advisor review, 2026-10-20 (task `argon-limits-4f5046`). Full raw output is kept in the advisor's job record; the summary below is the plan to follow.

## Goal
Replace hard-coded limits with defaults that adapt to the machine's RAM, CPU threads and GPU limits, and expose the rest as user settings.

## Classification
- **Keep fixed (correctness):** `MAX_ELEMENTS` (i32::MAX), `MAX_NAME` (63 bytes), UV solver tolerances (1e-20). Changing these breaks the file format or numerical stability.
- **Hardware-adaptive:** cook cache budget and image asset budgets (scale with system RAM); viewport buffer chunking (scale with wgpu `max_buffer_size` and `max_storage_buffer_binding_size`); Rayon chunk sizes (scale with `rayon::current_num_threads()`).
- **User-settable (preferences/UI):** copy memory limit, loop iteration budget (currently 10,000), solver steps and substeps, viewport normal-line density (currently 60,000).

## Gaps found
- The app queries only `max_texture_dimension_2d`. It never queries `max_buffer_size` or `max_storage_buffer_binding_size`, so a large viewport upload can panic the GPU device (`viewport/uploads.rs`).
- Packed primitives are flattened to a CPU mesh before rendering; no hardware instancing or indirect draws are used. Not verified whether any path needs them.

## Limits to replace (file, current value, what breaks if raised)
- `frame cache`: default frame budget. Excess RAM retention across frame scrubber ticks.
- `image_assets.rs:8-10`: MAX_IN_FLIGHT 512 MiB, MAX_DECODED 256 MiB, MAX_PIXELS 16M. Texture transfer thread exhaustion and decoded image heap spikes.
- `python_bridge.rs:33-34`: DEFAULT_MAX_OUTPUT 2 GiB, STDERR_CAP 64 KiB. Pipe overflow and IPC stalls.
- `loop.rs:15`: Budget 10,000 iterations per cook. Runaway loops hang the app thread.
- `goals.rs:401`: solver budget steps*iters*work <= 50,000,000; iterations clamped to 100. Quadratic solve hang.
- `rbd_engine/world.rs:107`: 2,000 bodies, 200,000 collision vertices, cache_mb clamped to 1,024 MiB. O(N^2) broadphase stall.
- `uv/unwrap.rs:867`: max_iter 50,000 for ABF/LSCM. Infinite convergence on non-manifold charts.
- `viewport/scene.rs:4`: NORMAL_LINES 60,000. Viewport fill-rate drop.
- `viewport/uploads.rs:13-26`: flat `create_buffer_init` without wgpu limit checks. Immediate panic on oversize buffers.
- `normals.rs:87`, `topo.rs:8`: RUN 16,384 fixed chunk. Replace with `(len / rayon::current_num_threads()).clamp(1024, 65536)`.

## Adaptation rules
- RAM: query total memory (sys-info or equivalent). Frame budget about 50% of available RAM; copy memory limit about 25%; voxel grid cap scales with free RAM.
- GPU: read `device.limits()`; chunk or multi-draw any viewport buffer that exceeds `max_buffer_size`.
- CPU: scale Rayon chunk size with thread count.
- Defaults are used when a query fails.

## Order of work (after the file split is finished)
1. Add a `Limits` struct with hardware detection and defaults (no behaviour change).
2. Route the image, python bridge and cache budgets through it.
3. Chunk viewport uploads against the GPU limits (fixes the device-loss risk).
4. Expose the user-settable limits in preferences.
5. Replace the Rayon chunk sizes.

## Status
Step 1 implemented in `minihou-core::limits::Limits`. It records RAM and Rayon
capacity, stores supplied GPU buffer limits, and holds the existing budgets as
defaults. RAM-derived budgets have the existing defaults as floors.

Step 2 routes the cook cache version and frame budgets, image transfer budgets,
Python bridge output and stderr caps, and loop iteration budget through
`Limits::default()`. Their runtime values remain unchanged. Hardware-adaptive
runtime sizing and steps 4–5 remain planned.

Step 3 records the effective wgpu device buffer limits in `Limits` when the
viewport GPU is constructed. Oversized indexed faces and edges are split at
primitive boundaries, with local vertex IDs and matching position, normal and
colour buffers. The draw loop renders every chunk, including selection depth,
wire, ghost and outline passes. Point/curve instances and textured vertices
also use bounded uploads. A live viewport check with a mesh beyond the device
limit is still needed; unit tests cover the CPU partitioning rules.

Step 4 adds copy memory, loop iterations, solver steps and substeps, and viewport
normal-line density to the Performance preferences tab. Their defaults match
the current `Limits` values, and the values save and load with other preferences.
The copy limit uses the existing preference key and the app's preference path.
The other runtime consumers still use their existing budgets; live preferences
UI behavior remains unverified.
