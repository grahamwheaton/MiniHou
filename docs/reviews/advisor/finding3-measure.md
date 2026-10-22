# Finding 3: packed viewport baseline

Measured on Windows on 2026-10-22 in the `minihou-app` debug test binary. The plain and textured cube baseline rows were recorded at commit `4ad3e19126`. The material split baseline below was measured after stage 2 using the helper's explicit full-expansion path. At stage 2, the helper exercises the fallback for the plain and textured cubes. Run:

```powershell
cd rust
$env:CARGO_BUILD_JOBS='2'
$env:MINIHOU_VIEWPORT_TIMING='1'
cargo test -p minihou-app packed_cube_viewport_measure -- --ignored --nocapture
```

The ignored helper uses one shared 8-point, 12-triangle cube source, then repeats it 1, 1,000 and 10,000 times. The second cube is `textured_cube.minihou`, with UVs and one material. The third variant alternates two material slots across that textured cube's faces. Each row is one run, not a median. Timings are wall clock milliseconds. Scene construction includes expansion, layout, provenance and BVH build; the BVH column is a subset of that time. Upload is `Gpu::upload` after `Gpu::new` on the headless adapter.

| Source | Copies | Cooked points | Expanded points | Expanded triangles | `display_geometry` ms | BVH ms | `Scene::new` ms | `Gpu::upload` ms | RSS after scene / upload MiB |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Plain cube | 1 | 1 | 8 | 12 | 0.141 | 0.025 | 2.280 | 0.217 | 292.7 / 297.8 |
| Plain cube | 1,000 | 1,000 | 8,000 | 12,000 | 4.746 | 28.622 | 45.208 | 50.442 | 302.6 / 296.7 |
| Plain cube | 10,000 | 10,000 | 80,000 | 120,000 | 50.343 | 360.012 | 496.162 | 593.906 | 329.2 / 374.7 |
| Textured cube | 1 | 1 | 8 | 12 | 0.062 | 0.018 | 1.184 | 0.232 | 333.2 / 334.6 |
| Textured cube | 1,000 | 1,000 | 8,000 | 12,000 | 68.648 | 29.631 | 118.618 | 47.437 | 346.6 / 358.0 |
| Textured cube | 10,000 | 10,000 | 80,000 | 120,000 | 7,115.551 | 368.748 | 7,656.729 | 485.870 | 231.3 / 306.3 |
| Material split cube | 1 | 1 | 8 | 12 | 0.057 | 0.014 | 1.561 | 0.205 | 343.9 / 353.6 |
| Material split cube | 1,000 | 1,000 | 8,000 | 12,000 | 17.743 | 28.381 | 66.149 | 44.318 | 364.1 / 365.1 |
| Material split cube | 10,000 | 10,000 | 80,000 | 120,000 | 202.296 | 351.871 | 719.425 | 443.680 | 462.6 / 509.2 |

Each source has 12 triangles. RSS values are process working set samples from `sysinfo`, not peak RSS; allocations and GPU driver caching make them non-monotonic. The helper does not measure GPU allocation bytes or frame median/p95 because this headless run does not instrument wgpu allocations or render frames. Those, plus live viewport behavior, require a running app on the target GPU. The large variation in 10,000-copy textured expansion (7.1 s versus 0.2 s for the material split variant) shows why these single samples should not be treated as stable throughput. The BVH and upload also grow with expanded triangle count.

The stage 2 viewport budget is **50,000 expanded triangles or 50,000 expanded points**. It keeps 1,000 copies of a cube on the existing path and diverts 10,000 copies before the measured 120,000-triangle cost. This is a conservative latency guard based on the 12,000 and 120,000 triangle samples, not a frame-rate target.

After the fallback change, a test scene with 100 copies of a 1,000-triangle source estimated 100,000 expanded triangles. It drew 100 bounds boxes (800 points, 1,200 triangles); measured proxy construction was 0.906 ms, BVH build 1.640 ms, and `Scene::new` 6.064 ms. This is one debug test sample. A 10,000-copy cube still yields 120,000 proxy triangles because each source already has only 12 triangles, so the bounds fallback cannot reduce that case. The live Scatter Align torus needs a check for its source triangle count, visible proxy bounds, selection behavior and frame times.
