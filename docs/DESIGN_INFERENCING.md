# MiniHou smart geometry inferencing (SketchUp-inspired) - specification

Source: the user's brief (ChatGPT summary of SketchUp's inferencing, 2026-10-17). Priorities: P0 essential, P1 advanced, P2 extended. Official references: SketchUp Help "Introducing Drawing Basics and Concepts" (point, linear, shape inferences, locking), "Drawing Basic Shapes", "Drawing Arcs", "Precise Modelling", Quick Reference Cards.

## 1. Objective and principle

One unified inferencing and snapping engine for the viewport, used by every applicable tool through a shared service (candidate generation, constraint resolution, scoring, presentation), never re-implemented per tool. The user indicates intent with the mouse; the engine helps reach geometrically exact results. It is more than snapping: it can remember a hovered point and use it to establish an alignment somewhere else.

## 2. Point inferences

P0: vertex, endpoint, midpoint, edge point (closest on edge), face point, intersection (true 3D only), world origin. P1: curve centre, arc midpoint, guide point, guide intersection, object origin, bounding-box points (corners, face centres, centre). P2: curve parameter point, face centroid, mesh element centre.
Behaviour: marker near the cursor, exact 3D location used on confirm, tooltip with the type, ranked by relevance and proximity, works with transformed and procedurally generated geometry, distinguishes a true mathematical point from a visual overlap in the current camera, hidden edges/internal vertices obey visibility and snapping settings.

## 3. Axis and direction inferences

P0: X/Y/Z axis detection, axis lock, axis alignment from the active start point, axis-from-point, parallel to edge, perpendicular to edge (in the working/face plane), extend edge. P1: through point, perpendicular to face, face-plane lock, custom axes, local axis, angled direction lock, angle increments (e.g. 15 deg).
Examples: Parallel: start a line, hover an existing edge to reference its direction, move away; when the new line aligns a magenta inference + 'Parallel' appears; click commits an exactly parallel line. Perpendicular: reference an edge, begin a line; when perpendicular IN THE CURRENT DRAWING PLANE a magenta indicator + 'Perpendicular' appears (perpendicularity to a line alone is ambiguous in 3D: resolve with the working plane, an inferred face plane or an explicit plane). Extension: hovering a line lets you continue it beyond either endpoint even when the new line starts elsewhere.

## 4. Reference tracking and temporary inferences (critical)

4.1 Hover-to-reference: dwell on a vertex/midpoint/valid reference -> it becomes an inference reference; move away without clicking; temporary alignment lines extend from it. 4.2 From-point tracking: align with a remote point without connecting geometry (hover a box corner, move to another face, see horizontal/vertical alignment with that corner). 4.3 Multi-reference (P1): acquire A and B; X-alignment from A and Y-alignment from B resolve to their intersection in the active plane; show it only if it is a unique valid 3D point. 4.4 Through-point (P1): start a line, hover a second point, lock the direction through it without ending the line there. 4.5 Persistent reference lock (P1): lock an acquired inference so nearby geometry cannot replace it. 4.6 Feedback: small source-point highlights, dashed extension lines, axis colours, subtle labels ('From Point'), clear temporary vs locked styles. P0: basic hover-to-reference and from-point; P1: multiple references and advanced locks.

## 5. Surface, plane and shape inferences

Surface: P0 on face, face plane, plane lock; P1 surface normal, face-edge alignment, project to plane (projected points labelled, never confused with real intersections), guide plane; P2 section plane, non-planar surface placement. Shape: P1 square, circle centre, arc tangency; P2 golden section, half/quarter/three-quarter circle, equal radius, equal length, angle matching.

## 6. Construction geometry and precision

Guides (P1): infinite guide lines, guide points, axis-aligned and angled guides, parallel offset guides, construction planes, intersections of guides with real geometry, show/hide and delete-all. Construction geometry never modifies evaluated geometry or topology.
Numeric precision (P0 for the line tool): exact length, angle, offset distance, absolute and relative coordinates, rectangle dimensions, radius/diameter, units (m, cm, mm), negative values where meaningful. Typed values work together with active inferences (lock X, type 2.5m -> exactly 2.5 m along X). Conflicts between typed values and inferred constraints must be communicated, not silently ignored.

## 7. Keyboard and mouse (SketchUp-style, configurable)

Right Arrow = lock X (red), Left Arrow = lock Y (green), Up Arrow = lock Z (blue), Down Arrow = lock the current inferred parallel/perpendicular direction, hold Shift = lock the current inferred direction/plane (release to unlock), Alt = cycle linear inference modes, Esc = clear temporary lock/reference or cancel the operation depending on state, typing = numeric input, hover = acquire reference, click = confirm. Three line modes: all inferences; linear inferences off with point/surface snapping on; parallel and perpendicular only. Individual snap categories can be enabled/disabled in viewport settings. Resolve shortcut conflicts with existing MiniHou commands centrally (check NATIVE_EDITOR.md; the keymap table test must stay green).

