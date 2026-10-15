# Clustered LOD for the native MiniHou viewport

## Scope and references

This is a cached-mesh viewport proxy inspired by Nanite, not a Nanite renderer. The immutable result of a MiniHou cook is the asset input. The prototype is CPU-only in `rust/crates/minihou-app/src/cluster_lod.rs`; `minihou-core` continues to own geometry and cooking. The existing reduced-triangle proxy remains the renderer path until the later integration milestone.

Epic's [SIGGRAPH 2021 deep dive](https://advances.realtimerendering.com/s2021/Karis_Nanite_SIGGRAPH_Advances_2021_final.pdf) describes clusters of roughly 128 triangles, group-level simplification with retained group boundaries, a hierarchy of errors and bounds, GPU visibility determination, hardware and software rasterization paths, a visibility buffer, and demand-paged geometry. [meshoptimizer's clusterlod reference](https://github.com/zeux/meshoptimizer/blob/master/demo/clusterlod.h) shows a practical build and projected-error formula; its [simplification documentation](https://github.com/zeux/meshoptimizer#simplification) discusses border locking and the reduction stalls it can cause. The MiniHou prototype implements only the CPU cluster build and group-atomic selection.

## Data model and invariants

- `Cluster`: at most 128 indexed triangles, source point indices, a bounding sphere, accumulated object-space error, and a level. Vertex positions are shared by levels. The eventual GPU form also has a contiguous index range and a material key.
- `DagNode`: one *group replacement*: `clusters` are rendered together, or all `children` are visited. It holds a sphere covering the **more detailed** source triangles, an error no smaller than any child's, and a level. It is a tree-shaped subset of the general cluster DAG; group-atomic cuts give the same boundary property without cluster-level DAG traversal.
- The boundary edge chain of a group is identical before and after replacement. A child group cut and an adjacent group's different-level cut therefore share their pre-existing interface vertices. Every selected node is disjoint in the replacement hierarchy; no triangles from its descendants are also drawn.
- A forest of roots is valid. A hard single cluster root conflicts with preserving a long open mesh boundary: a 2000-square grid has 8000 perimeter edges, already far beyond 128 triangles. The current conservative prototype locks *all* group boundary vertices, including the outer perimeter. A future builder may relax true outer edges after explicitly separating them from inter-group interfaces, or keep a virtual root over several meshlets.

## Build from a cooked `Geometry`

1. Validate `P` and topology. Triangulate closed polygons with the viewport's existing triangulation; preserve material and corner-attribute seams as separate simplification constraints. Curves and point-only geometry bypass clustered LOD. Reject degenerate or non-manifold regions, or keep those regions at full detail. The current `ClusterLod::build` accepts an already indexed triangle mesh and has only position data; this adapter and attribute support are still needed.
2. Sort triangle centroids by Morton code; partition into clusters of at most 128 triangles. This is spatially coherent for regular terrain, though a connectivity-aware meshlet builder should replace it for arbitrary assets.
3. Group approximately four base clusters, merge their triangles, lock boundary vertices, compute plane quadrics, choose low-cost interior edge collapses to approximately half the triangles, and split the result into new clusters. Then pair group nodes (typically two clusters each), repeat, and stop at one root or at a locked-boundary stall. The prototype retains an existing endpoint on each collapse. It uses independent collapse batches and repeats up to eight passes. It does not yet enforce the full link condition, protect folded normals or UV/material seams, or guarantee a tight error bound.
4. Store each group's child links, representative clusters, source-covering sphere, and accumulated error. The prototype's accumulated sum of the largest collapsed edge per pass is monotonic and deliberately conservative as a positional displacement estimate. Production quality needs a formally validated error bound, ideally using a measured source-to-simplified distance and attribute terms.
5. Build on a worker after a successful cook. Cache the result by the `Session` node content key plus proxy format/version and seam policy. `Session` computes the key internally (`session.rs::key_of`) but does not expose it; integration needs a read-only key accessor or a key carried in `CookResponse`. Reuse an unchanged cooked asset and evict the proxy alongside its geometry under a separate byte budget. Cancel or discard stale revisions exactly as the cook worker already does.

## CPU selection and wgpu drawing

`select(view_projection, eye, viewport_height_px, near, error_px)` traverses roots. Frustum-test the node's source-covering sphere. Estimate projected error as `object_error * abs(projection[1][1]) * viewport_height / (2 * max(distance(eye,center)-radius, near))`. Draw the group's clusters if this is within the pixel budget; otherwise visit every child. This is conservative for perspective and deliberately simple for the prototype. Orthographic projection needs a separate constant world-units-to-pixels scale before renderer integration. Hysteresis and temporal stability are later work.

For wgpu 27, pack level vertices and indices into large immutable buffers, with a range for each cluster. A simple first renderer can loop over selected ranges and call `draw_indexed`; after measuring CPU draw overhead, write indirect arguments and call `draw_indexed_indirect` for each range or a supported multi-draw variant. Culling stays on CPU initially. Keep source meshes available for precise picking, edit previews, and modes that need original topology. Each cooked display node has its own transform and cache key. Frustum culling is in the prototype; an HZB occlusion pass is optional later. The old proxy's even-triangle sampling can remain as a low-memory fallback.

Portable WGSL/wgpu does not provide Nanite's full mesh-shader pipeline or portable 64-bit atomic visibility-buffer encoding. Native extensions may evolve, but v1 should use conventional hardware triangle rasterization and the existing shading path. A software rasterizer, two-path hardware/software selection, visibility buffer material resolve, GPU persistent traversal, and disk streaming are outside v1.

## Memory planning

The table is a **planning estimate**, not a measured allocation. It assumes approximately one vertex per two source triangles, 12-byte positions, 12-byte `u32` triangle indices, about **4 times** source indices over all hierarchy levels, and about 64 bytes of metadata per 128-triangle cluster across levels. Four times is grounded in this prototype's noised grids (4.01 times at 2M triangles; 3.82 times at 8M) and is far less optimistic than an ideal halving sequence, because boundary locks limit simplification. Normals, UVs, source `Geometry`, temporary build maps, and staging buffers are excluded; peak building can be several times larger.

| Source triangles | Positions | Hierarchy indices | Cluster metadata | Approx. persistent proxy |
| ---: | ---: | ---: | ---: | ---: |
| 1M | 6 MB | 48 MB | 2 MB | 56 MB |
| 4M | 24 MB | 192 MB | 8 MB | 224 MB |
| 16M | 96 MB | 768 MB | 32 MB | 896 MB |

An uncompressed vertex/normal pair doubles the vertex component. The prototype stores `Vec<Tri>` per cluster and node and therefore has additional allocation and capacity overhead; measure its real peak before offering a 16M-triangle interactive build.

## CPU prototype measurements

Release build on the development Windows machine, running the ignored `benchmark_noised_grids` test (one timed selection at each distance, so these are indicative samples rather than a statistical frame-time distribution). The grid height is a deterministic sinusoidal noise field. The current proxy comparison is its 10% triangle sampling count; it leaves gaps, whereas each clustered cut preserves group boundaries. Pixel threshold 2, viewport height 900, camera distances 2 / 200 / 2000 in grid-normalized units:

| Grid | Source triangles | CPU build | Selected triangles (near / middle / far) | Select time (near / middle / far) |
| --- | ---: | ---: | --- | --- |
| 1000² | 2,000,000 | 16.5 s | 1,995,642 / 565,182 / 74,205 | 0.335 / 0.030 / 0.0013 ms |
| 1414² | 3,998,792 | 37.2 s | 3,903,780 / 943,572 / 50,162 | 1.358 / 0.055 / 0.0009 ms |
| 2000² | 8,000,000 | 73.6 s | 7,476,581 / 1,486,998 / 78,354 | 1.670 / 0.068 / 0.0013 ms |

The 4M-triangle near selection is below 2 ms in this run. This does not include upload, drawing, scene updates, or GPU time. The 2000² root **node** still holds 613 clusters / 78,354 triangles; it is not a single root cluster. The 10% old proxy would draw 200,000 / 399,879 / 800,000 triangles on these inputs at any camera distance.

## Staged milestones and acceptance

1. **CPU prototype (this change):** cluster and group construction, monotonic errors, group-atomic projected-error selection, tests for grid and closed sphere cuts, determinism, distance response, and ignored timing report. The requested 4M-triangle selection target is under 2 ms in release; report the actual measurement. Any unachieved criterion must stay open.
2. **Robust asset builder:** `Geometry` adapter; connected clustering; seam/material/normal preservation; manifold and link-condition checks; validated positional error; deterministic parallel build if profiling warrants it. Acceptance: a 4M-quad noised grid builds without cracks, survives arbitrary mixed cuts, reports bounded peak memory and build latency, and cleanly handles malformed/non-manifold input.
3. **Viewport integration:** expose the `Session` content key, build/cache off the UI thread, pack contiguous buffers, add a default-off preference `Proxy mode: Reduced triangles | Clustered LOD`, and retain the old mode. Acceptance: no UI stall on cooking or orbiting, revision-safe cancellation, correct appearance and picking, and all current app tests passing.
4. **Performance and quality:** benchmark draw counts and upload size; add batching/indirect draws, hysteresis, orthographic error, optional HZB, and memory eviction. Acceptance: stable frame time and no visible cracks across camera motion on the 4M-quad demo; quality and timing recorded on real hardware.

## Known prototype limits

Boundary locking can stall long before a 128-triangle root. The current independent-batch quadric collapses can damage non-manifold or poorly shaped input, and the edge-length error is a selection heuristic rather than a proved Hausdorff bound. Morton sorting does not guarantee connected clusters. The benchmark grid uses procedural noise rather than the engine's cooked graph, and CPU timings do not predict wgpu draw or upload cost. These are explicit follow-up gates before a user-facing proxy mode.
