# MiniHou Rust core (first steps)

The start of a native compute backend, following `RustPlan.md`: **benchmark first, port one piece at a time, keep Python as the
reference and the fallback**. When the extension is installed in the Python that runs the engine, the engine uses it
automatically for its slowest steps (see "What the engine uses it for"); without it, MiniHou runs exactly as before.

```
rust/
  Cargo.toml                    workspace
  crates/minihou-core/          the core library (no Python)
    src/geometry.rs             columnar geometry, the same layout as minihou/geometry.py
    src/topo.rs                 edges (numbered exactly like minihou.topo.edges)
    src/preview.rs              the viewport's layout: fan triangles, Morton order (radix sort), run table, draw counts
    src/normals.rs              Newell face normals, area-weighted point normals
    src/noise.rs                Perlin / fractal noise, bit for bit the numpy float32 arithmetic
    src/nodes/                  grid, box, point, curve, line, circle, sphere, tube, torus, bound, delete, transform, normal, noise, match_size, merge, null, output
    src/graph.rs                reads graph JSON and cooks it (pull evaluation, each node once per cook)
    tests/core.rs               Rust-only checks
    examples/profile_*.rs       kernel timing without Python (RAYON_NUM_THREADS=1 shows what does not scale)
  crates/minihou-py/            PyO3 bindings: the Python module minihou_rs
minihou/rust.py                 the Python side: optional import, wrappers that return MiniHou types
rust-toolchain.toml             the pinned toolchain (at the repository root, so it applies wherever cargo or maturin runs)
tests/test_rust_parity.py       the same graphs cooked by both engines, compared
bench/bench_rust.py             numpy vs numba vs Rust, 1M and 4M quads
```

## Local Python parity build (debug)

The `minihou_rs` extension is built by PyO3 in `crates/minihou-py` through
maturin. Build it for the interpreter that runs the tests; an older installed
extension may import successfully while lacking current nodes or functions.
From the repository root in PowerShell:

```powershell
python -m venv --system-site-packages .venv
.\.venv\Scripts\python.exe -m pip install --no-deps maturin
$env:CARGO_BUILD_JOBS = '2'
$env:CARGO_TARGET_DIR = Join-Path (Get-Location) 'rust\target_pyparity'
$env:CARGO_PROFILE_DEV_DEBUG = '0'
$env:CARGO_INCREMENTAL = '0'
.\.venv\Scripts\python.exe -m maturin develop --manifest-path rust\crates\minihou-py\Cargo.toml
.\.venv\Scripts\python.exe -m unittest tests.test_rust_parity tests.test_parity_inputs
```

The target stays under `rust/`, and `maturin develop` installs into the local
venv. Check `minihou_rs.__file__` when diagnosing a suspected stale import.

## Build

Needs `rustup` (it fetches the pinned toolchain itself) and `pip install maturin`.

```
cargo test --manifest-path rust/Cargo.toml                 # the core on its own
maturin build --release -m rust/crates/minihou-py/Cargo.toml -o rust-wheels
pip install --force-reinstall --no-deps rust-wheels/minihou_rs-*.whl   # or: maturin develop --release (in a virtualenv)
python -m unittest tests.test_rust_parity -v
python bench/bench_rust.py
```

`cargo build` on its own makes nothing Python can load: the wheel is what the engine needs. On Windows (PowerShell, from the
repository root, with the same Python that runs the engine):

```
py -m pip install maturin
maturin build --release -m rust\crates\minihou-py\Cargo.toml -o rust-wheels
py -m pip install --force-reinstall --no-deps --no-index --find-links rust-wheels minihou-rs
py -c "from minihou import rust; print(rust.status())"     # minihou_rs 0.1.0
```

`--force-reinstall` matters after a rebuild: the version number does not change, so pip would otherwise keep the old build.
`--no-deps` stops it reinstalling numpy too (which is not in `rust-wheels`, so `--no-index` would fail).
The engine prints `compute backend: numba ...; rust: minihou_rs 0.1.0` when it starts. The engine Blender starts is whichever
Python the add-on's "External Python / Portable Runtime" preference names (or the one `tools/launch_blender.py` runs under), so
the wheel has to be installed into that one. `MINIHOU_NO_RUST=1` turns it off again without uninstalling.

