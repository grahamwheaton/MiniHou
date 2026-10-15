# Port one node to Rust

## When to port

- Profile a real cook or editor action. Port a node when the measurements put it on the hot path. See `bench/bench_rust.py` and the profiling results in `rust/README.md`.
- Treat the existing Python node as the reference. Keep its numpy and numba paths available.
- Preserve what saved graphs cook to: topology, attributes, detail, and the digest of saved geometry. Check rounding where later nodes make discrete choices.

## Recipe

1. Read the Python node's `@node` declaration and function in `minihou/nodes/`. Use its input ports, parameter names, defaults, and output as the contract. Existing examples are `grid` in `minihou/nodes/grid.py`, `box_node` in `minihou/nodes/primitives.py`, `transform` in `minihou/nodes/transform.py`, `normal` in `minihou/nodes/normal.py`, and `noise` in `minihou/nodes/noise.py`.

2. Add a `NodeSpec` and cook function in `rust/crates/minihou-core/src/nodes/generate.rs` for a source or `modify.rs` for a modifier (or add a module under `nodes/` and declare it in `nodes/mod.rs`). Copy the shape of `NORMAL` and `normal` from `modify.rs`:

   ```rust
   pub const NORMAL: NodeSpec = NodeSpec {
       name: "normal",
       inputs: GEO,
       params: crate::params!("domain": "point"; "cusp_angle": 60.0;),
       cook: normal,
   };

   fn normal(inputs: &Inputs, p: &Params) -> Result<Geometry> {
       let geo = NORMAL.one(inputs, 0)?;
       let domain = p.e("domain", &["point", "prim", "vertex"])?;
       let mut out = (*geo).clone();
       match domain.as_str() {
           "point" => {
               let n = normals::point_normals(geo.p(), &geo.topology, geo.npoints);
               out.point.insert("N".into(), Arc::new(Attr::f32(3, n)));
           }
           "prim" => {
               let n = normals::normalize_to_f32(&normals::prim_normals(geo.p(), &geo.topology));
               out.prim.insert("N".into(), Arc::new(Attr::f32(3, n)));
           }
           _ => return Err(Error::new("domain 'vertex' (cusped vertex normals) is not supported by the Rust backend yet")),
       }
       Ok(out)
   }
   ```

   Replace this example with the new node's real name and behavior. Declare **every** Python parameter with the same name and default. Mark an unimplemented parameter `unsupported`, as `TRANSFORM` marks `"group": "", unsupported` in `modify.rs`. `Params::resolve` in `rust/crates/minihou-core/src/params.rs` then rejects a nondefault value with an error naming the Rust backend. For a refused mode, return an explicit `Error::new` naming the Rust backend, as `normal` does for `domain="vertex"`. Support a setting only when its full behavior matches Python. Preserve input ordering and optional or multi input flags; see `BOX` in `generate.rs` and `NodeSpec::one` in `nodes/mod.rs`.

3. Register the spec in `registry()` in `rust/crates/minihou-core/src/nodes/mod.rs`; copy the `modify::NORMAL` entry. `get()` finds it there. `Graph::cook` in `rust/crates/minihou-core/src/graph.rs` uses `nodes::get`, `Params::resolve`, and the spec's `cook`; `rust/crates/minihou-core/src/lib.rs` exports `nodes` and `graph`. These files need no new dispatch case for an ordinary node.

4. If the Python node needs a Rust kernel, add its `#[pyfunction]` and `wrap_pyfunction!` registration in `rust/crates/minihou-py/src/lib.rs`, then a converting wrapper in `minihou/rust.py`. Copy `affine` in both files for a float32 point kernel: the binding checks the `(n, 3)` input with `slice2`, runs the core kernel in `py.detach`, and returns `array(py, out, 3)`; the wrapper calls `_need().affine(_f32x3(P), ...)`. Use the actual kernel's dtype and shape. `cook`, `node_types`, and `node_params` in the PyO3 file already use the node registry, so a graph-only port does not need a separate binding for each node.

5. Add a guarded fast path to the existing function in its `minihou/nodes/*.py` file. Copy `box_node` in `minihou/nodes/primitives.py` for a source: `if rust.enabled() and geo is None and not add_vertex_normals: return rust.cook_node("box", ...)`; the following Python body handles refused settings. For a kernel modifier, copy the `fast = rust.enabled() and geo.npoints` branch in `transform` or the `domain == "vertex"` branch before the Rust call in `normal`. Keep the Python/numba fallback reachable for every Rust refusal. `noise` uses `_noise_fused` and then its existing numpy path for scalar attributes. Do not route a refused setting into `rust.cook`: direct Rust cooks must fail loudly, while the Python engine must still cook it.

