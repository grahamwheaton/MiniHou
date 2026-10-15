# minihou-view: native wgpu viewport spike

A time-boxed spike: can a native viewer fed straight from the Rust core beat the WebGL2 viewer
(`minihou/web/static/preview.js`) on big meshes? Numbers first, so this crate is a measuring tool, not an editor.

```
cargo run --release -p minihou-view -- --grid 2000                 # 2000 x 2000 = 4M quads (grid + noise)
cargo run --release -p minihou-view -- --graph some.json --mode solid
  --mode solid|shaded|wire|shaded-wire   (keys 1-4 at runtime)   --backend vulkan|dx12|gl|primary
  --gpu high|low   --size WxH   --bench FRAMES [--sync]   (prints one JSON line and exits)
```

Controls: LMB orbit, MMB / RMB / Shift+LMB pan, wheel zoom, `F` frame, `Esc` quit.

## What it does

* Cooks a graph with `minihou-core` (grid, box, transform, normal, noise, merge only), builds the viewport layout with
  `preview::layout` (fan triangles, Morton order, run table, edges), takes point normals from `N` or `normals::point_normals`,
  and uploads the final `f32` / `u32` arrays straight into wgpu buffers (no Python, no copy but the upload).
* Solid: flat shading from screen-space derivatives (no normal buffer read). Shaded: smooth point normals. Wire: the edge
  array as a line list. Shaded+Wire: depth-biased surface plus the lines. Lighting is a fixed three-term Workbench-like
  rig, two-sided, no PBR.
* Textures later: `shader.wgsl` has the marked place (a `uv` vertex buffer at location 2, a `@group(1)` texture + sampler).
  Not implemented.
* Not done: the run table is not used for frustum culling (the whole mesh is drawn every frame), no picking, no selection,
  no resize-time swapchain tuning, no multi-object scenes. It reads nodes `minihou-core` has, nothing more.

## Measured

Machine: i7-14700HX (28 threads), RTX 4060 Laptop 8 GB (driver 617.42) and the Intel iGPU, Windows 11. Mesh: grid of
N x N quads with the default `noise` node on top, float32, `u32` indices. 1M quads = 1000 x 1000 (2M tris), 4M = 2000 x 2000
(8M tris). Release build.

How each number was taken:

* **Native frame time**: `--bench 300..600 --sync`: the camera yaws 0.01 rad per frame, present mode Immediate, and after
  every frame the CPU waits for the GPU (`device.poll(wait)`), so the median is the real per-frame GPU+CPU time. Window
  1280x720 (the result did not change at 300x150 or 1920x1080: the cost is vertex/primitive bound).
* **WebGL2 frame time**: the real editor (`python -m minihou serve`, Python engine, unmodified `preview.js`) loaded in Edge
  (the same Chromium/ANGLE/D3D11 stack WebView2 uses), driven over CDP: each frame the page changes `view.yaw`, calls
  `redraw()`, and times a 1x1 `readPixels` right after the draw (forces the GPU to finish). Median of 150 frames. Default
  editor options (culling on, nav quality full). Canvas was small (about 290x540 px), again vertex bound.
  `raf_interval` rows in the raw output were capped by the display refresh (120 Hz) and are not used.
* **GPU memory**: `nvidia-smi` `memory.used` (system-wide, 1 s samples) peak minus the 443-477 MiB idle baseline, plus
  wgpu's own allocator report for the native side. Not a per-process number.
* **Time to first frame (native)**: from `main()` to the first presented frame, with a GPU wait. Includes cook, layout, normals,
  window, device, upload and pipeline creation. Process spawn itself (a few ms) is not included.
* **Time to mesh (web)**: from launching Edge (against an already running engine) until `preview.tris` reports the mesh,
  polled every 250 ms.

### Frame time, median ms (lower is better)

| 4M quads / 8M tris     | native RTX 4060 | WebGL2 RTX 4060 (forced) | native Intel iGPU | WebGL2 Intel iGPU (editor default) |
|------------------------|----------------:|-------------------------:|------------------:|-----------------------------------:|
| Solid (flat)           | 2.51            | n/a (no flat mode)       | 9.49              | n/a                                |
| Shaded                 | 2.50            | 2.5                      | 9.86              | 9.8                                |
| Wire (edges only)      | 1.89            | 3.9 (12.0M edges)        | 13.65             | 36.3 (12.0M edges)                 |
| Shaded + wire          | 4.00            | 5.5 (12.0M edges)        | 22.35             | 36.6 (12.0M edges)                 |

