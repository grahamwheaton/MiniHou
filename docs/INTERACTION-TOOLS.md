# Curve and transform tools

## Using the tools

Restart MiniHou after updating the source checkout. Create or Curves shelf > **Bezier** adds an editable cubic Bezier node. The existing `curve` node remains a polyline; old graphs are unchanged. Select the Bezier node to show its control points in the 3D view.

- Drag an anchor to move it and both tangents. Click an anchor to show its two tangent handles.
- Choose Auto, Vector, Aligned, Mirrored or Free in the inspector. Auto derives a smooth tangent from neighbours. Vector points toward each neighbour. Aligned keeps opposite tangent directions with independent lengths. Mirrored also keeps equal lengths. Free makes handles independent. Dragging an Auto/Vector tangent changes it to Free.
- Append adds an anchor; Shift-double-click in the view appends at the mouse in the last anchor's view plane. Split inserts a point halfway along the selected segment without changing its shape. Delete retains at least two anchors. Closed joins the final anchor to the first; Resolution controls samples per segment.
- During a curve drag, X/Y/Z toggles an orientation-axis constraint. Shift gives precision, Ctrl temporarily inverts snapping, Escape/right-click/pointer cancellation restores the starting curve. Numeric XYZ controls edit the selected anchor. Curve previews update locally during dragging; the graph commits on release.

Select points, edges, faces or **Object** in the left viewport strip, then use Move/Rotate/Scale (W/E/R). Object means a connected mesh island, not a Blender object datablock. Stored selection keys remain `piece`, and geometry groups/graph formats remain compatible.

The selected geometry's Move gizmo has axis arrows and a centre square for movement in the view plane. During a centre drag, X/Y/Z constrains an axis; Shift+X/Y/Z constrains the complementary plane. Press the same constraint again to release it. During an axis-handle drag, X/Y/Z switches the edited component. Rotation rings and scale cubes retain their parameter-specific meanings. Hover brightens a handle; active handles highlight, other handles dim, the pointer changes to a grabbing hand, and an axis guide plus numeric feedback shows the operation. Shift precision transitions preserve the current displacement. Escape/right-click/pointer cancellation restores the original parameter; a transform created for the cancelled gesture is removed and its original connections restored.

Selection operations store a coordinate frame in the transform node's `axis_rotation` parameter. Existing graphs default to an identity frame. Move may reuse a matching grouped transform. Rotate/Scale insert another transform for the next selection gesture, so successive operations use the chosen axes without silently reinterpreting previous Euler angles.

## Orientation semantics

Orientation applies to **selection transforms and curve constraints**. An existing node's parameter handles continue to edit that node's own frame/Euler parameters; changing the toolbar does not reinterpret stored parameters.

| Orientation | Points, edges, faces and Object selections | Curve controls |
| --- | --- | --- |
| Global | MiniHou geometry XYZ | MiniHou geometry XYZ |
| Local | Viewed transform's axis frame composed with its rotation; identity for other node types | Identity |
| Normal | Area-weighted selected/adjacent face normals define Z. For edges, the last selected edge defines projected X; otherwise X is a stable projection of geometry X. Degenerate/cancelling normals fall back to Local. | Local fallback; curve controls do not provide face normals |
| View | Screen right/up/toward viewer; follows the camera until the gesture begins | Same |
| Cursor | Explicit XYZ Euler angles in Settings | Same |
| Custom | Capture frame stores the selection's current Normal frame | Uses the captured frame |

MiniHou modifier geometry is in host object coordinates. “Global” here is **not Blender world space**; upstream transforms on arbitrary graphs do not carry a reliable object frame. Cursor orientation is a MiniHou setting, not synchronized to Blender's 3D cursor. Custom stores one frame in browser settings, not a named project library. There is no Gimbal toolbar mode; existing transform-node rotation rings already represent the Euler parameter axes.

## Snapping

Enable Snap in the viewport toolbar. Settings persist in the browser profile. Ctrl temporarily inverts the toggle *after* starting a gizmo drag; Shift/Ctrl held while clicking over a mesh handle still add/remove selection. Settings expose positive move, rotation-degree and scale increments, face backface culling and inclusion of selected geometry. The selected geometry is excluded by default. Targets come from the currently previewed mesh, frozen at the start of a mesh drag.

