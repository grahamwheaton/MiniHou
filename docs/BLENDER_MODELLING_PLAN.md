# Blender modelling coverage and implementation plan

Goal: make Blender's mesh modelling operations discoverable in MiniHou, prioritising everyday work. This is a coverage audit, not a claim of complete Blender parity.

## Baseline and priorities

Updated from `main` at `ed0e6d0`. The baseline already has transform, proportional/soft transform, extrude, inset/outset, bevel, mirror, boolean, subdivide, loop cut/point-pair split, bridge, dissolve, triangulate, fill, weld, decimate, normals, deformers, UV projection/unwrap, selection groups, sketch and push/pull tools. Their implementations have their own contracts; a similar name does not establish Blender parity.

1. **Everyday geometry editing:** merge, merge by distance, split, connect, make faces, grid fill, solidify, poke and triangles-to-quads. Implement with real Blender algorithms and selection-aware typed controls.
2. **Shape and topology tools:** standalone To Sphere and Shear; native bevel, region extrude, inset, bisect, symmetrize, spin, limited dissolve and wireframe.
3. **Long-tail headless operations:** an allowlisted `blender_mesh` node exposes native mesh operator parameters as JSON. Do not execute Python supplied by a graph. Include primitives, subdivision/un-subdivide, smoothing, intersections, hole fill, cleanup, normals, edge flags and related operations.
4. **Editor integration:** dedicated Blender shelf and node-menu group, selection-aware C-pie pages, search aliases and standalone-first search ranking.
5. **Validation:** real Blender geometry tests, frozen-input and attribute tests, UV seam checks, cancellation cleanup, executable discovery, evaluator cache checks and browser insertion/cooking. Publish a reproducible operator inventory.
6. **Remaining interactive/data-model work:** Knife and Knife Project, Rip and cursor-based Poly Build, precise native edge/vertex slide, selection-loop parity, custom split normals, shape keys and sculpt data. These need explicit viewport/context or richer data transport; do not expose no-op placeholders as completed tools.

Items 1–5 are implemented in this change. A direct background `transform.edge_slide` experiment on Blender 4.3.2 crashed the disposable process, confirming that this interactive/context-dependent path cannot safely be exposed by the current headless worker. Item 6 remains open; the inventory below records the gaps. There is no claim that all native Blender modelling tools are implemented.

## Tools added

- Without Blender: `to_sphere`, `shear`.
- Typed native controls: `merge_vertices`, `merge_by_distance`, `split_geometry`, `split_edges`, `connect_vertices`, `make_faces`, `grid_fill`, `native_solidify`, `poke_faces`, `triangles_to_quads`, `limited_dissolve`, `symmetrize`, `spin`, `native_bevel`, `native_extrude`, `native_inset`, `native_bisect`, `native_wireframe`.
- Advanced: `blender_mesh`, with 70 allowlisted mesh operations. Availability was audited against Blender 4.3.2. The dispatcher does not imply every operator/options/selection combination has been tested, or that newer Blender versions retain the same RNA parameters.

## Usage and runtime

Install Blender and put it on `PATH`, or set `BLENDER_EXE` to a trusted installed Blender executable. The engine never imports `bpy`; it runs a fresh background Blender with factory startup and automatic script execution disabled, using the bundled trusted worker. It does not open `.blend` files, edit a user's scene, require a GPU or install anything in Blender. A native cook starts a process, so it is slower than a NumPy node; unchanged graph outputs use MiniHou's evaluator cache. Executable and worker/catalog identity/mtime are included in native node cache keys.

Select vertices, edges or faces in the preview and use **C → Polygons → Solid tools → Blender tools**, the **Blender** shelf or the **Blender Modelling** Add Node group. Only tools with a matching selection mapping consume the current selection. Native tools expose `result_group` (default `native_selection`) for Blender's selected result on points, faces and edges. This lets an inset feed its selected inner face into an extrude without fragile face numbers. Empty disables the result groups. The runnable `examples/blender_modelling.json` demonstrates this chain.

Node `group` fields accept existing point/primitive patterns; edge tools accept edge patterns/groups. Empty selects everything. Native operators enforce their own selection requirements and may cancel invalid selections; MiniHou reports those errors instead of returning an apparently successful result.

