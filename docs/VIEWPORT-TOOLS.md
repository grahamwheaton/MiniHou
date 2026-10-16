# Viewport tools

Build with `python tools/build_app.py --with-accel`. The default delivery is the ready-to-run `dist/MiniHou` folder; ZIPs require `--zip`. `--output` selects a distribution directory. The new app is smoke-tested before replacing the previous folder, which is preserved under build/app-previous-*.

## Viewport layout

The 3-D view is laid out like Blender's. A **header** runs along the top: the selection-mode chip, the **View**, **Select** and **Add** menus, the transform orientation and snapping, and on the right the source (display flag or the selected node), the spreadsheet, frame-selected, the display-panel toggle and maximise. **View** holds the front/right/top/back/left/bottom viewpoints, perspective/orthographic, Walk / Fly, frame all/selected and the spreadsheet; **Select** switches between orbit-only, points, edges, faces and objects (and paint / quick assign); **Add** opens the node menu and search. The **tools** run down the left (selection modes, then Move / Rotate / Scale when a selection or handle node is active, then Paint and Assign). The **display settings** are a collapsible panel on the right (Shading, Overlays, Detail with the cook proxy and Performance, Sketch & Measure); the ☰ button in the header hides it and the browser remembers whether it is open and which sections are folded.

Changes from the design review (14 October):

* **Header**: the transform/snapping settings are behind the ⚙ button and the spreadsheet is the ▦ button. When the pane is
  narrower than about 760 px the mode chip and the node name hide (the left toolbar and the Parameters panel show them).
  The **Ctrl-click path** choice (most direct or shortest) is in the **Select** menu.
* The **quick-parameter strip** under the header appears only when the Parameters panel is hidden (▤) or the view is maximised,
  since otherwise it repeats the panel.
* Tool hints never run into the axis gizmo; in a narrow view only their title shows.
* The timeline stays on one row; its slider shrinks first.
* The editor's layout buttons (Graph / Split / 3D, swap, stack, panel) sit at the right of the menu bar: one header row instead of two.
* Opening a graph frames it at a readable zoom (85% or more). A graph too big for that shows its end, where the display node is;
  **Home** or **F** still fit everything.
* Pressing **Escape** to stop a cook shows a neutral notice, not a red error.

Navigation preferences now default to Blender: middle mouse orbits, Shift-middle pans, Ctrl-middle dollies. Geometry-depth orbit is optional. Houdini / MiniHou retains the earlier Alt-left navigation. Right-mouse WASD free flight remains available in both modes.

Ctrl-R over the viewport starts Loop Cut. Hover a quad edge for a cut preview; wheel changes count. Click to start sliding, click again or Enter to commit. Right-click during slide commits centered cuts; Escape cancels without creating a node. Uses the existing PolySplit engine and inserts into the displayed stream. Multiple cuts are evenly spaced; only single cuts slide. Stops at non-quad faces; this is not arbitrary knife cutting or full Blender edge-slide parity. The native Rust port and its differences are in [RUST_LOOP_CUT.md](RUST_LOOP_CUT.md).

Match Size exposes target translation and six target-box size handles with box guides. Enable Scale in the node to edit target dimensions. Uniform fitting still follows the node's uniform option: dragging a non-limiting dimension may not change the fitted result. Face squares resize the target box while keeping the opposite face fixed when Translate is enabled; with Translate disabled resizing stays symmetric. Z justification is preserved. This is a six-face cage, not Blender's full corner/edge cage.

Lattice Grid exposes XYZ displacement handles, free view-plane point handles and cage guides. X/Y/Z constrains a plane drag; Shift+axis excludes that axis. Esc/right-click cancels. Connect an unmodified grid to Lattice rest and a second grid to deformed; selecting Lattice lets its handles edit a directly connected Lattice Grid. Other deformed-input node types should be edited at their source. Cages above 216 points omit handles for responsiveness; incompatible offsets reset when resolution changes. Curve anchors now have explicit orientation-aware axis handles in addition to tangent controls and Auto/Vector/Aligned/Mirrored/Free modes.

Selection: 1/2/3 choose points/edges/faces. Ctrl-click extends by shortest topological path (fewest edges, not geometric distance). Alt-click selects edge loops, Ctrl-Alt-click selects rings; Shift adds, Ctrl-Shift subtracts. Loops stop at non-four-valence vertices; rings traverse quads. Face selection derives the faces adjoining the chosen strip. These are not all of Blender's selection variants.

Holding Ctrl temporarily inverts snapping during transform and curve drags. PCG shelf icons now distinguish common filtering, transform, surface, attribute, merge and sampling operations.


