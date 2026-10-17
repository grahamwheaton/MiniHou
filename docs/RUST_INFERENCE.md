# Viewport inference engine (Phase 1)

Spec: `docs/DESIGN_INFERENCING.md`. Code: `rust/crates/minihou-app/src/inference/` (library crate, pure, no egui). First client: the sketch tools in `src/ui/sketch.rs` (Line, Polyline, Polygon, Rectangle, Circle, Measure). Move is not integrated yet.

## Architecture

| Module | Role |
| --- | --- |
| `mod.rs` | `InferenceEngine`, `InferenceInput`, `InferenceResult`, `ToolSpec` / `ToolContext`, pipeline |
| `accel.rs` | Welded edge list (soft/hidden flags), triangle BVH (ray hit) and edge BVH (cone query), cached midpoints/bounds, `GeometryKey` |
| `candidates.rs` | `Kind`, `Candidate`, `Guide`; point snaps (vertex, midpoint, on edge, true 3D intersection) and line candidates |
| `constraints.rs` | `Lock` (axis / direction / plane), working plane rule, typed values (`parse_typed`, `apply_typed`) |
| `scoring.rs` | Ranking tiers, identity keys, hysteresis |
| `references.rs` | Hover-to-reference `Tracker` (dwell, FIFO cap), `Reference`, `RefDescriptor` (live-snap ready) |
| `presentation.rs` | World-space `Primitive`s: marker shape/colour, solid/dashed lines, tooltip |
| `settings.rs` | `InferenceSettings` (persisted under `inference` in preferences.json), categories, colours, `InferenceKeys`, `LinearMode` |
| `math.rs` | Vectors, line/segment/triangle tests, `Projector` (same convention as `transform_gizmo::project`, plus exact inverse `ray`) |

## API

```rust
let mut engine = InferenceEngine::new();               // one per viewport; caches the accelerator
let res = engine.update(&InferenceInput {
    ray, cursor: [x, y], projector, time,                // seconds; drives dwell and tooltip delay
    geometry: Some(GeometryRef { positions, triangles, revision }),
    tool: ToolContext { spec: ToolSpec::LINE, start, start_face, ground, tool_points },
    settings: &settings, shift_held, typed,              // typed = parse_typed(text, units)
});
res.point; res.winner; res.alternatives; res.primitives; res.reason; res.typed_status;
res.plane; res.lock; res.face; res.descriptor;
engine.toggle_axis_lock(0); engine.lock_inferred(); engine.cycle_candidates();
engine.cycle_linear_mode(&settings); engine.escape(); engine.segment_committed();
```

Tools declare `ToolSpec { categories, dof }` (`Dof::Point | Line | Planar`); the engine never mutates geometry and never cooks the graph. In the app the engine lives in egui memory (`ui::sketch::engine(ctx)`), settings are copied there each frame from `Preferences::inference`.

Pipeline per cursor update: accelerator (rebuilt only when the `GeometryKey` changes) -> world-per-pixel model `a + b*t` sampled from the projector -> nearest surface hit (occlusion depth) -> candidates -> rank + hysteresis + cycling -> hover dwell / transient edge -> locks -> typed value -> presentation.

## Ranking (spec section 9)

Tier 0 lock; 1 endpoint, midpoint, origin; 2 edge-edge intersection, intersection of two inference lines, from-point alignment (points before lines); 3 on edge, extension; 4 axis, parallel, perpendicular; 5 on face; 6 free. Face sits under the directions because it is area-based (always "under" the cursor over a surface); otherwise axis/parallel inference would be impossible over any surface. Within a tier: screen distance, then depth. Hysteresis keeps the previous winner unless the best of the same tier is closer by `max(2 px, 15%)`. A lock projects the best exact point onto the locked line ("Locked X from Endpoint") or uses the line point closest to the cursor ray.

Tolerances: acquisition is in pixels (default 10). Geometric truth uses world tolerances: two edges intersect only when their closest points are within `1e-5 * max(1, scene diagonal)` and both parameters lie on the segments, so edges that only cross in projection never produce an intersection.

## Working plane

1. a Shift-held face plane lock; 2. the face the stroke's start point was snapped on (Line/Polyline/Polygon/Measure; directions from the start are then restricted to that plane, so drawing on an angled face does not drift); 3. the sketch chip's ground plane through the stroke origin unless seen edge-on (|cos| < 0.08); 4. the world axis plane through the start point that faces the camera most. Perpendicular means perpendicular to the edge's projection into this plane.

## Reference tracking

Hovering an endpoint/midpoint/intersection/origin or an edge for `dwell_s` (0.4 s) acquires it (max 4, FIFO). Acquired points add dashed from-point X/Y/Z lines; acquired edges add extension lines; intersections of those lines with each other and with the start point's axes/parallel/perpendicular are point candidates ("From Points"). The last hovered edge (no dwell) is the transient reference for parallel/perpendicular/extension. Esc clears typed text, then locks and references, then the stroke, then returns to Select. RMB returns to Select.

## Keys and settings

Defaults (configurable in Preferences > Viewport > Inference, documented in NATIVE_EDITOR.md's inference table, checked by a test): Right/Left/Up lock X/Y/Z, Down locks parallel/perpendicular, hold Shift locks the current inference (direction or face plane), Tab cycles overlapping candidates, an Alt tap cycles the three linear modes (also the chip button), X/Y/Z still lock axes. Conflicts: the global keymap is not polled while a sketch tool is active, so arrows never reach the timeline; Alt is a tap only, so Alt+LMB and Alt+digit numpad emulation keep working; Tab (Add menu) is suspended with the keymap. Typing: `2.5m` (length along the lock or cursor direction), `x,y,z`, `@dx,dy,dz`, `len<deg` / `<deg` (angle from the plane's first in-plane world axis). A typed value that contradicts the lock is applied and the conflict is shown in the status line.

## Static snap and live-reference design

Commits store numeric points. Each clicked point's `RefDescriptor` is passed through `UiAction::SketchCommit { refs }` and stored as `inference_refs` on the new sketch node: `{kind: vertex|edge|face|intersection|origin, node, element: [point ids], params: [weights], point_count, identity}`, where `identity` is the source node's `measurement_identity` (created on demand, as for measurement anchors). Live following (Phase 3) only needs: re-evaluate with `RefDescriptor::evaluate(positions)` when the source node's identity matches and `point_count` is unchanged, otherwise detach (keep the stored value, grey it, as measurements do). Not implemented: following, re-binding after topology change, transforms of packed/instanced geometry.

## Performance

`t14_large_mesh_update_under_budget` builds a 1,002,528-triangle bumpy grid. Unoptimised test build: accelerator build ~7.5 s once (lazy, cached per geometry key), median cursor update 1.4-2.1 ms, max ~7 ms; asserted median < 3 ms in optimised builds and < 10 ms unoptimised. Hidden/occluded geometry is pruned in the edge BVH by the surface hit depth.

## Limits / Phase 2

Unwelded meshes (one vertex per triangle corner) do not get soft-edge detection; `revision` is 0 from the app (pointer/length/sample hash detect re-cooks; in-place edits of the same buffer without a revision bump could be missed between samples). Accelerator build is synchronous on the first hover of a new geometry. Not yet: Move/Rotate/Scale clients (acceptance test 15), through-point and persistent reference lock UI, object origin and bounding-box points (bounds are cached), guides/construction geometry, face-normal and custom axes, angle increments, arc/circle centres, shape inferences, clipping/isolation filters, curve-only (non-triangle) geometry.