6. Add the type to `NODE_TYPES` in `minihou/rust.py` **when the Python node calls a Rust kernel**. `backend_of` uses that set and the change in `calls()` to label the editor's backend chip. Route the wrapper through `_need()`, which increments the thread's call count. `Evaluator` reads it around the node cook in `minihou/evaluator.py`.

7. Extend `NODE_TYPES`, the declared defaults test, the refusal cases, the node cases, and the engine on/off cases in `tests/test_rust_parity.py`. Extend `_random_graph`, degenerate cases, and refused fallback cases in `tests/test_rust_fuzz.py`. Add a Rust, fallback, and Python marker case in `tests/test_backend_marker.py` when the Python node gets a kernel. Update the "What is ported" table and, if applicable, "What the engine uses it for" in `rust/README.md`; add an `engine_rows` or `main` benchmark row in `bench/bench_rust.py` when profiles show a hot path.

## Tests required for a port

Use `RustParityTests.same`, `paths`, and `assert_same_geometry` in `tests/test_rust_parity.py`. They compare the direct Rust cook with the Python numpy path and, if installed, numba. Require exact topology, attribute names, shapes, dtypes, and detail; allow the existing float tolerance for arithmetic. Extend `test_every_node_declares_the_python_parameters_and_defaults` (`REGISTRY.get(t).coerce_params({})` against `rust.node_params(t)`) and `test_unsupported_settings_are_refused` (error names the Rust backend). Include supported and refused cases in `tests/test_rust_fuzz.py`, including small or degenerate geometry and the Python engine fallback. Check the `rust`/`fallback`/`python` label through `test_ported_python_and_fallback` in `tests/test_backend_marker.py`.

From the repository root in PowerShell, with the extension installed:

```powershell
py -m unittest tests.test_rust_parity tests.test_rust_fuzz tests.test_backend_marker -v
$env:MINIHOU_NO_NUMBA = '1'
py -m unittest tests.test_rust_parity tests.test_rust_fuzz tests.test_backend_marker -v
Remove-Item Env:MINIHOU_NO_NUMBA
$env:MINIHOU_NO_RUST = '1'
py -m unittest tests.test_rust_parity tests.test_rust_fuzz tests.test_backend_marker -v
Remove-Item Env:MINIHOU_NO_RUST
```

`MINIHOU_NO_NUMBA=1` exercises the numpy oracle. With `MINIHOU_NO_RUST=1`, Rust-specific parity and fuzz classes skip; the remaining tests check optional-import behavior and the editor marker with Rust disabled. The normal run exercises numba as another oracle when numba is installed.

## Build and reinstall

Run from the repository root in PowerShell, using the `py` interpreter that runs MiniHou:

```powershell
cargo test --release --manifest-path rust/Cargo.toml
py -m pip install maturin
maturin build --release -m rust/crates/minihou-py/Cargo.toml -o rust-wheels
py -m pip install --force-reinstall --no-deps --no-index --find-links rust-wheels minihou-rs
py -c "from minihou import rust; print(rust.status())"
```

## Definition of done

- Parity, defaults, refusals, engine on/off, backend marker, and fuzz tests pass.
- `rust/README.md` lists the covered and refused settings. `bench/bench_rust.py` has a measured row if the node is on a hot path.
- Saved geometry keeps the same digest. Compare representative saved graphs before and after the port, including downstream operations that use it.

## Pitfalls in this repo

- `grid` uses float64 `linspace` before float32 values; `source_transform` in `generate.rs` scales and rotates in float64 before returning to float32. Match operation order, not only the formula. Matrix products can differ by one or two float32 ulps (`rust/README.md`, "Parity").
- Edge numbering is part of parity. `test_edges_match_both_python_paths_exactly` checks both sides of the 20k vertex numba threshold and open, one-vertex, and empty primitives.
- A one-ulp position difference can flip a later discrete choice. `rust/README.md` records an oriented bounding box of a 32-gon ring switching between flats and corners. Test downstream behavior and saved digests.
- Reinstall rebuilt wheels with `--force-reinstall`: `rust/README.md` says the version stays the same, so pip otherwise keeps the old build. `--no-deps` avoids looking for numpy in `rust-wheels` under `--no-index`.
- The editor chip depends on `_need()` incrementing `calls()` in `minihou/rust.py`. A direct PyO3 call or a missing `NODE_TYPES` entry leaves `backend_of` with the wrong evidence.