## Segmented bevel

PolyBevel now has Divisions (1–64), Round/Flat shape, Convexity (0 flat to 1 round), Limit Overlap, and Exclude Edges. Existing one-division graphs retain their chamfer. For a rounded bevel start with Divisions 4 and Convexity 1. Output Groups includes Point Fillet Polygons/Edges, Edge Fillet Polygons, Offset Edges/Points and Merged Points; groups follow the segmented topology. Merged Points stays empty because this implementation limits offsets rather than merging collisions.

Corners where three or more bevelled edges meet are built the way Blender's vertex mesh is (`bmesh_bevel.cc`, `adj_vmesh`): each corner of the corner polygon owns a grid of quads, and the grids meet in the middle, at a point for an even number of divisions and at a polygon (a triangle for a box corner) for an odd number. The boundary is the profile of the strips that end there; the inside is a Coons patch pushed onto the sphere through that boundary, so the corner of a box is an exact octant of a sphere (and the corner arches like the edges do) for any number of divisions. Convexity blends that sphere toward the flat patch, Flat shape keeps the patch flat, and a corner no sphere fits (a saddle where convex and concave edges meet, or where the edges are almost flat) stays a plain Coons patch. A corner that is a cap (an open end of a bevel) is still closed with a simple fan.

Round strips use tangent rational quadratic profiles. Corner junctions use shared-boundary triangle fans, not a rolling-ball patch solver. A cross-section curve on the second input (as in Houdini) replaces Shape and Convexity: the curve runs from the first to the last point, which are the bevel's two ends, and its offset from that chord, measured in chord lengths and positive towards the corner being rounded, is the profile, sampled at Divisions segments (a bulge of 0.207 is the quarter circle). Houdini-specific profile ramps, slide-on-ring overrides, asymmetry tolerance, distance scaling modes, and global collision merging are not implemented; controls for unsupported algorithms are not shown. Terminating selected edges now create endpoint caps, including single box edges.

Loop-cut commits now retain the current camera, including right-click to center the cut.


F over the viewport frames the current component/object selection, or the displayed geometry if no components are selected. Wheel zoom uses the surface depth under the cursor when available, slowing near geometry; over empty background it falls back to distance-scaled zoom.


## Gizmos and helpers

A selected node shows its **handles** (drag to change a parameter) and **guides** (outlines of the region it works on, dashed, not grabbable) in the 3-D view, and the **bounds** checkbox in the viewer's top bar outlines the displayed geometry's bounding box with its X/Y/Z size.

| node | handles | guides |
|---|---|---|
| `circle`, `torus` | radius arrow (rides on the shape) | |
| `grid` | size X / size Y arrows on the edges | |
| `soft_transform` | move / rotate / scale (W, E, R), radius arrow | falloff sphere |
| `twist` | strength ring | axis |
| `squash_stretch` | amount (multiplies) | axis |
| `linear_taper` | length arrow | axis, start plane |
| `bend` | length, bend and twist | capture region, start plane |
| `clip`, `mirror` | origin x / y / z | the plane, with a normal tick |
| `revolve` | angle ring, origin | axis |
| `uv_project` | move / rotate / scale of the projection frame | projection centre |
| `copy_transform` | translate | |
| `group_create` | box centre and size (box method or bounds filter) | the bounding box, or the sphere |
| `match_size` | target position and size | target box |
| `falloff` | radius arrow (sphere) | falloff sphere, ramp line and start plane, axis, wave direction (by shape) |
| `slice` | plane spacing | first plane |
| `sdf_shape` | radius arrow | bounds |
| `curve`, `bezier` | (viewport curve tool) | |

With a bounding object connected to `group_create`, the guide shows that object's bounds and the box arrows go away (their parameters are ignored).