`minihou.rust.available()` says whether the extension is importable; MiniHou imports and runs the same without it (the parity
tests skip). `tests/test_rust_parity.py` also checks the engine with Rust on against the engine with Rust off, and the CI job runs
the whole test suite with the extension installed.

## What is ported

| Node | Covered | Refused (an error naming the Rust backend, never silently ignored) |
|---|---|---|
| `grid` | everything | |
| `box` | divisions, axis divisions, triangles, consolidate, points, rotate, scale | an input to bound, `add_vertex_normals` |
| `point` | one point at `center` | |
| `curve` | typed points, open/closed topology, optional input append | |
| `line` | origin, direction, length, points | |
| `circle` | radius, center, rotation, divisions | |
| `sphere` | radius, center, rotate, scale, uniform scale, rows, columns | |
| `tube` | radius_bottom, radius_top, height, center, rotate, scale, uniform scale, rows, columns, caps | |
| `torus` | radius, thickness, center, rotate, scale, uniform scale, rows, columns | |
| `bound` | padding; the world-axis box and its `bound_min`/`bound_max`/`bound_size` detail | `oriented` (a principal-axis fit needs `np.linalg.svd`) |
| `delete` | expr (@P.x/y/z comparisons, empty, 0 and 1 literals), invert, point/primitive/vertex attribute compaction, curve splitting | general wrangles / attribute expressions |
| `transform` | translate, rotate, scale, uniform scale, pivot, axis rotation, N; point/primitive named groups, numeric ranges and exclusions; simple `@P.x/y/z` comparisons; proportional editing with smooth, sphere, root, sharp, linear and constant falloffs, connected and projected distances | general wrangle group expressions, random falloff |
| `normal` | point, prim | vertex (cusped) |
| `noise` | scalar / vector, P or any attribute, octaves, lacunarity, gain, seed | `group` |
| `match_size` | translate, center, justify_z, scale, size, uniform, point `N` (rescaled only for a non-uniform scale); a second input as the target bounding box | |
| `merge` | topology, attributes zero-filled, curves, detail | materials / instance lists in the detail, mixed float/int attributes |
| `heightfield_erode` | hydraulic and thermal erosion loops (kernels only; node post-processing and randomness stay in Python) | graph JSON direct cook (Python engine coordinates kernels) |
| `null`, `output` | pass through | |

Graph JSON: `bypass` flags work; locked geometry and every other node type are refused. Each Rust node declares all of the
Python node's parameters with the same defaults, and a test compares them with `REGISTRY.get(type).coerce_params({})`.

## Parity

`tests/test_rust_parity.py` cooks the same graphs in Python and Rust, with both Python paths (numpy and numba, which the
Python engine picks between) as the oracle:

* topology, attribute names, dtypes, shapes and detail must match exactly (non-float detail attributes and float detail attributes outside bounding box limits are bit-exact); edges are numbered identically on both paths
  (below and above the 20k-vertex numba threshold, with open curves, one-vertex and empty primitives);
