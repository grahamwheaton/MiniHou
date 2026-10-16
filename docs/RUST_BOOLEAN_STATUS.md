# Rust surface Boolean, PolyReduce, Push/Pull and Boolean Fracture

Implemented in `minihou-core`, 2026-10-16. The Python implementation remains the oracle.

## Implemented behavior

- `boolean`: union, intersect and A-minus-B; symbolic edge/triangle intersection keys; shared cut segments; triangle boundary chains and nested loops; two-bridge hole reconstruction with the slit/ear-clip fallback; region classification with three-ray majority parity; welding, spike removal, pinch splitting and coincident-wall cleanup; reconstruction of intact source polygons and planar cut polygons. Bent polygons retain separate source-triangle planes.
- Attributes: edge interpolation for point floats, source endpoint for point integers, barycentric interpolation for corner floats, greatest-weight source corner for corner integers, source primitive attributes, missing-attribute zero fill, material name/index unification, USD material source dictionaries, removal of stale normals, configurable source/seam/point groups.
- `polyreduce`: area-weighted plane quadrics, stiff open-border quadrics, partial-pivot 3x3 solve with endpoint/midpoint fallback, the original edge hash and cost ranking, disjoint batched collapses, link-condition and simultaneous flip/degeneracy rejection, blocked-edge retry/widening, triangle targets/percentages, float point-attribute blending and normal renormalization, original corner/primitive attribute provenance.
- `pushpull`: convex planar face selection; bounded cutter construction; positive union and negative subtraction; pocket and through-hole behavior; the four solve-frame attempts; source validation, result validation, limits and clear errors. Existing inset/outset behavior is retained.
- `boolean_fracture`: connected cutter bodies or union mode, sequential intersections/subtractions, earlier-cutter precedence, inside/outside/both, cutter/piece attributes, six output groups, material-aware piece concatenation.

No SDF/voxel approximation or external geometry dependency is used. Boolean broad-phase candidates use an AABB sweep instead of Python's uniform grid; candidate pairs retain the same lexicographic order. Classification uses the same ray intersection tolerances and majority policy, with direct triangle queries instead of Python's acceleration grid.

## Numeric fidelity and limits

All **116 Python fixtures pass**: 73 Boolean, 13 PolyReduce, 17 Push/Pull and 13 Boolean Fracture cases (108 successful outputs and 8 expected rejections). Counts, polygon outlines, integer attributes, detail dictionaries, groups and topology checks agree. Comparison permits point/face reordering and cyclic changes of a polygon's starting corner, while preserving winding and each corner's attributes. Float comparisons use `1e-5 * (1 + abs(reference))`, or `2e-5` for PolyReduce; volumes use `2e-5 * (1 + abs(reference volume))`. There are no known geometry/count/attribute mismatches in these fixtures; binary64 intermediate values are not claimed to be bit-identical to LAPACK/NumPy on every platform.

The PolyReduce port preserves two numerically significant Python details: direct division when normalizing plane normals, and the contiguous NumPy `einsum` three-component reduction order `(x + z) + y` at the applicable call sites. Using reciprocal multiplication or ordinary left-to-right dots can change which nearly equal-cost edge wins. Norm reductions remain left-to-right. Border quadrics accumulate all edge starts, then all edge ends, before addition to the face quadrics.

Boolean uses Python's binary64 plane/intersection behavior first. Near cancellation, plane-side signs fall back to an adaptive `orient3d`: a fast determinant/error-bound check followed by error-free floating-point expansions (two-diff, two-sum and FMA two-product). Exact-sign tests include near-coplanar points and cancellation examples verified against independent i128 determinants, with permutations and translations. Intersections and positions are still floating-point constructions; output positions are float32, like Python. Python's deliberate growth/nudge of B and weld tolerance are preserved, including their small volume bias.

Two source exceptions are recorded explicitly instead of claiming an unconditional manifold guarantee:

- Cubes touching only along an edge: Python union has one edge incident to four faces. Rust matches it.
- Cubes touching only at a point: Python union has two separate vertex fans at that shared point. Its half-edge twin test passes, but a full vertex-link manifold test fails. Rust matches both results.

The other closed-solid Boolean fixtures pass the edge and vertex manifold checks, have no boundary edges, and match Python's winding, Euler characteristic and polygon-crossing checks. Open PolyReduce grids retain their expected boundaries. Push/Pull retains Python's validation/retry policy; it is not a new guarantee against every possible pathological input. For example, the supplied rotated face-5 fixture has Python's Euler characteristic 4, which the Rust output also retains.

## Validation

Run from `rust/` in PowerShell:

```powershell
$env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"
$env:CARGO_TARGET_DIR = 'target_bool'
cargo test -p minihou-core boolean_parity --lib -- --nocapture
cargo test -p minihou-core --lib
```

- Final focused run: 5 Rust test functions pass, covering all 116 fixtures plus adaptive predicate cases.
- Full core library run: **191 passed, 1 ignored**, before the last eight fixture additions; those additions pass in the final focused run. Concurrent workers subsequently added other tests; this number is a snapshot of the completed full run.
- Python reference suite: **88 passed** across `test_boolean`, `test_reduce`, `test_pushpull`, `test_materials`, `test_output_groups`, `test_fracture_suite2` and `test_rust_param_meta`.
- Parameter metadata was regenerated and its dedicated test passed again after concurrent additions to the type list.
- Final logs and temporary numerical comparison traces are under `rust/target_bool/validation/`.

Fixture coverage includes offset/flush/nested/disjoint/identical boxes, face/edge/point touching, sphere-box, coplanar sphere rings, bores, torus holes, the shipped multiple-hole/chain example, six seeded noisy-sphere/rotated-torus trials, concave caps, bent polygons, materials/UVs/integer provenance/zero fill/groups, empty inputs, dense sphere targets, flat-region reduction, open-border retention, rotated Push/Pull faces, pockets, through holes, complete removal and overlapping fracture cutters.

No workspace-wide tests, release builds or editor executable launches were used. Editor UI behavior was not tested. Existing menu and shelf entries for these node types were already present.

## Files changed for this task

- `rust/crates/minihou-core/src/boolean_kernel.rs` — new kernel and adaptive predicate implementation.
- `rust/crates/minihou-core/src/nodes/boolean.rs` — new polygon node, attributes and source polygon reconstruction.
- `rust/crates/minihou-core/src/nodes/reduce.rs` — new quadric decimator.
- `rust/crates/minihou-core/src/nodes/pushpull.rs` — completed solid Push/Pull alongside existing inset/outset.
- `rust/crates/minihou-core/src/nodes/boolean_fracture.rs` — new fracture node.
- `rust/crates/minihou-core/src/nodes/boolean_parity.rs` — Python parity and exact-sign tests.
- `rust/crates/minihou-core/src/nodes/test_data/boolean_parity.json` — generated Python oracles.
- `rust/crates/minihou-core/src/lib.rs` — kernel module declaration.
- `rust/crates/minihou-core/src/nodes/mod.rs` — targeted module/registry entries, preserving concurrent changes.
- `tools/dump_rust_boolean_fixtures.py` — reproducible pure-Python fixture generator.
- `tools/dump_rust_param_meta.py` — four node types added, preserving other workers' lists.
- `rust/crates/minihou-app/src/param_meta.json` — regenerated metadata.
- `docs/RUST_BOOLEAN_STATUS.md` — this handoff.

No commit or push was made. Unrelated working-tree changes were preserved.