Library nodes declare both in code: `@node(..., handles=[Handle("radius", axis="x", at="@center", reach=True, when="mode=sphere")], guides=[Guide("sphere", at="@center", size="@radius")])`. `Guide(shape, at, axis, size, id, label, color, when)` draws a `box` (size = full size, Vec3 or number), `sphere` (size = radius, or a Vec3 of full sizes for an ellipsoid), `circle`, `plane` (a square with a normal tick), `line`, `bounds` (of the node's output) or `axes`; `at` and `axis` take the same forms as for `Handle`, `size` is a number or `"@parameter"`, and `when` ("flag", "name=value", "name!=value", joined by "&") shows a handle or guide only while its condition holds. Providers in `minihou/handles.py` can return guides too (`handles.guide(id, label, lines)`).

## Interaction refinement

Graph/file, Undo/Redo, Add Node, Find, Note, Tidy and Frame commands are inside the node editor pane. Shift-click D adds or removes extra visible nodes; plain click D chooses one display and clears extra displays. Extra visibility is preview-only and does not add export outputs. Selecting components from combined visible geometry creates an explicit merge when editing.

In the viewport W/E/R selects Move/Rotate/Scale; Shift+E extrudes selected faces. Graph shortcuts remain scoped to the node pane. Text inputs do not invoke tools. Fly navigation consumes its own movement keys.

Bezier anchors and selected tangents share oriented move axes, rotation rings and scale tips. Rotate/scale act around the anchor (selecting the anchor transforms both tangents). X/Y/Z constrains; Shift+axis excludes an axis for Move; Ctrl temporarily inverts snapping, using move/angle/scale increments as appropriate. Auto/Vector switches to Free on manual editing. Rotation without an axis uses the orientation Z axis. Surface snapping applies to Move; rotation/scale snap increments only.

User preferences includes an explicit Python execution checkbox, off for a fresh user. It is stored in the user's MiniHou preferences.json separately from graphs and browser presets. CLI --allow-python and MINIHOU_PYTHON_NODES remain supported as startup overrides. Python executes with normal account permissions; enable only for trusted graphs.

## Viewport toolbar and Knife

The viewport uses a compact grouped two-column tool strip with vector icons, accessible names, shortcut tooltips and focus outlines. Wide panes show labels; narrow panes use icons. Display settings remain collapsible. Knife hides transform handles while active and has a compact options strip with a Controls disclosure.

Press **K** or click Knife. Click vertices, edges, face interiors or empty space to build a cut; drag to draw a continuous stroke. Green markers preview crossings of mesh edges. Double-click closes a loop. Right-click finishes the current path so another can be drawn. Enter/Space applies all paths as one `knife` node and selects the resulting cut edges; Escape cancels. Ctrl+Z removes the previous click/stroke before commit. Graph Undo removes the entire committed operation. Middle mouse navigates and starts a new path.

| Blender Knife behaviour | MiniHou implementation |
| --- | --- |
| Free cuts across multiple faces, edge and face interiors | Perspective-correct projected cuts, including concave polygons, crossing paths, interior endpoints and closed loops |
| Vertex/edge/prior-cut snapping | Automatic; Shift snaps edge midpoints; Ctrl temporarily ignores snapping |
| Cut through / occlude | C or Cut through checkbox; visible-only is default. Toggle affects all pending strokes |
| Selected only | Shift+K or checkbox; selected faces, or faces whose complete boundary is selected in point/edge mode |
| Axis constraints | X/Y/Z cycles global axis, local axis, free |
| Angle constraints | A cycles free, screen, face-relative; type increment or use its field. R cycles face reference edges |
| Measurements | S cycles distance, angle, both, hidden |
| X-ray cut overlay | V or checkbox; separate from whether the cut affects occluded geometry |
| Multiple editable objects | Displayed/selectable geometries are merged before cutting, in the same undo transaction |
| Data preservation | Float point/corner attributes interpolate; discrete attributes inherit a source; primitive attributes and split-edge crease/groups are retained |
| Result selection | New cut edges are selected and stored in edge group `knife` |

Comparison reference: [Blender Knife manual](https://docs.blender.org/manual/en/4.4/modeling/meshes/editing/mesh/knife_topology_tool.html).

The implementation covers these interaction modes, but exact Blender equivalence is not claimed. MiniHou merges editable objects into one procedural stream rather than retaining Blender object edit datablocks. It cuts polygon meshes, not loose wire curves or instances. Non-planar faces follow MiniHou's tessellation; closed interior loops may need bridge edges because a polygon cannot contain a hole. X-ray-off overlay visibility is evaluated at path vertices, so a line crossing an occluder can differ from Blender's pixel-level overlay. Each operation accepts at most 512 segments; confirm and start another operation for longer cuts. This is not the separate Knife Project operator.

Cuts are stored as view-projected segments. Changing upstream topology later recomputes their intersections; they are not a topology-independent sculpt history. A changed source during an uncommitted cut is rejected. Invalid cuts fail before the graph transaction is committed. Large or pathological meshes have not been benchmarked for Blender-level performance.

`examples/graphs/knife_demo.json` demonstrates a closed interior cut plus a crossing stroke. Automated tests cover closed-shell/volume preservation, 40 seeded crossing-cut fixtures, perspective, concave faces, attributes, multi-object merging, failure rollback, and real Chromium click/drag/key/undo workflows. These tests are distinct from a manual packaged Windows EXE acceptance pass.