* floats within 1e-5 (positions differ by 1-2 float32 ulps after a matrix product or fused JIT operations); `fbm3` matches numpy bit for bit;
* `bound` detail attributes (`bound_min`, `bound_max`, `bound_size`) are bit-exact against the NumPy oracle; however, when `bound` is placed downstream of a node that computes positions with JIT-fused floating point operations (such as Numba's noise kernel), the bounding box min/max coordinates naturally inherit the ~1e-7 float32 rounding differences of the upstream points under the Numba oracle;
* `serde_json` with the `float_roundtrip` feature ensures bit-exact IEEE-754 roundtripping of float numbers in JSON graph inputs and outputs;
* a seeded set of random parameters per node goes through both engines too;
* malformed topology is a `ValueError`, not a crash.

## What the engine uses it for

When `minihou_rs` is installed, the Python engine uses it automatically (`MINIHOU_NO_RUST=1` turns it off; `minihou.rust.disabled()`
does the same inside a block). The order of preference is Rust, then numba, then numpy. The results are the same within float32
rounding: topology, edge numbering and noise are identical, positions and normals can differ in the last bit, and a node that
makes a discrete choice from them can then choose differently (an oriented bounding box of a 32-gon ring comes out across its
flats instead of across its corners; both are correct):

| Where | What Rust does |
|---|---|
| `topo.edges` | the edge table (exact same numbering) |
| viewport (`preview._arrays_for`) | the whole layout of a big mesh in one call: fan triangles, Morton order, run table, edges, as the final `u32` arrays; the run table again after a deformer; the bounding box |
| `grid`, `box` nodes | built in Rust (parallel) |
| `bound` node | the world-axis box is a `box` kernel call (the min/max and the final placement stay in numpy) |
| `noise` node | one fused parallel pass, bit for bit the numpy result |
| `transform` node | the matrix step for P and N in one pass |
| `normal` node | face normals; point normals from a point-to-primitive table that is cached on the (frozen) topology, so a deformer upstream does not rebuild it |
| `heightfield_erode` node | hydraulic and thermal erosion loops in Rust (bit-identical to numpy) |
| `remesh`, `sdf_boolean` | narrow-band triangle distances, outside flood, field smoothing/combine and surface nets |
| `points_to_surface` | metaball accumulation, field smoothing and surface nets |
| `sdf_shape` | field smoothing and surface nets (analytic distance functions stay in Python) |

### SDF kernel parity

`sdf_mesh.rs`, `sdf_field.rs` and `sdf_nets.rs` port the kernels from `minihou/sdf.py`; these are Python-engine
accelerators, not new standalone Rust graph nodes. The original NumPy bodies remain available with `rust.disabled()`.
The wrappers increment the backend counter and all four volume node types are registered for the editor marker.

* Mesh fields match float32 bits in the synthetic parity cases, including a box-edge sign regression at voxel size 0.17.
  Most work is native: candidate marking, nearest-corner tree, triangle-grid traversal, distances, flood and signs.
  Sparse wall samples whose vertex seed was not improved use the existing Python `spatial.nearest` to preserve the host
  backend's nearest-corner tie order, followed by a native requery. SciPy's unspecified tie order can otherwise select
  a different triangle normal and flip a near-zero side test. The pure core convenience API uses its own deterministic
  seed choice; host-equivalent signs require `from_mesh_with_ties` plus `resolve_seed_ties`, as the binding does.
* Surface nets preserves exact float64 positions, point order and quad topology in the tests. It retains NumPy's
  offset-major accumulation order, per-round sign snapshots and batched saddle assignments, including duplicate targets.
* Points, blur, resample and polynomial smooth minimum match exactly on this Windows / NumPy 2.5.4 machine.
  Metaballs retain chunk/point accumulation order, pairwise radius reduction and scalar `pow`, which matches NumPy's
  vector power here (100,000 sampled cubic inputs). Replacing it with multiplication changes last bits.
  Mixed float32 inputs with float64 smoothing scalars retain the float32 subtraction before promotion.
  There are no exp/log calls in this version of `sdf.py`.

Coverage: `tests.test_rust_sdf_mesh` (13 methods), `tests.test_rust_sdf_field` (9), `tests.test_rust_sdf_nets` (6),
and `tests.test_rust_sdf_nodes` (8). These include boxes/spheres/tori, random fields and clouds, empty and single-voxel
inputs, large voxel sizes, all boolean modes with and without smoothing/attribute transfer, all analytic shapes,
backend markers, malformed binding arrays and the shipped volume graph's exact output arrays/topology digest.

### SDF boolean timings

`python bench/bench_sdf.py --repeat 3 --json bench/results/sdf_2026-10-15_bob.json`, Windows work desktop,
Python 3.13.16 / NumPy 2.5.4. Median of three runs after warm-up, two overlapping subdivided boxes (8 divisions),
union, smooth 0, attribute transfer off. Timings cover the full boolean node including triangulation, both fields and
meshing; source construction is excluded. NumPy has both Rust and numba disabled. Both outputs are checked for exact
topology and positions within 1e-6 absolute. Rust times include Python wrappers and sparse nearest-seed correction.

| Grid | Voxels | Voxel size | NumPy | Rust | Speed-up |
|---|---:|---:|---:|---:|---:|
| 105 × 100 × 95 | 997,500 | 0.01393617 | 4.181 s | 0.418 s | 10.0× |
| 211 × 199 × 190 | 7,977,910 | 0.00655000 | 16.926 s | 2.085 s | 8.1× |

Validation on this checkout (2026-10-15): release Cargo tests 25 passed; release Clippy with `--all-targets -- -D warnings`
clean. Python combined run: 76 tests (the one error was the since-removed stray `demo.json` node). The combined run included `test_rust_parity`, `test_rust_fuzz`,
`test_rust_session`, `test_backend_marker`, `test_nodes_degenerate_smoke`, `test_nodes_param_boundaries`, `test_examples`
and all four new SDF modules. The parity/fuzz/marker plus SDF modules also passed with `MINIHOU_NO_NUMBA=1` (62 tests),
and with `MINIHOU_NO_RUST=1` (62 tests, 59 skipped). Node parity was rerun after adding nonconstant point-color transfer
coverage: all 8 tests passed.

Two Python-side changes came out of the profiling too (they help without Rust as well): the viewport's "does this need reducing"
check was converting the whole topology to int64 on every update; its counts are now computed once per topology and cached
(`proxy.draw_stats`).

## Incremental cook (`Session`)

`Graph::cook` builds its cache per call. `minihou_core::session::Session` keeps it: edit a parameter, cook again, and only the
nodes whose recipe changed run. Each node's result is cached under a content key (type, parameters as canonical JSON, bypass
flag, the keys of what it reads), so there are no dirty flags and going back to an earlier value is a cache hit. Python:

```python
s = rust.Session(graph)             # a dict, JSON text or a minihou Graph
geo = s.cook()                      # same result as rust.cook(graph)
s.set_param('noise1', 'frequency', 2.5)
geo = s.cook()                      # only noise1 and what reads it run; s.last_stats says which
```

* The cache keeps the newest 2 results per node (`set_keep_versions`) and can take a byte budget (`set_budget_bytes`, LRU).
  Without the per-node limit a slider drag piled up about 600 MB per tick at 4M quads (6 GB after 7 edits).
* At 4M quads (laptop): an edit of the noise node runs noise, transform and normal in about 100 ms; the grid is a cache hit.
  The Python call also copies the arrays into numpy (about 50 ms here), which a native caller does not pay; returning views
  of the cached arrays is the next step for Python.
* Checked by `rust/crates/minihou-core/tests/session.rs` and `tests/test_rust_session.py` (edit sequences against a fresh cook,
  byte for byte, stats, errors, budget, threads).

## Numbers

`python bench/bench_rust.py --engine` (median after a warm-up) and `python bench/bench_first_call.py` (each backend in a fresh
process). "numba" is the engine as it was with numba installed; "Rust" is the same engine with the extension. Three machines:

* **laptop**: MSI, i7-14700HX (20 cores / 28 threads), 16 GB, Windows 11, Python 3.14, numpy 2.4, numba 0.68
  (`bench/results/rust_2026-10-15_msi-28thread.json`, `first_call_2026-10-15_msi-28thread.json`);
* **cloud**: a 4-thread VM (Xeon 2.1 GHz), Linux, Python 3.13, numpy 2.5, numba 0.68 (`rust_2026-10-15_cloud-4thread.json`,
  `first_call_2026-10-15_cloud-4thread.json`).
