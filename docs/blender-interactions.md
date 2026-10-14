# Blender-style viewport interactions in MiniHou

This describes the combined `build/blender-interactions` source checkout. Restart its MiniHou engine and reload the editor after updating. These tools operate on MiniHou geometry and graph nodes; they do not provide complete Blender viewport parity or manipulate Blender object datablocks directly.

## Select and transform

Use the left strip or **1 / 2 / 3 / 4** for points, edges, faces or Object. Object selects a connected mesh island. Click to select, Shift to add, and Ctrl+Shift to remove. Work at full preview detail/full cook when an operation reports that reduced geometry cannot be edited.

**G / R / S** starts a modal Move / Rotate / Scale with the pointer over the viewport. The left Move / Rotate / Scale buttons also start these operations. Move the pointer, then click or Enter to confirm; Escape/right-click cancels. Dragging the visible gizmo remains available and confirms on release. Shift provides continuous precision; Ctrl temporarily reverses the Snap toggle. Move supports X/Y/Z axis constraints and Shift+X/Y/Z complementary planes; repeating a constraint releases it. Rotation/scale constraints depend on the available parameter handles. Numeric transform typing and trackball rotation are not implemented.

If no suitable transform target is available, the editor switches to point mode and attempts to select all geometry. Selection transforms create or reuse graph Transform nodes. Existing parameter handles still edit their own parameters.

Orientation choices are Global, Local, Normal, View, Cursor and Custom. Capture frame stores the current selection normal frame as Custom; Cursor orientation uses explicit Euler angles in Settings. Global means MiniHou geometry coordinates, including host-local geometry in modifier workflows. Cursor orientation is not synchronized to Blender's cursor. Gimbal orientation and individual-origin pivots are not provided. See [orientation and curve details](INTERACTION-TOOLS.md) and [curve controls](CURVE-TRANSFORM-CONTROLS.md).

## Selection gestures and routes

**B** starts Box select outside a transform. **C** starts Circle select; Lasso is in the left strip. Box/lasso replace by default, Shift adds, and Ctrl or middle mouse subtracts. Circle paints additions while dragging; Ctrl/middle mouse subtracts, wheel changes brush radius, and Enter finishes. Escape/right-click restores the selection from before the selection session.

Region selection uses projected points, edges with both endpoints inside, and face triangle centres. It selects through geometry rather than providing full visibility-aware Blender selection.

Select a starting element, then **Ctrl-click** the destination to select a connected route. The **Ctrl-click path** selector chooses:

- **Most direct**: geometrical route preference using length, distance from the endpoint chord and bend penalties. This can favour a straight detailed chain over a fewer-step detour.
- **Shortest (fewest steps)**: minimum-hop topology route, not minimum physical length.

Most direct is not an edge-loop recognizer and irregular topology can produce ambiguous alternatives. Disconnected endpoints have no route. See [selection details](viewport-selection.md).

## Snapping and picked move base

Enable **Snap**, choose its element and base, and use Settings for increments, selected-geometry inclusion and face backface culling. Elements include Increment, Grid, Vertex, Edge, Edge midpoint, Edge perpendicular, Face and the combined **Vertex + Face** preset. Base choices are bounds Center, point-average Median, Active and Closest vertex. Targets come from the previewed geometry, frozen at mesh gesture start. Geometry snapping applies to movement/curve controls; rotation and scale use their increments.

For a picked move base: select geometry, start **G**, press **B**, point at the desired source base, then click to set it. Continue moving and click/Enter to confirm. Escape while picking returns to movement; Escape again cancels the transform. This is a temporary base for the active move gesture. It is not a reusable reference point or a rotation/scale pivot, and curve-control transforms do not provide this picked-base workflow. An axis/plane constraint projects the snap displacement into the allowed direction, so it can prevent reaching the target exactly.

There is no Volume/Face Nearest mode, normal-alignment snapping, arbitrary simultaneous snap-element picker, snap-point accumulation or scene-wide Blender object filtering. The combined preset is specifically Vertex + Face.

## Proportional transforms

Select seed geometry, open **Proportional editing**, enable it (or press **O** with the pointer over the viewport), set Radius/Falloff, then transform with G/R/S or a gizmo. Wheel during an active mesh handle transform adjusts radius. Selected points receive full influence; nearby points receive weighted translation, rotation and scale. The resulting Transform node stores the settings.

Falloffs are Smooth, Sphere, Root, Sharp, Linear, Constant and deterministic Random. Radius uses geometry units and distance to the nearest selected seed. **Connected only** follows actual mesh/curve edges and leaves disconnected geometry unchanged. **Projected** removes depth along the captured view direction; Connected takes precedence when both are enabled. An empty Transform group means all geometry is selected, so choose a subset to see a falloff.

There is currently no drawn influence-radius circle or weight-colour overlay, and proportional editing does not affect Bezier control editing. Connected distance is an edge path rather than a continuous surface geodesic. Projected distance is an orthographic plane metric; weighted rotations use Euler angles. Random is stable MiniHou sampling rather than Blender's sequence. Point normals receive the local weighted transform, without derivatives of the influence field. See [engine behavior and performance limits](proportional-transform.md).

## Modeling, curves and navigation

Select appropriate geometry, then use **E** for Extrude, **I** for Inset, **Ctrl+B** for Bevel or **Alt+E** for Push/Pull. Move the pointer to adjust the scalar amount; Shift gives precision, Ctrl quantizes the offset, and Bevel wheel changes divisions. Click/Enter confirms, Escape/right-click cancels. These are node-backed scalar gestures, not every Blender operation option. See [modal modeling](modal-modeling.md).

