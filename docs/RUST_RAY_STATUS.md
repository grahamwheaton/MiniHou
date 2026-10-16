# Rust Ray SOP

`ray` still takes `geo` and `target`. All original Python parameter names and defaults are retained. The Python implementation is unchanged.

For the added SOP workflow, choose `method = "project_rays"`. `direction_from` accepts `normal` (computes missing point normals), `specified_vector` (uses the existing `direction`), or `attribute` (reads `direction_attribute`). The legacy `direction` and `normal` methods keep their previous behavior, including requiring an explicit point N for `normal`.

## Implemented

- Point and primitive Group selection. Primitive selection projects all referenced points once, preserving shared topology. Collision Group selects second-input primitives without renumbering hit primitive IDs.
- Nearest or farthest ray intersections, reverse rays, legacy bidirectional nearest-hit selection, and minimum-distance surface queries. Both intersection and closest-point queries use the existing median-split triangle BVH.
- Scale multiplies the intersection path, including values above one. In `project_rays`, Lift moves along the ray after scaling; Bias moves along the collision face normal. Legacy methods retain Lift along the collision normal. Max Distance applies before Scale/Lift/Bias and can be disabled for unlimited reach.
- Deterministic per-point seeded tangent-disk jitter, including an unperturbed first ray. Average, median, shortest and longest successful rays combine lengths along the base direction. Average also combines hit normals and imported attributes. Missed samples do not contribute. Minimum Distance uses one closest-point query.
- Transform Points can be disabled independently of all output attributes. `hitdist` and `hitN` remain unconditional legacy outputs. The Point Intersection toggles additionally write `dist` and `N`; missed/unselected points retain their existing N. Configurable hit point group, primitive ID and UVW output names have independent toggles. Disabling an output does not erase pre-existing source attributes/groups.
- Legacy `import_attribs` remains active independently. The new Import Attributes from Hits switch enables separate point/vertex/primitive/detail patterns. Float point/vertex attributes use barycentric interpolation at actual collision corners; vertex seams are retained. Primitive values come from the original hit primitive. Existing attributes of incompatible type/width are replaced. Source values at misses survive. P is excluded; importing N overrides Point Intersection Normal without normalizing the interpolated N. Patterns support spaces, commas, wildcards and ordered `^` exclusions.
- Enable conditions in generated metadata disable dependent parameter widgets. Show Guide Geometry draws the selected/displayed ray node's collision wireframe without adding geometry. Guide data is carried in the reserved `__ray_guide_segments` detail entry, removed by a subsequent ray cook when guides are off.

## Compatibility choices and approximations

- Entity defaults to points, Max Distance remains enabled at 1e6, and the hit group/primitive ID outputs remain enabled (`rayHit`, `hitprim`) to preserve Python defaults. Guides default off to preserve legacy detail data and avoid unnecessary overlay payloads.
- Lift in the new mode follows the requested ray-direction semantics. SideFX's currently published Ray documentation describes normal-direction Lift; legacy modes preserve that behavior. Bias follows the requested post-hit normal offset rather than a ray-origin offset.
- Collision geometry is polygonal. Spheres are polygon meshes, not analytic/NURBS surfaces. UVW is triangle-local `(u,v,0)`, including triangles produced from n-gons; it is not Houdini's n-gon parameterization. With Average, the primitive/UVW pair comes from the actual hit nearest the average length so it stays coherent.
- Jitter uses a documented uniform disk rather than Houdini's private RNG/distribution. Even-sized median sets choose the upper median. Integer attributes use the first triangle corner, and multi-hit averages truncate to integer; numeric detail values broadcast as float point attributes. String/structured details copy as detail data when at least one point hits; string point/vertex/primitive attributes are outside the geometry model.
- Project Rays tolerance accepts intersections within the given world-space distance of triangle edges and expands BVH bounds accordingly. It does not thicken surfaces or accept parallel rays. Legacy ray methods retain exact intersections for compatibility.
- With `both_ways`, each sample chooses the closer forward/backward hit before combining. Guides are an egui wireframe overlay (no depth occlusion), shown only while this ray node is selected and displayed; guides above 20000 unique collision edges are deterministically thinned.

The reference pane is `.claude/tmp/ref_houdini_ray.png`; public semantics were cross-checked against [SideFX's Ray SOP documentation](https://www.sidefx.com/docs/houdini/nodes/sop/ray.html). That live documentation is newer than Houdini 20 and is not evidence of a Houdini 20 host comparison.

## Validation

Run from `rust/`, with `%USERPROFILE%\.cargo\bin` on PATH and `CARGO_TARGET_DIR=target_ray`:

```powershell
cargo test -p minihou-core ray
```

The ray tests cover plane/sphere projection, lift/bias/scale/clipping, reverse/farthest and bidirectional hits, minimum distance, direction modes, both group domains, optional hit outputs, all import domains, seeded jitter/combiners, tolerances, degeneracy, BVH versus exhaustive queries, and metadata coverage. The existing Python volume/ray oracle still checks every original Ray parameter default and the complete expected geometry. Its default-count check permits Ray's added Rust parameters.

Python checks from the repository root:

```powershell
python tools/dump_rust_param_meta.py
python -m unittest tests.test_rust_param_meta tests.test_attrib_tools.RayTests tests.test_empty_target_nodes
```

No workspace-wide test, release build, app launch or app termination is required. Native UI rendering and Houdini-host comparisons need separate interactive validation.

Verified on 2026-10-16: the focused Cargo command passed 19 tests (18 new ray checks plus the existing volume/ray Python-oracle test); the focused Python command passed all 6 tests. The app was neither built nor launched, so the UI and guide overlay have source-level validation only.