| 1M quads / 2M tris     | native RTX 4060 | WebGL2 RTX 4060 |
|------------------------|----------------:|----------------:|
| Shaded                 | 0.77            | 1.1             |
| Wire                   | 0.66            | 1.5             |
| Shaded + wire          | 1.16            | 2.2             |

Notes: the editor draws 12.0M edges for the 4M-quad mesh against 8.0M in the Rust layout (it appears to include the
triangulation diagonals), and alpha-blends them, so the wire rows are not the same workload; per edge they are about
equal. Native Solid is the cheapest surface mode; the editor has no equivalent. The 1M native numbers are from the first
(non-Intel) run set, `--sync`.

### Startup and memory (4M quads, RTX 4060)

| | native | WebGL2 editor |
|---|---:|---:|
| Cook + layout + normals | 30 + 120-230 + 55-65 ms (core, in process) | in the Python engine, not split out |
| Start to first frame / mesh visible | 0.86-1.9 s (Vulkan 0.92 s, DX12 0.86 s on repeat runs; 1.5-1.9 s on cold first runs) | 8.3 s cold engine, 1.7 s with the engine warm and the mesh cached |
| GPU memory, +MiB over idle | +577 Vulkan, +452 DX12 (of which about 258 is a fixed context cost: a 1-quad mesh costs +258) | +350 (Edge, forced to the dGPU) |
| wgpu allocator | 249 MB (P 48, N 48, tri 96, edge 64, depth, swapchain) | n/a |

The 1M cold web case showed the mesh 1.2 s after launching the browser; the 4M cold case 8.3 s. I did not split the 7 s
between engine cook, layout, transfer and upload; the Rust-core `preview::layout` itself takes 120-230 ms here.

## Verdict

* **Steady frame time at equal GPU is a tie** for the surface: 2.5 ms for 8M triangles either way, 9.8 vs 9.9 ms on the
  iGPU. WebGL2 through ANGLE/D3D11 is not the bottleneck for a plain indexed draw, and a hand-written WebGL2 viewer with
  the Morton run table and u32 arrays is already close to the metal. A native renderer will not make orbiting a 4M-quad
  mesh meaningfully faster by itself. Lines are the one visible difference: 22 vs 37 ms shaded+wire on the iGPU, but
  with 33% fewer edges drawn, so roughly even per edge.
* **The big lever is GPU choice, not API**: on this laptop the editor's WebGL2 context runs on the Intel iGPU by default
  (the in-app pane reports `Intel UHD ... D3D11`), 4x slower than the RTX 4060. A native wgpu viewer picks the
  high-performance adapter on its own (`--gpu low` to get the other); WebView2/Chromium can be pointed at the dGPU too
  (`--force_high_performance_gpu` here worked), so this is a setting to fix in the current desktop host as well.
* **What native does buy**: no transfer or parse (the buffers are the core's own arrays), a first frame under 1 s at 4M quads
  when the core cooks in process versus several seconds through the engine, a Solid mode for free, and a place for
  backends/picking/textures that doesn't depend on a browser. It costs about 250 MB of fixed VRAM for the Vulkan context
  (DX12 less) and a full UI toolkit story (this spike has none: no editor panels, no text).
* **What didn't work / caveats**: the in-app Browser pane is hidden during automation and throttles `requestAnimationFrame`
  to zero, so the web measurements were taken with a separate Edge instance over CDP instead. Memory was system-wide
  `nvidia-smi` deltas, not per process. The editor figures are one run each, the native figures are medians of 300-3000
  frames and reproduce within about 5%. The Python editor path was not profiled in pieces. The native side draws every
  triangle each frame (no culling), the editor culls by run; with the camera framing the whole mesh that makes no
  difference here, but it would matter zoomed in.
* **Recommendation**: do not move to a native renderer for orbit speed. Fix the adapter choice in the desktop host
  first (a flag), then decide on native only if the goal is dropping Chromium for startup size, memory and
  in-process data flow, and plan the editor UI as the real cost.