The advanced node takes `operation`, `domain` (`point`, `edge`, `face`), `group` and an `options` JSON object. Example: `operation=subdivide`, `domain=face`, `options={"number_cuts": 2}`. Mesh primitives can run with the optional input disconnected. Unknown operators, malformed JSON, unsupported native keyword arguments, missing executables and native cancellation are reported.

## Data contract and limits

- Point, polygon and corner numeric attributes keep their dtype and component count through temporary native BMesh scalar layers. Blender determines interpolation/default values for newly generated geometry. Discrete groups on new elements follow native custom-data semantics, not a promise of semantic group expansion.
- `uv` also initialises a real Blender UV layer. Corner UV seams can prevent triangles-to-quads when requested. Point UVs gain an outgoing corner UV attribute so native seam data can be represented. Primitive `material_index` initialises native face material IDs; material descriptions remain in detail data.
- Edge groups/crease weights use native edge layers and return through MiniHou's outgoing-corner edge representation. Mark Seam / Mark Sharp use `edge_group_seam` / `edge_group_sharp`. These are engine attributes; the live Blender link does not automatically turn every such attribute into editable Blender edge flags.
- Derived `N` attributes are invalidated after deformation/native modelling. Use the Normal node to recompute them. Custom split-normal editing is not supported. Native face shade flags are represented by `shade_smooth`, not a guarantee that every renderer honours them.
- Polygon meshes, point clouds and attribute-free loose curves/edges are accepted. Open primitives with face/corner attributes are explicitly rejected: Blender has no corresponding data domains on loose edges. Newly created loose edges receive zero face/corner rows. Nonmanifold geometry is allowed where Blender accepts it; repeated-corner and invalid polygons are rejected.
- Limits: 250,000 points, 1,000,000 corners, 90 seconds per cook. Cancellation kills and reaps the disposable worker. Operations can still be expensive within those limits. Model interactively with smaller inputs.
- Native Solidify (`native_solidify`) means Blender's **Solidify Faces** edit operation, not all Solidify modifier options. The existing standalone `solidify` shell node keeps its parameters and behaviour. Symmetrize can introduce centre-plane edges even on an already symmetric mesh.

## Reproduce the audit

Run:

```sh
blender --background --factory-startup --python-exit-code 1 --python tools/audit_blender_modelling.py -- /tmp/blender-coverage.json
python -m unittest tests.test_blender_modelling_tools tests.test_node_menu tests.test_shelf tests.test_pie_menu
```

The integration tests skip only when Blender is unavailable; standalone tests still run. The audit discovers the installed Blender's RNA operators, reports unavailable allowlisted names, and explicitly classifies remaining operators. The checked-in inventory is a Blender 4.3.2 snapshot; rerun the audit for another version.

See [operator inventory](BLENDER_MODELLING_INVENTORY.md). Blender references: [mesh editing](https://docs.blender.org/manual/en/latest/modeling/meshes/editing/index.html), [mesh operators](https://docs.blender.org/api/current/bpy.ops.mesh.html).

## Validation results

Real Blender 4.3.2 integration tests cover common native geometry operations, input immutability, numeric attributes/groups, UV seams, material IDs, selected-result chains, invalid arguments, executable discovery, cancellation cleanup and evaluator caching. Chromium verifies selection-aware native shelf insertion/cooking and undo. The wheel was built and a native poke operation was successfully executed from its unpacked contents; a full desktop executable was not built.

The full suite initially ran 1,419 tests and found the Blender worker misplaced under the engine package; that violation was fixed by moving the worker into the Blender add-on package, and the engine-no-bpy, packaging and native integration suites pass. Six other failures were reproduced on an unchanged `main` archive: damaged disk-cache entries, missing parameter exposure in `pushpull_hole.json` and `sketch_basics.json`, a modifier example requiring an unbound Blender object, an unspecified subprocess text encoding in `tests/test_sketch_tools.py`, and the default Playwright browser missing in this environment. Targeted browser workflows were run with the installed system Chromium. These baseline issues are not changed by this modelling work.
