# Finding 3: packed viewport baseline

Measured on Windows on 2026-10-22 in the `minihou-app` debug test binary. Run:

```powershell
cd rust
$env:CARGO_BUILD_JOBS='2'
$env:MINIHOU_VIEWPORT_TIMING='1'
cargo test -p minihou-app packed_cube_viewport_measure -- --ignored --nocapture
```

The ignored helper uses one shared 8-point, 12-triangle cube source, then repeats it 1, 1,000 and 10,000 times. The second cube is `textured_cube.minihou`, with UVs and material data. Each row is one run, not a median. Timings are wall clock milliseconds. `Scene::new` includes expansion, layout, provenance and BVH build; the BVH column is a subset of that time. Upload is `Gpu::upload` after `Gpu::new` on the headless adapter.

| Source | Copies | Cooked points | Expanded points | Expanded triangles | `display_geometry` ms | BVH ms | `Scene::new` ms | `Gpu::upload` ms | RSS after scene / upload MiB |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Plain cube | 1 | 1 | 8 | 12 | 0.141 | 0.025 | 2.280 | 0.217 | 292.7 / 297.8 |
| Plain cube | 1,000 | 1,000 | 8,000 | 12,000 | 4.746 | 28.622 | 45.208 | 50.442 | 302.6 / 296.7 |
| Plain cube | 10,000 | 10,000 | 80,000 | 120,000 | 50.343 | 360.012 | 496.162 | 593.906 | 329.2 / 374.7 |
| Textured cube | 1 | 1 | 8 | 12 | 0.062 | 0.018 | 1.184 | 0.232 | 333.2 / 334.6 |
| Textured cube | 1,000 | 1,000 | 8,000 | 12,000 | 68.648 | 29.631 | 118.618 | 47.437 | 346.6 / 358.0 |
| Textured cube | 10,000 | 10,000 | 80,000 | 120,000 | 7,115.551 | 368.748 | 7,656.729 | 485.870 | 231.3 / 306.3 |

The source has 12 triangles in both cases. RSS values are process working set samples from `sysinfo`, not peak RSS; allocations and GPU driver caching make them non-monotonic. The helper does not measure GPU allocation bytes or frame median/p95 because this headless run does not instrument wgpu allocations or render frames. Those, plus live viewport behavior, require a running app on the target GPU. The textured source shows that expansion itself can dominate even with only 12 source triangles; the BVH and upload also grow with expanded triangle count.

The stage 2 viewport budget is **50,000 expanded triangles or 50,000 expanded points**. It keeps 1,000 copies of a cube on the existing path and diverts 10,000 copies before the measured 120,000-triangle cost. This is a conservative latency guard based on the 12,000 and 120,000 triangle samples, not a frame-rate target.