## 8. Visual language

X red line, Y green, Z blue, Parallel/Perpendicular magenta, Endpoint green circle, Midpoint cyan circle, Intersection cross, Face/Edge small diamond or highlighted geometry, Acquired reference highlighted source point, Locked direction solid guide, Temporary tracking dashed guide. Colour must never be the only cue: shape, tooltip and line style also convey meaning. Show only the most relevant inference by default, highlight the geometry causing a parallel/perpendicular, tooltips 'Endpoint', 'On Face', 'Parallel', 'From Point' after a short delay, avoid clutter, keep legible across zoom levels, configurable marker size/colours/tooltips, immediate response to cursor movement without graph re-evaluation.

## 9. Candidate selection and stability

Ranking: 1 explicit user lock/constraint; 2 valid exact point under the cursor; 3 intersections and acquired-reference alignments; 4 edge and face snapping; 5 automatic axis/parallel/perpendicular; 6 unconstrained cursor. Refine by tool context, screen-space distance, reference strength, validity. Screen-space tolerance (default 8-12 px, configurable); hysteresis against flicker between near-identical candidates; exact point snaps are not displaced by weaker directional inferences; locked directions stay locked over other geometry; cycle overlapping candidates when ambiguous; never infer intersections between 3D edges that overlap only in projection; robust to degenerate edges, coincident vertices, tiny geometry; hidden geometry excluded unless enabled; respect clipping planes, isolation and visibility filters.

## 10. MiniHou procedural integration

Shared service with a common interface (tools request candidates and receive results). Operates on the currently visible evaluated geometry. Convert between world, object-local, node/tool-local, custom working axes and screen space. Hover/cursor movement never mutates geometry; committing updates a tool input, node parameter or editing operation. Static snap (default): store the resolved position as a value. Live snap (future): persistent parametric reference that follows upstream geometry (needs stable element ids, explicit handling of topology change): DESIGN THE REFERENCE-TRACKING INFRASTRUCTURE SO LIVE REFERENCES CAN BE ADDED LATER without redesign (the measurement anchors with measurement_identity already use stable point identities: reuse that idea). Performance: spatial acceleration (BVH or equivalent) for surface and edge hit testing, cached derived candidates (midpoints, centres, bounding-box points), cache invalidation only on relevant geometry change, prioritise visible and nearby geometry, handle large meshes without scanning every element per mouse move, keep visual updates independent of expensive graph evaluation.

## 11. Tool integration (eventual)

Line/Polyline (endpoint, axes, parallel, perpendicular, tracking), Rectangle (face plane, corners, square, edge alignment), Circle/Arc (centres, radii, tangencies), Move (point-to-point alignment, axis and plane constraints), Rotate (pivot snapping, axis orientation, angle alignment), Scale (reference points, edge lengths), Extrude (face-normal direction, target faces, distances), Draw on Surface, construction tools, node viewport handles. Each tool declares the inference categories it supports and its degrees of freedom.

## 12. Phases

Phase 1 (P0): vertex/endpoint/midpoint, edge/face/intersection, world origin, X/Y/Z alignment and lock, parallel/perpendicular, edge extension, basic reference tracking (hover-to-reference + from-point), on-face / face-plane, visual markers, tooltips and lines, integration with the LINE tool first (then Move). Phase 2 (P1): multi-reference, through-point, object origin/bbox, face normal/custom axes, guides, arc/circle centre, shape inference, settings, broader tools. Phase 3 (P2): more shape relations, tangency, section geometry, surface-specific snapping, persistent parametric references, advanced filters.

## 13. Acceptance tests

1 axis snap while moving roughly along X; 2 axis lock (lock X, diagonal cursor, committed line exactly X); 3 midpoint of an arbitrary edge; 4 parallel to a referenced diagonal edge; 5 perpendicular in the drawing plane; 6 extension beyond an endpoint without typing an angle; 7 from-point alignment with a box corner; 8 point from two reference alignments; 9 drawing on an angled planar face without drifting off-plane; 10 axis lock + typed distance satisfies both; 11 no flicker between overlapping candidates; 12 two visually crossing but spatially separated edges give NO false intersection; 13 upstream node change is reflected (engine uses newly evaluated geometry); 14 responsive on a large mesh without unnecessary graph re-evaluation; 15 identical point/axis semantics across Line, Move and Rectangle. Use numerical geometric tolerances for correctness tests, separate from pixel tolerances for acquisition.

## 14. Definition of done

Common points and directions acquired accurately; axis, parallel, perpendicular, through-point relationships predictable; geometry can be referenced without clicking; inferences can be explicitly locked; feedback clearly says what a click will do; locations are 3D-correct; works with procedural output; reusable across tools; large meshes stay responsive; automated tests cover geometric correctness, candidate stability and key workflows. IMPLEMENTATION RULE: one coherent engine with shared candidate generation, constraint resolution, scoring and presentation; tools supply context and consume the result.
