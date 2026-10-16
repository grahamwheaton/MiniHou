# Rust USD status

Implemented in `rust/crates/minihou-core/src/usd_io.rs`, `usd_io/reader.rs`, and
`nodes/usd_nodes.rs`. Runtime USD interchange no longer needs Python, `pxr`, a
C++ SDK, or a USD subprocess.

## Dependency decision

Research on 2026-10-16 found [mxpv/openusd](https://github.com/mxpv/openusd), a
pure-Rust USD implementation with USDA, USDC and USDZ readers and writers,
composition, and schema helpers. `cargo search openusd` reported 0.8.0. That
published release cannot resolve a fresh dependency graph because its ZIP 8.x
dependency versions are yanked. Both `openusd` and `openusd-schemas` are therefore
pinned to upstream commit
[`64bae2609d78565adbc8e8df8c575c7526815a00`](https://github.com/mxpv/openusd/tree/64bae2609d78565adbc8e8df8c575c7526815a00),
which uses ZIP 9. The lockfile records the revision and dependency versions.
This is an ordinary core dependency, compatible with this workspace's Rust 1.99.
It is a third-party implementation, not Pixar's complete C++ runtime.

## Supported

| Area | Behavior |
| --- | --- |
| Formats | Native read/write `.usda` text, `.usdc` binary, `.usd` (binary when written), and `.usdz` through the library. Export node and File dialog retain Python's `.usd/.usda/.usdc` extension contract; import accepts all four. |
| Meshes | Points, counts/indices, polygons, right/left handed orientation; nonuniform and mirrored transforms bake into positions/normals, with winding and corner attributes corrected. Export uses subdivision scheme `none`. |
| Curves / points | Linear, nonperiodic BasisCurves and loose Points; curves expand shared points for USD and reconstruct original point/primitive order on import. Cubic curves import as their control polygons with a note, matching Python. Degenerate faces/curves are counted as dropped; their unused points survive. |
| Transforms | Nested Xforms, matrix/translation/rotation/scale/orient ops, inverse ops and reset stacks via the schema library; native scene instance proxies are traversed. |
| Primvars | Float32/int32 scalar and tuple columns on point, corner, and primitive domains, interpolation, indexed values, constant inheritance, and `elementSize` for wide tuples. `st`/`displayColor`/`displayOpacity`/normals map to `uv`/`Cd`/`Alpha`/`N`. Original names/domain metadata disambiguate duplicate names. |
| PointInstancer | Prototype names/customData, positions, indices, Euler-to-quaternion orientationsf, scale, ids, velocities, angular velocities, invisible IDs; exact MiniHou primvars retained. Foreign prototypes produce instance names and bounds, rather than being imported as scene meshes. |
| Groups / materials | Face and point GeomSubsets plus group primvars. Material names and face slots, direct/inherited bindings and face materialBind subsets, UsdPreviewSurface diffuse color/roughness. Imported preview color supplies face Cd if geometry has no color. Material sources are recorded and referenced on re-export when the source file exists; otherwise preview surfaces are written. Multiple outputs reconcile material tables. |
| Metadata | Z or Y up, all five Python units (meters/centimeters/millimeters/inches/feet), metersPerUnit, default root, extent, MiniHou counts/dropped/detail/name/domain bookkeeping. Graph JSON in customLayerData preserves editor/extension fields as well as engine data. Details can vary by frame. |
| Animation | Real upstream cooks across start/end/step (20,000 frame limit), time samples, interpolation, topology changes, absent/late attributes, visibility for absent geometry, and prototype-target union/remapping. Static exports have ordinary default values. Timeline is restored after worker export, including failure. |
| Import settings | All Python parameter names/defaults: missing-file policy, time/offset/scale, prim include/exclude patterns (`*`, `?`, `^`, `-`), purposes, optional path attribute, axis/units, reload counter. File mtime/size participates in cache keys. |
| Integration | Registered `usd_import` and real `usd_export`; existing add-menu/shelf entries already covered both. Regenerated parameter metadata. File → Export USD dialog and HTTP `/api/export_usd` / USD node Save action use the native full-resolution worker writer. HTTP import Reload increments the normal node parameter. |

`usd_export` writes on cook and returns its input unchanged, as explicitly
requested for Rust. **Python's node only writes on its Save action**, so this is
an intentional behavior difference. Frame-range node export cooks a separate
session with the same live inputs and does not move the interactive timeline.
Nested exports are suppressed while the shared export worker gathers samples.

Relative node paths use `MINIHOU_WORKSPACE` when set; otherwise the current
MiniHou checkout root is detected from cwd/ancestors, falling back to cwd.
The HTTP endpoint retains its existing workspace path containment checks.
Explicit native File-dialog paths can be outside that workspace.

## Limits and remaining differences

- This imports flattened geometry, not a USD stage editor. Cameras, lights,
  analytic shapes, NURBS, volumes, skeletons/skinning, blend shapes, and other
  unsupported geometric schemas are not converted; known unsupported geometry
  appears in `usd_skipped`. Surface subdivision, curve evaluation/tessellation,
  and point-instance mesh realization are not performed.
- String/asset/matrix-valued primvars cannot fit Rust Geometry's numeric attribute
  tables and are skipped with notes. IDs are reduced to core int32 storage.
  Widths and arbitrary shader/network properties are not geometry attributes.
- Material extraction handles ordinary direct/inherited bindings and face
  subsets, not collection-based binding resolution or material-purpose strength
  rules. Preview color extraction reads a UsdPreviewSurface shader; it does not
  evaluate texture networks. Referencing an imported material preserves its
  external network, but assets/textures are not localized or bundled by this
  exporter, including when writing USDZ. Source material paths containing `@`
  fall back to a preview surface.
- Instancer shear/nonuniform parent scale follows Python's approximation: notes
  report that rotations ignore it. MiniHou scale primvars are converted back from
  storage units, preserving a centimeter round trip (the Python reader currently
  lets those scaled exact primvars win without that correction).
- Animation samples retain authored values per frame instead of Python's sample
  deduplication. Shading samples also retain an initial default so Blender can
  obtain a preview color. Property type changes between frames fail explicitly.
  The layer's accumulated sample data remains in memory; cooked geometries are
  consumed one frame at a time.
- The dependency supports composition, references/payloads and variants, but this
  adapter exposes no variant-selection UI. Complex production composition,
  clips, custom resolvers and unusual schema plugins have not been validated.
- Python `pxr` is unavailable in this environment. No claim is made that the full
  Python USD test suite ran, or that this port implements all OpenUSD behavior.
- The egui dialog compiles but has not been exercised in a running native editor.
  The app was neither launched nor killed. Blender validation used a separate
  factory-startup background process and did not touch an interactive scene.

## Validation

All commands use debug/test profiles and `CARGO_TARGET_DIR=rust/target_usd`.
No release build was run.

- `cargo test -p minihou-core`: 193 passed, 1 existing ignored test (including
  11 USD integration tests).
- `cargo test -p minihou-app --lib`: 66 passed, including native HTTP worker
  export with live Blender input and restoration of the timeline.
- `cargo check -p minihou-app`: passes. Existing unrelated warnings remain.
- `python tools/dump_rust_param_meta.py` was run;
  `python -m unittest tests.test_rust_param_meta` passes.
- USD tests cover all four encodings, mixed topology and numeric attributes,
  groups, material tables/references, units/Y-up, instancers, prototype changes,
  native scene instances, animated upstream cooking, late/missing attributes,
  empty frames, overwrite errors, reload, and embedded graph preservation.
- `tests/fixtures/usd_foreign.usda` is hand-authored from USD grammar.
  `usd_blender_4_5.usdc` was produced by Blender 4.5.3's USD exporter using
  `tools/check_rust_usd_blender.py`; Rust verifies its parent transform is baked.
- `cargo run -p minihou-core --example usd_smoke -- target_usd/interop` writes
  interoperability artifacts. Blender 4.5.3 background-imported the USDA, USDC,
  USD and USDZ triangle fixtures with correct vertex/face counts, UVs and bound
  preview material, and verified moving mesh coordinates at frame 2 in USDC.
  Reproduce with `blender --background --factory-startup --python-exit-code 1
  --python tools/check_rust_usd_blender.py -- rust/target_usd/interop` from repo root.
  Logs/artifacts are under the ignored `rust/target_usd/` directory.

## Files touched by this task

- `.gitignore`, `rust/Cargo.lock`, `rust/crates/minihou-core/Cargo.toml`.
- Core: `src/usd_io.rs`, `src/usd_io/reader.rs`, `src/nodes/usd_nodes.rs`,
  `src/nodes/mod.rs`, `src/lib.rs`, `src/graph.rs`, `src/session.rs`.
- Core validation: `tests/usd.rs`, `tests/fixtures/usd_foreign.usda`,
  `tests/fixtures/usd_blender_4_5.usdc`, `examples/usd_smoke.rs`.
- App: `src/usd_export.rs`, targeted hooks in `src/main.rs` and `src/ui/topbar.rs`,
  native export in `src/http_geometry.rs`, option/reload handling in
  `src/http_ui.rs`, updated `src/http_tests.rs`, regenerated `src/param_meta.json`.
- `tools/dump_rust_param_meta.py`, `tools/check_rust_usd_blender.py`, this document.
- Two incidental compile fixes required after concurrent surface changes:
  `src/nodes/surface.rs` annotates the hull index vector as `Vec<usize>`;
  `src/nodes/deform.rs` exposes `closest_point_triangle` to sibling node modules.
  These were one-line edits; other workers' implementation changes were retained.

The core/app lists above are relative to their respective crate directories.
Existing changes in shared files were preserved; the file list does not claim
ownership of those earlier or concurrent changes.