Bezier nodes expose anchor/tangent handles and Auto/Vector/Aligned/Mirrored/Free modes. The left **Draw curve** tool lays out points and creates an Auto Bezier curve. Existing control Move/Rotate/Scale acts on one selected control/handle; multi-control selection, tilt/radius and NURBS are not supplied.

Use Numpad 1/3/7 for axis views (Ctrl gives the opposite), Numpad 5 for perspective/orthographic, and Numpad decimal/Home/F for framing. **Walk / Fly**, Shift+backquote, or holding right mouse starts fly navigation. WASD moves, Q/E adjusts height, wheel changes speed; Enter confirms a latched flight and Escape restores its starting view. This is free flight without ground collision/gravity.

## Primitive drag placement

Drag a primitive from the **Create** shelf into the viewport. **Box, Sphere, Grid, Tube and Circle** show a dashed projected ghost before drop. Drop places its centre on the XY construction plane at Z=0 and adds the node; sources without a centre parameter receive a following Transform node. Other source nodes can be dropped but do not have a ghost outline. Non-source tools are rejected by viewport placement.

The ghost uses registry defaults and lightweight wireframes without cooking geometry. Sphere is shown as three great circles; grid preview subdivisions are capped. This is construction-plane placement, not snapping to arbitrary scene surfaces, interactively sizing a primitive or previewing every source type. An edge-on construction plane cannot be intersected; orbit toward it before placing. The ghost clears on leave/drop/drag end/Escape.

## Sketch, dimension and tape

The left strip supplies Line, Rectangle, Circle, Draw curve, Dimension and Tape measure. **Sketch & Measure** adds plane selection (XY/XZ/YZ), grid spacing, Pen/Polygon and Clear measurements. Click points to draw; two-point shapes finish automatically. Enter finishes a multi-point sketch, Escape/right-click cancels, Backspace removes the last point, X/Y/Z constrains an in-plane axis, and typing a positive length controls the next segment. Ctrl temporarily inverts sketch snapping.

**Dimension and Tape measure invoke the same two-endpoint measurement tool.** Pick two positions to show a line and a four-decimal distance in scene units. Measurements remain in this browser session until Clear measurements or reload; they are not graph nodes, saved annotations, constraints or exported dimensions. Sketch inference is construction-plane based, using endpoints/midpoints/grid and simple axes, not arbitrary 3D surface inference. See [sketch details](SKETCH-TOOLS.md).

## Array helper library

Load the bundled `nodes` library (normal app startup does this), add a system from Library/node search, connect geometry to **source**, and edit its exposed parameters. An unconnected source uses a small preview box. Double-click to inspect the graph.

| System | Controls and placement |
| --- | --- |
| Array (`array`) | Count, XYZ Offset and Origin; first copy at Origin |
| Circle Array (`circle_array`) | Count, Radius, Center, Start Angle, Rotate Copies; XY ring without a duplicate endpoint |
| Grid Array (`grid_array`) | Columns, Rows, signed Spacing X/Y and Origin; XY grid from Origin |
| Array Between Two Positions (`array_between_two_positions`) | Count, Start and End; includes endpoints; Count 1 places at Start |

Counts are 1–10,000; grid total is also limited to 10,000. The existing Copy to Points 256 MiB allocation guard prevents oversized realised outputs. Defaults are 5 linear, 12 circle, 5×5 grid and 5 between copies. Source local coordinates are retained; centre the source upstream when needed. For lightweight Blender object instancing, use the corresponding `array_*_points` generator followed by Instance. Ready examples and further usage are in [array helpers](ARRAY-HELPERS.md).

## Requested versus delivered

| Requested area | Delivered in this checkout | Remaining boundary |
| --- | --- | --- |
| Blender-style transforms | G/R/S, gizmos, constraints, precision, confirmation/cancellation | No numeric modal entry, trackball, complete Blender pivot/keymap parity |
| Pick snap base with B | Temporary base in mesh modal Move | Not curve controls, rotation/scale pivots or saved references |
| Proportional editing | Seven falloffs, radius/wheel, connected/projected, weighted transforms | No radius/weight overlay; metric/rotation/normal differences above |
| Viewport selection | Box/lasso/circle, direct and shortest route choices | Through-selection semantics; no full loop recognition |
| Modeling interaction | Extrude/inset/bevel/push-pull scalar modal gestures | Limited operation options and topology-specific restrictions |
| Curve editing | Bezier controls and tangent modes; drawing creates curves | Single-control editing; no NURBS/tilt/radius |
| Houdini-style drag ghost | Five primitive outlines and XY drop placement | Other sources have no ghost; no scene-surface placement or drag sizing |
| Dimensions/tape | Two-endpoint session measurement | Same tool under both labels; no saved parametric dimensions |
| Array helpers | Four reusable bounded graph systems and lightweight targets | Realised-copy memory cap; no automatic source-pivot fitting |
| Navigation | Axis/ortho/frame controls and free fly | No walk collision/gravity; production-scene performance unproven |

## Validation boundary

Small engine tests and real headless Chromium fixtures cover helper layouts/allocation guards, region selection and route preference, proportional math, modal gesture lifecycles and projected placement outlines. Array work passed seven helper tests plus 21 existing library/example tests; placement outlines passed two real Chromium tests. The combined editor integration has additional tests run by the integrating agent; report their current run separately rather than treating module fixtures as full application proof.

Native WebView2/Blender interaction on the user's production scene remains an acceptance step. Automated fixture success does not establish complete Blender parity, production-scale performance or host-app behavior. Earlier documentation may describe prior milestone gaps; this file describes the current combined implementation and its remaining limits.