* **work desktop**: i9-13900KS (24 cores / 32 threads), 128 GB, Windows 11, Python 3.13.16, numpy 2.5.4, numba 0.68.0;
  9 repeats (`rust_2026-10-15_bob-32thread-4be6e0b.json`, `first_call_2026-10-15_bob-32thread-4be6e0b.json`).

The laptop was also rerun quietly with 9 repeats on Python 3.14.4, numpy 2.4.4, and numba 0.68.0
(`rust_2026-10-15_msi-28thread-e127d6e.json`, `first_call_2026-10-15_msi-28thread-e127d6e.json`).

What an artist waits for, 4M quads (1M in brackets), numba -> Rust:

| | laptop, 28 threads | | cloud, 4 threads | |
|---|---|---|---|---|
| slider edit on a noise node (re-cook noise > transform > normal) | 272 -> **98 ms** (67 -> 27) | 2.8x | 1051 -> **357 ms** | 2.9x |
| cook grid > noise > transform > normal from scratch | 620 -> **208 ms** (150 -> 52) | 3.0x | 1204 -> **542 ms** | 2.2x |
| viewport, a new mesh | 1200 -> **223 ms** (322 -> 44) | 5.4x | 2130 -> **543 ms** | 3.9x |
| viewport, a deformer moved the points | 202 -> **24 ms** (67 -> 5.9) | 8.4x | 290 -> **73 ms** | 4.0x |
| first cook after the engine starts | 1.01 -> **0.18 s** (0.24 -> 0.06) | 5.6x | 1.10 -> **0.67 s** | 1.6x |
| first viewport after the engine starts | 2.25 -> **0.30 s** (0.42 -> 0.09) | 7.5x | 2.13 -> **0.80 s** | 2.7x |

Plain numpy (the packaged app has no numba) on the laptop at 4M quads: first cook 13.6 s, first viewport 12.4 s.

Single kernels, 4M quads, numpy / numba / Rust:

| | laptop | cloud |
|---|---|---|
| edges | 2827 / 314 / **146 ms** | 2032 / 335 / **214 ms** |
| point normals (topology table cached) | 2026 / 112 / **40 ms** | 4253 / 106 / **71 ms** |
| fbm3 noise, 4 octaves | 5211 / 50 / **33 ms** | 5262 / 193 / 204 ms |

Heightfield erosion (512x512 terrain, 262k points, `python bench/bench_heightfield.py`):

| | numpy | Rust | Speed-up |
|---|---|---|---|
| thermal (20 iterations) | 925.6 ms | **15.3 ms** | 60.7x |
| hydraulic (30 iters x 4096 droplets) | 22.33 s | **2.08 s** | 10.7x |
| whole `heightfield_erode` node (30 hyd + 20 thm) | 23.72 s | **1.71 s** | 13.9x |

Same engine, three machines (4M quads, median ms, numba -> Rust, speed-up):

| | cloud 4 threads | laptop 28 threads | work desktop 32 threads |
|---|---|---|---|
| cook grid > noise > transform > normal | 1204 -> **542 ms**, 2.2x | 565 -> **216 ms**, 2.6x | 692 -> **257 ms**, 2.7x |
| slider edit on noise | 1051 -> **357 ms**, 2.9x | 295 -> **123 ms**, 2.4x | 291 -> **151 ms**, 1.9x |
| viewport new mesh | 2130 -> **543 ms**, 3.9x | 1414 -> **295 ms**, 4.8x | 1517 -> **307 ms**, 4.9x |
| viewport positions only | 290 -> **73 ms**, 4.0x | 218 -> **32 ms**, 6.7x | 238 -> **36 ms**, 6.5x |
| edges | 335 -> **214 ms**, 1.6x | 305 -> **86 ms**, 3.5x | 285 -> **90 ms**, 3.2x |
| point normals | 106 -> **71 ms**, 1.5x | 109 -> **34 ms**, 3.2x | 106 -> **39 ms**, 2.7x |
| fbm3 noise | 193 -> 204 ms, 0.9x | 37 -> **36 ms**, 1.0x | 48 -> **42 ms**, 1.1x |
| whole graph from scratch | - | 605 -> **126 ms**, 4.8x | 718 -> **177 ms**, 4.1x |
| first cook (fresh process) | 1100 -> **670 ms**, 1.6x | 569 -> **176 ms**, 3.2x | 499 -> **145 ms**, 3.4x |
| first viewport (fresh process) | 2130 -> **800 ms**, 2.7x | 1285 -> **216 ms**, 6.0x | 1083 -> **192 ms**, 5.6x |

