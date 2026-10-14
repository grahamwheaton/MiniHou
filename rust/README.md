# MiniHou Rust core (first steps)

The start of a native compute backend, following `RustPlan.md`: **benchmark first, port one piece at a time, keep Python as the
reference and the fallback**. Nothing in MiniHou uses it yet; it builds and tests beside the Python engine.

```
rust/
  Cargo.toml                    workspace
  crates/minihou-core/          the core library (no Python)
    src/geometry.rs             columnar geometry, the same layout as minihou/geometry.py
    src/topo.rs                 edges (numbered exactly like minihou.topo.edges)
    src/normals.rs              Newell face normals, area-weighted point normals
    src/noise.rs                Perlin / fractal noise, bit for bit the numpy float32 arithmetic
    src/nodes/                  grid, box, transform, normal, noise, merge, null, output
    src/graph.rs                reads graph JSON and cooks it (pull evaluation, each node once per cook)
    tests/core.rs               Rust-only checks
    examples/profile_edges.rs   kernel timing without Python
  crates/minihou-py/            PyO3 bindings: the Python module minihou_rs
minihou/rust.py                 the Python side: optional import, wrappers that return MiniHou types
tests/test_rust_parity.py       the same graphs cooked by both engines, compared
bench/bench_rust.py             numpy vs numba vs Rust, 1M and 4M quads
```

## Build

Needs a Rust toolchain (`rustup`, stable) and `pip install maturin`.

```
cargo test --manifest-path rust/Cargo.toml                 # the core on its own
maturin build --release -m rust/crates/minihou-py/Cargo.toml -o rust-wheels
pip install rust-wheels/minihou_rs-*.whl                   # or: maturin develop --release (in a virtualenv)
python -m unittest tests.test_rust_parity -v
python bench/bench_rust.py
```

`minihou.rust.available()` says whether the extension is importable; MiniHou imports and runs the same without it (the parity
tests skip). `tests/test_rust_parity.py` also checks the engine with Rust on against the engine with Rust off, and the CI job runs
the whole test suite with the extension installed.

## What is ported

| Node | Covered | Refused (an error naming the Rust backend, never silently ignored) |
|---|---|---|
| `grid` | everything | |
| `box` | divisions, axis divisions, triangles, consolidate, points, rotate, scale | an input to bound, `add_vertex_normals` |
| `transform` | translate, rotate, scale, uniform scale, pivot, axis rotation, N | `group`, proportional editing |
| `normal` | point, prim | vertex (cusped) |
| `noise` | scalar / vector, P or any attribute, octaves, lacunarity, gain, seed | `group` |
| `merge` | topology, attributes zero-filled, curves, detail | materials / instance lists in the detail, mixed float/int attributes |
| `null`, `output` | pass through | |

Graph JSON: `bypass` flags work; locked geometry and every other node type are refused. Each Rust node declares all of the
Python node's parameters with the same defaults, and a test compares them with `REGISTRY.get(type).coerce_params({})`.

## Parity

`tests/test_rust_parity.py` cooks the same graphs in Python and Rust, with both Python paths (numpy and numba, which the
Python engine picks between) as the oracle:

* topology, attribute names, dtypes, shapes and detail must match exactly; edges are numbered identically on both paths
  (below and above the 20k-vertex numba threshold, with open curves, one-vertex and empty primitives);
* floats within 1e-5 (positions differ by 1-2 float32 ulps after a matrix product); `fbm3` matches numpy bit for bit;
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
| `noise` node | one fused parallel pass, bit for bit the numpy result |
| `transform` node | the matrix step for P and N in one pass |
| `normal` node | face normals; point normals from a point-to-primitive table that is cached on the (frozen) topology, so a deformer upstream does not rebuild it |

Two Python-side changes came out of the profiling too (they help without Rust as well): the viewport's "does this need reducing"
check was converting the whole topology to int64 on every update; its counts are now computed once per topology and cached
(`proxy.draw_stats`).

## Numbers

`python bench/bench_rust.py --engine`: median of 5 after a warm-up, a 4-thread cloud VM (Xeon 2.1 GHz), Python 3.13, numpy 2.5,
numba 0.68. "numba" is the engine as it was (numba installed); "Rust" is the same engine with the extension. Raw data:
`bench/results/rust_2026-10-15_cloud-4thread.json` (the first round, kernels only: `rust_2026-10-14_cloud-4thread.json`).

What an artist waits for, 4M quads (1M in brackets):

| | numba | Rust | |
|---|---|---|---|
| slider edit on a noise node (re-cook noise > transform > normal) | 1051 ms (230) | **357 ms** (82) | 2.9x |
| cook grid > noise > transform > normal from scratch | 1204 ms (311) | **542 ms** (125) | 2.2x |
| viewport, a new mesh | 2130 ms (411) | **543 ms** (146) | 3.9x |
| viewport, a deformer moved the points | 290 ms (72) | **73 ms** (17) | 4.0x |

The numba column already includes this round's Python-side fix (`proxy.draw_stats`): before it, the positions-only update
took 461 ms at 4M quads with numba.

The first cook and viewport right after the engine starts, each backend in a fresh process (`python bench/bench_first_call.py`;
`bench/results/first_call_2026-10-15_cloud-4thread.json`):

| 4M quads (1M) | numpy only | numba | Rust |
|---|---|---|---|
| first cook | 19.3 s (4.0) | 1.10 s (0.32) | **0.67 s** (0.21) |
| first viewport | 8.0 s (1.7) | 2.13 s (0.42) | **0.80 s** (0.20) |

Single kernels, 4M quads:

| | numpy | numba | Rust |
|---|---|---|---|
| edges | 2032 ms | 335 ms | **214 ms** |
| point normals (topology table cached) | 4253 ms | 106 ms | **71 ms** |
| fbm3 noise, 4 octaves | 5262 ms | 193 ms | 204 ms |

What this says:

* The gain is in the **whole path**, not in single kernels: noise is compute-bound and ties numba; the wins come from fewer
  passes over memory, no int64 temporaries, keeping per-topology tables, and parallel steps numba runs serially.
* **No first-call penalty**: numba's first real call on a big mesh costs extra even after its warm-up (the viewport: 2.1 s
  first, 0.4 s at 1M); Rust is as fast the first time.
* The packaged app (no numba) gets all of this; against plain numpy the engine paths are 10-30x faster.
* 4 threads is a small machine; Rayon's share grows with cores. Repeat on an artist's workstation and end to end with Blender.

## Known gaps

* **Only built and tested on Linux so far.** CI builds the extension on Windows too, but the numbers above are from Linux.
* **Nobody gets it yet**: `minihou_rs` is not in the desktop build or the requirements; it has to be built (see Build).
* **Memory**: the cached point-to-primitive table is about 80 MB for a 4M-quad topology, held with the cooked geometry and not
  counted by the evaluator's memory budget yet.

## Next steps

1. Run `bench/bench_rust.py --engine` on real hardware (8-24 cores) and add the Blender-side timings from the plan (`bench/bench_link.py`).
2. Package the wheel with the desktop build (the biggest win for people without numba), after the Windows job is green.
3. Count the per-topology caches (`_draw_stats`, `_point_prims`) in the evaluator's memory budget.
4. The live link's encode / diff of big arrays, and the Blender side's mesh update, are the next stages to measure.
5. Port by measured need: the heavy nodes (subdivide, booleans, scatter + copy to points, remesh) once profiles show them on top.
