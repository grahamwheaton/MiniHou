# MiniHou handoff — Persistent Instance Overrides

**Target:** `grahamwheaton/MiniHou`, branch `native-ui`  
**Goal:** MiniHou creates individual linked Blender objects. Blender edits to location / rotation / scale survive procedural recooks without breaking instance identities.  
**Deliverable status:** This bundle contains a tested **pure-Python transform-resolution prototype**, a Blender integration adapter example, and a project-specific implementation plan. **It has not been installed or tested against Blender/MiniHou itself.** The coding agent must integrate it and run the full tests.

## Why this is a relatively contained change

Already present in native-ui:
- `instance` node: produces points with source-object choices and per-point `rot`, `scale`.
- `scene_objects` node: emits a manifest with `id`, source, location, rotation, scale.
- Blender add-on `scene_plan.py`: locates existing generated Blender objects by stable custom-property identities and plans reuse/retarget/hide/unhide.
- Blender add-on `scene_objects.py`: creates **real linked Blender duplicates**; currently rewrites local location/rotation/scale when the generated transform differs.

No new geometry instancing engine, protocol version bump, or major architecture refactor should be needed.

## User-facing behaviour

Add `transform_mode` to **Scene Objects** in **instances** mode:

| Mode | Behaviour |
| --- | --- |
| `procedural` (default) | Existing behaviour: generated transform wins on recook. |
| `preserve` | A channel manually edited in Blender is thereafter fixed to that Blender value; unedited channels continue to follow MiniHou. |
| `offset` | Hand changes are recorded as a persistent delta from the last generated placement: position offset in parent-local units, relative quaternion rotation, and multiplicative scale adjustment. As MiniHou moves/rotates/scales an instance, the user's adjustment remains. |

Provide a per-object **Reset to Procedural** control in the add-on (and optionally per-channel reset). In v1, a reset can remove the override metadata and use the last generated transform; to prevent immediate recapture on next cook, the subsequent state needs to record a procedural baseline. Add an optional **Freeze** control later if useful.

The generated ID must stay stable: use a unique integer `id` point attribute. With point-index fallback, reseeding/reordering may assign an old object's overrides to a different point. Do not silently promise stable identities when scatter counts, seeds or point order change. Consider stable-ID scatter improvements as a separate task.

## Suggested files to modify

1. `rust/crates/minihou-core/src/nodes/scene_objects.rs`: add parameter `"transform_mode": "procedural"` to `SCENE_OBJECTS` and put a validated `transform_mode` string in the scene manifest, without changing the graph schema.
2. `minihou/nodes/scene_objects.py`: mirror the parameter via `Enum("transform_mode", ["procedural","preserve","offset"], ...)` and add to the Python manifest. Keep Rust/Python parameter parity; update the Python node handler signature. `build_manifest()` is near line 241.
3. `blender_addon/minihou_link/scene_transform_overrides.py`: copy/adapt `prototype/scene_transform_overrides.py`. Pure transform math and persistence state; no bpy dependency.
4. `blender_addon/minihou_link/blender_scene_overrides_adapter.py`: copy/adapt `prototype/blender_scene_overrides_adapter.py`. Converts Blender Euler XYZ <-> quaternion, captures current local transform, resolves a target placement before planning, saves state after success.
5. `blender_addon/minihou_link/scene_objects.py`: call the adapter in `apply()` as described below. Existing `scene_plan.py` can remain unchanged if the effective placement is supplied to its planner.
6. `tests/test_scene_plan.py`, `tests/test_blender_scene_objects.py` and optionally Rust parity/integration tests: add regression coverage.
7. `docs/SCENE_OBJECTS.md`: document modes, limitations, and reset.

**Important:** Node metadata, old graph compatibility and all Python/Rust output paths must stay consistent. The existing default is `procedural`.

## Suggested integration in `scene_objects.apply()`

Use `prepare()` before the existing `sp.plan(...)`, but only for instances. The `effective` manifest is *not* sent back to MiniHou; it is used locally to determine target Blender transforms.

```python
from . import blender_scene_overrides_adapter as overrides

# In apply(), after:
by_id, extras = _existing(tag)

# NEW:
effective, pending_states = overrides.prepare(manifest, by_id)

# EXISTING planner call, but pass effective rather than manifest:
plan = sp.plan(
    effective,
    {i: _info(o) for i, o in by_id.items()},
    present,
    parent=parent_obj.name if parent_obj else "",
    extras=[_info(o) for o in extras],
)

# Keep existing creation, retarget, transforms, materials, and hide/show logic.
# After every operation has completed successfully and objs maps id -> object:
overrides.finalize(objs, pending_states)
```