What this adds:

* **28 and 32 threads are close** across the new runs. Their Rust times are about **1.5-6x** faster than the 4-thread VM,
  depending on the row; adding four more threads does not show a clear gain.
* Numba's start plus warm-up depends on its on-disk cache: **9.2 s** on the desktop's very first run (empty cache), then
  about **0.8-1.2 s** on repeats; on the laptop **0.9-3.3 s** across runs. Rust starts in **0.7-1.0 s** every time.
  The first cook and viewport times are listed separately above.
* **fbm3 noise is roughly tied** with numba on both Windows machines.
* At **1M quads**, the desktop cook and slider edit improve by **1.4x** and **1.1x** respectively.

### Scaling round (after the laptop numbers)

Profiling with `RAYON_NUM_THREADS=1` against 4 showed half the edge table and a good part of the viewport layout running on one
thread. Fixed, with identical results (the parity tests compare every array exactly):

* big arrays filled with a non-zero value up front (`vec![-1; n]`, 128 MB each at 4M quads) are now zero-allocated and written
  by the parallel loops; the counting sort's prefix sums and counters are parallel;
* the Morton sort is a parallel LSD radix sort (3 passes of 10 bits, stable) instead of a comparison sort of 64-bit keys;
* the topology check every Rust call makes (16M indices at 4M quads) is parallel;
* `proxy.draw_stats` (the viewport's triangle / loose point counts) is a parallel Rust pass instead of a numpy scatter, and the
  topology digest hashes the arrays in place instead of copying them first (same digest).

Same 4-thread VM, previous build against this one back to back, 4M quads, a new topology each time (the benchmark's "new mesh"
row now really uses a new topology; before it reused the digest and counts cached on the old one):

| | before | after |
|---|---|---|
| viewport, a new mesh | 867 ms | **589 ms** |
| edges | 363 ms | **253 ms** |
| cook grid > noise > transform > normal | 618 ms | **540 ms** |

The removed steps were serial, so a many-core machine gains more than these 4 threads show. What is left serial in a new-mesh
viewport is the topology digest (blake2b over the index arrays: about 135 ms here at 4M quads).

## Known gaps

* **Three machines measured** (above). The Rust extension has been built and tested on Windows with Python 3.13 and 3.14.
* **Nobody gets it yet**: `minihou_rs` is not in the desktop build or the requirements; it has to be built (see Build).
* **Memory**: the cached point-to-primitive table is about 80 MB for a 4M-quad topology, held with the cooked geometry and not
  counted by the evaluator's memory budget yet.

## Next steps

1. The laptop was rerun after the scaling round and the work desktop was measured. Test end to end with a real Blender scene;
   GPU choice in the desktop host is a separate question. Decide on the topology digest (a parallel hash would change the digest
   of big meshes, which saved measurement references and RBD bake caches use).
2. Package the wheel with the desktop build (the biggest win for people without numba: 12-14 s down to 0.2-0.3 s at 4M quads).
3. Count the per-topology caches (`_draw_stats`, `_point_prims`) in the evaluator's memory budget.
4. An end-to-end Blender test with a real mesh (engine and link together): `bench/bench_link.py` sends made-up point arrays and
   never runs the engine, so it cannot show engine changes. On the laptop a link update of 1M points costs about 5 ms.
5. Port by measured need: the heavy nodes (subdivide, booleans, scatter + copy to points, remesh) once profiles show them on top.
