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
tests skip).

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

## First numbers

`bench/bench_rust.py`, median of 5 after a warm-up; Rust includes the Python <-> Rust calls. A 4-thread cloud VM (Xeon 2.1 GHz),
Python 3.13, numpy 2.5, numba 0.68; raw data in `bench/results/rust_2026-10-14_cloud-4thread.json`.

| | quads | numpy | numba | Rust |
|---|---|---|---|---|
| edges | 1M | 312 ms | 85 ms | **63 ms** |
| point normals | 1M | 682 ms | 23 ms | 21 ms |
| fbm3 noise, 4 octaves | 1M | 910 ms | 52 ms | 53 ms |
| graph: grid > noise > transform > normal | 1M | 1755 ms | 249 ms | **121 ms** |
| edges | 4M | 2197 ms | 374 ms | 384 ms |
| point normals | 4M | 4380 ms | 166 ms | 213 ms |
| fbm3 noise, 4 octaves | 4M | 5986 ms | 202 ms | 225 ms |
| graph: grid > noise > transform > normal | 4M | 12517 ms | 914 ms | **751 ms** |

What this says, plainly:

* **Against numba, single kernels are a tie.** numba compiles the same loops to similar machine code; porting a kernel that
  numba already handles buys little on its own. (Against plain numpy, which is what the default packaged build runs because it
  leaves numba out, Rust is 5-30x faster.)
* **Whole graphs gain more** (1.2-2x over numba here): no per-node Python, fewer temporary arrays, no conversions between nodes.
  That points at the plan's Phase D (a native geometry core that keeps data on the Rust side across nodes) as where the real
  win is, more than at swapping individual kernels.
* **The packaged app** is the other easy win: shipping the Rust extension gives the default build numba-class speed without
  numba's size or its first-call compile (0.5 s for edges on first use).
* 4 threads is a small machine; Rayon's share grows with cores. These numbers need repeating on an artist's workstation, and
  end to end with Blender (`RustPlan.md` section 3) before any decision.

## Next steps

1. Run `bench/bench_rust.py` on real hardware (8-24 cores) and add the Blender-side timings from the plan.
2. Keep geometry on the Rust side between nodes: the evaluator calls the Rust cook for any run of consecutive Rust-capable nodes
   and converts only at the boundaries (this is what the graph numbers above suggest).
3. Opt-in use from the engine (`MINIHOU_RUST=1`) for `topo.edges` and the normal node, with the parity tests in CI.
4. Port by measured need: the heavy nodes (booleans, subdivide, scatter + copy to points, remesh) once profiles show them on top.
5. Package the wheel with the desktop build.