Put `finalize()` **after** all transform writes, retargeting, parenting, and hiding. If the engine fails or a malformed previous state is found, do **not** overwrite generated objects and delete the user's edits; fail visibly, preserving the last good Blender state. For this reason, prepare/parse/validate the entire manifest before any object write. If you need fully atomic updates, stage the operations or provide rollback.

The adaptor's `reset_object()` is a prototype helper; expose it through a proper Blender operator that validates selected generated objects and uses appropriate undo support. Any action must preserve owner and stable ID custom properties.

## Transform invariants

- Compare **current Blender object transform** with **previous applied transform** to detect manual edits. Do not compare it only with the freshly generated placement.
- Track **previous generated transform** as a separate baseline for delta calculation.
- Persist both baselines plus overrides as custom properties on the generated Blender object (`minihou_transform_state_v1`, JSON string). This survives saving/reopening a `.blend`.
- Do not accumulate deltas on repeated recooks; a no-op recook must not move the object.
- Use parent-local Blender channels (`location`, `rotation_euler`, `scale`), not `matrix_world`. This should work with Scene Objects parented to host objects in the modifier path.
- Use quaternion differences for rotation; do not subtract Euler angles directly. Convert to Euler at the existing add-on boundary.
- Keep scale offsets multiplicative when safe, with documented handling of zero components.
- Do not mistake constraints' evaluated matrices for a manual edit; beware keyframes and drivers on transform channels, which need explicit policy (e.g. automatic Preserve mode, skip controlled channels, or warning).
- Retargeting an instance to a same-type different Blender object should keep overrides. If retargeting **rebuilds** the object, transfer and reattach stored metadata before finalise.
- Hidden/parked objects retain IDs, overrides, object modifiers, and materials; restored objects regain their previous manual adjustments.
- Duplicate objects with copied identity markers must not take over the original object's overrides; retain the existing deduplication/hide logic.
- A saved file reopened with cached generated objects must behave the same after reconnection.
- First sync after upgrading a file without old baseline metadata **cannot reliably infer** whether the current placement was manually edited. Choose safe documented behaviour, and ideally add an explicit "Adopt current as override" action for migrated scenes.

## Suggested acceptance tests

1. Existing Scene Objects tests still pass in default `procedural` mode; changed scatter positions still update as before.
2. Offset: source loc `(1,0,0)`, Blender edit `(1.5,0,0)`, next MiniHou loc `(4,0,0)` -> object `(4.5,0,0)`.
3. Preserve: edit position only; position stays fixed but rotation and scale follow subsequent MiniHou updates.
4. Offset: relative rotation and scale persist when MiniHou changes its generated rotation/scale; no Euler wrap problems.
5. No drift over repeated identical recooks or after reopening `.blend`.
6. Multiple objects: edit 3 of 100, other 97 remain procedural; changes to one do not affect others.
7. Stable IDs: changes in point ordering leave edits on matching `id`; missing ids are parked/reused. With id fallback, document the expected limitation.
8. Parent transforms: moving the owning Blender object doesn't get misinterpreted as instance offset.
9. Missing/reappearing Blender source, mesh->mesh retarget, and cross-type source retarget keep object identity/metadata when possible.
10. UI Reset to Procedural resets one or many selected instances; undo works in Blender.
11. Verify preview/live-link path **and** modifier instance path.
12. Record tested Blender version and run the repository's Blender self-tests, Python unit tests and Rust tests as feasible.

## Prototype files and tested status

- `prototype/scene_transform_overrides.py`: core per-object resolver. Has no Blender import and can be tested immediately.
- `prototype/blender_scene_overrides_adapter.py`: reference boundary adapter; needs placement in the real add-on package and a Blender integration test.
- `tests/test_scene_transform_overrides.py`: 9 independent unit tests covering modes, offsets, rotation, scale, persistence, reset, migration and validation.

Run the pure tests from the bundle root:

```bash
python -m unittest discover -s tests -v
```

## Hand-off instruction for the coding agent

> Implement `Persistent Instance Overrides` in MiniHou `native-ui`, using this bundle as a reviewed prototype, not as guaranteed final code. Integrate into the existing Scene Objects Rust/Python manifest and Blender add-on; use `scene_plan`'s stable-ID matching, and do not replace the instancing system. Support Procedural, Preserve Blender and Offset modes, with persistent per-object transform state and a Reset to Procedural UI action. Validate live-link and modifier workflows including file reload and same-source retargeting. Keep old graphs and default behaviour unchanged. Run tests, fix regressions, and produce a focused PR; do not merge until all relevant checks pass. Detail any caveats around animations, edited parents, missing sources, and point ID stability.