| Element | Implemented behavior |
| --- | --- |
| Increment | Quantizes displacement relative to the starting snap base in the gesture frame |
| Grid | Quantizes absolute coordinates relative to the frame's origin |
| Vertex | Nearest projected target vertex within 18 CSS pixels |
| Edge | Closest projected edge position, interpolated with perspective correction |
| Edge midpoint | Halfway along a target edge |
| Edge perpendicular | Perpendicular foot from the original base onto the edge segment |
| Face | Nearest ray/triangle intersection under the pointer, optionally culling backfaces |

Snap Base offers bounding-box Center, point-average Median, Active (last selected point/edge/face centre), and Closest vertex. For Object selections, Active is the last face added to the selected island, since there is no distinct object origin. Curves use the dragged control as the base. Axis and plane constraints project the snapped displacement into the allowed direction/plane; consequently the target itself may be unreachable under that constraint. The yellow marker identifies the target. Rotation and scale use their increments regardless of the geometry element selected; they do not align to target normals.

## Blender parity assessment and remaining work

Reviewed against the official [transform orientation](https://docs.blender.org/manual/en/5.0/editors/3dview/controls/orientation.html) and [snapping](https://docs.blender.org/manual/en/5.0/editors/3dview/controls/snapping.html) manuals. This is a functional subset, not complete parity.

| Area | Status / remaining gap |
| --- | --- |
| Bezier anchors/tangents | Implemented; no multi-control selection, tilt/radius, multiple splines per node or NURBS |
| Move/Rotate/Scale | Interactive handles, free move, precision, constraints and cancellation; no trackball rotation, numeric modal entry, negative/cross-zero mouse scaling or Blender G/R/S modal keymap |
| Orientations | Six defined modes above; no Gimbal selector, Blender world/object metadata or cursor synchronization |
| Snap elements | Seven choices above; no Volume, Face Nearest or per-element Face Project |
| Snap base | Four choices; Closest is selected vertices rather than Blender object bounding-box bases |
| Snap settings | Increments, selected-geometry inclusion and face culling; no multiple simultaneous elements, align rotation to target, scene-wide selectable-object filters or snap-point accumulation |
| Pivot | Selection bounds centre; no individual origins or cursor pivot |
| Undo | Existing edit history retained; gizmo streaming can produce multiple undo entries per drag. Curve dragging commits one edit. Cancellation is an explicit restoring edit, not history erasure |
| Scale/performance | Existing bounded preview streaming retained; geometry snapping scans candidates rather than using a spatial index. Million-element snapping needs further profiling |
| Host validation | Numerical and real headless Chromium tests; native WebView2/Blender interactive acceptance on the production scene remains unverified |

The home-only Escape commit `c851906` was unavailable in this checkout. Cancellation here is an independent implementation and must be reconciled before cherry-picking that home commit.

## Validation entry points

- `tests/test_interaction_tools.py`: Bezier endpoints/topology/modes/invalid coordinates/sweep; oriented translation, scaling, normals and handle directions; legacy identity-frame compatibility.
- `tests/test_editor_e2e.py`: actual anchor/tangent dragging, mode changes, splitting and cancellation; axis and centre snapping reaching engine geometry; parameter/node cancellation; orientation round trips and snap candidates; existing editing/selection/preview streaming regressions.
- `tests/test_node_menu.py`, `tests/test_shelf.py`: combined Claude catalogs expose every built-in node, including Bezier and Object Input.

Validation on this Windows checkout: the broad run completed 1,057 tests with 162 skips and one temporary-file `PermissionError` in `SessionFileTests.test_open_and_create_files`. The subsequent 98-test file/browser/catalog run passed, including that class. A final nine-test interaction/numerical check passed. Blender 5.2 factory-startup background validation passed all 124 tests. The existing streaming benchmark delivered 30 preview frames during a 1.83-second drag; this is a regression check, not a claim of improved production-scene performance. UI capture: `build/validation/bezier-interaction.png`.
