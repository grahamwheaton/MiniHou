# MiniHou native editor

Open a graph JSON file by passing its path to `minihou-app.exe`, or start the app without arguments for the demo graph. The top bar opens and saves files, changes shading, and opens Preferences. The shelf adds tools to the selected node; drag a tool into the network or viewport to add it there. The network editor selects, connects, duplicates, deletes, copies, and pastes nodes. The right panel edits parameters and shows geometry information. The centre viewport supports orbit, pan, zoom, mesh picking, view presets, and display proxy levels. The bottom strip shows cook state, file messages, and the Blender triangle cap when set.

Click a shelf tool to add it; tools with a required input connect after the selected node, while generators remain unwired and join the displayed set. Click a blue Visible flag to show one node, or Shift-click to add or remove it from the displayed set. The graph stores one displayed node as a string and several as a list. Tab over the network or viewport opens the add menu at the pointer; type to filter, use arrows and Enter to add, or Esc to close it. In the viewport Tab opens the menu in the Points/Edges/Faces modes too; Ctrl+Tab toggles object / edit mode (rebindable in Preferences, and the selection-mode buttons and 1-4 keys still switch modes). Ctrl+Numpad +/- (Ctrl+Alt+Plus/Minus) grow and shrink the viewport element selection. The selection nodes `grow`, `select_path`, `select_loop` and `select_ring` are only added from the menu; added while elements are selected they start from that selection (docs/RUST_SELECT_NODES.md). In the network editor, `Ctrl+C` copies the selected nodes and `Ctrl+V` pastes them with fresh IDs. Wires between copied nodes are preserved. Click a noodle to select it; Shift-click adds it and Ctrl-click removes it from the wire selection. Copy and paste continue to include only nodes.

Preferences are available from the top bar. The Viewport tab contains the Blender and Houdini / MiniHou navigation styles and the Auto proxy triangle budget. Blender navigation (the default) uses middle mouse to orbit, Shift+middle to pan, Ctrl+middle to zoom, and the wheel to zoom. Houdini / MiniHou uses Alt+left to orbit, Alt+middle to pan, and Alt+right to zoom. The viewport proxy menu has the Blender triangle cap; zero means unlimited. Preferences are stored in `%APPDATA%\MiniHou\preferences.json` on Windows.

Shift- or Ctrl-click nodes to toggle them in the network selection, or drag on empty canvas to box select nodes and noodles (Shift adds, Ctrl subtracts). A noodle is selected wherever the box intersects its drawn curve, including between its endpoints. `X` and `Delete` remove the selected set from either the network or object-mode viewport. `Ctrl+X` dissolves selected nodes and reconnects their consumers to the first input source. Shake a dragged card left and right to disconnect its input and output wires. These graph changes are each one undo step.

Hold Shift and drag RMB at least 5 screen pixels to slice noodles with a thin red/white freehand trace. Crossed noodles highlight red; releasing RMB disconnects all marked wires in one undo step, without dissolving nodes or reconnecting around them. Esc or focus/pointer loss cancels. A Shift-RMB click below the drag threshold opens the normal context menu. Selected noodles highlight in the theme accent; X, Delete and Ctrl+X disconnect them. In a mixed selection, the same edit also performs the usual delete/dissolve operation on the selected nodes. Wire selection is canvas UI state, clears when the graph changes, and is never saved in the document.

Viewport region selection works in object, point, edge, and face modes. Press B for box selection, C for a modal circle brush, or Ctrl+Alt+L to arm a lasso; B/C/L buttons also appear in the tool strip. Ctrl+RMB drag starts a lasso directly, and Ctrl+Shift+RMB adds. With the Select tool, LMB drag starting on empty space draws a box. Shift adds to box/lasso selection; MMB or Ctrl+LMB subtracts. RMB or Esc cancels. Circle paints additions continuously, interpolates strokes, and uses the wheel to change its radius from 4 to 300 screen pixels. Enter finishes Circle; Esc or RMB restores the selection from before the circle session. Regions select through geometry: points by position, edges only when both endpoints are inside, faces by triangle centre, and the displayed object by its bounds centre.

## Viewport drawing and cuts

The left strip groups Measure, the five drawing shapes, and Knife/Loop Cut with separators. Each shape has its own line icon and tooltip. Click Line, Polyline, Polygon, Rectangle or Circle to activate it; it stays highlighted for repeated drawings. Each completed drawing adds a curve node through the regular undo path. Two clicks complete Line/Rectangle/Circle; Enter completes Polyline/Polygon, and clicking near the first polygon point closes it. Esc or RMB returns to Select. The sketch chip offers XY/XZ/YZ planes, X/Y/Z axis locks, and keyboard length entry with document units; Ctrl toggles grid snapping.

K activates Knife: click a path across the mesh, Backspace removes its last point, and Enter applies all segments as one knife node and undo step. Ctrl+R activates Loop Cut: hover a visible quad edge to preview the new loop(s); the mouse wheel, PageUp/PageDown, or +/- sets the cut count (1-64, shown beside the cursor). Click to start sliding (the loop follows the pointer along the hovered edge; Shift fine, Ctrl snaps to 0.1), then click, Enter or release a drag to insert one polysplit node (split_edges pA-B, loops, position) as one undo step; Enter while hovering commits a centred cut, RMB or Esc cancels the pending cut and returns to Select. A confirmed cut also returns to Select. They require one successfully cooked displayed stream. The chosen node in the network does not redirect the cut. Pending Knife points are cleared if the camera or target changes. Loop previews stop at boundaries, non-quad faces and non-manifold edges.

The cut tools currently provide basic cuts: Knife uses visible surfaces, without vertex/edge snapping, cut-through toggles or selected-face filtering. Loop Cut follows the pre-rust tool (docs/RUST_LOOP_CUT.md): several cuts are evenly spaced (only a single cut slides), and there is no even/flip edge-slide mode. Multi-display editing is not implemented.

Element selection follows the pre-rust viewport (docs/RUST_ELEMENT_SELECT.md): in point/edge/face modes Alt+click selects a loop (Shift adds), Ctrl+Alt+click a ring, Ctrl+click the most direct path from the active element, Ctrl+Shift+click subtracts; with Houdini navigation Alt+click stays navigation. Faces select per polygon. Switching between point, edge and face mode converts the selection (down to constituents, up only to fully selected elements); Object mode keeps it hidden until an element mode is entered again.

## Default keymap

The table below is checked against `Keymap::blender()` by a unit test. Shortcuts in the Network context work while the pointer is over the network editor; viewport shortcuts work over the viewport. Alt+1/3/5/7/9 and Alt+Period are numpad fallbacks because egui cannot distinguish numpad keys. Emulate Numpad uses plain 1/3/5/7/9 for views and Shift+1/2/3/4/5 for viewport modes. In the modeling menu, 1-8 choose a numbered category or tool.

<!-- keymap:start -->
| Shortcut | Command | Context |
| --- | --- | --- |
| 1 | Points | Viewport |
| 2 | Edges | Viewport |
| 3 | Faces | Viewport |
| 4 | Object | Viewport |
| G | Move (modal) | Viewport |
| R | Rotate (modal) | Viewport |
| S | Scale (modal) | Viewport |
| B | Box select | Viewport |
| C | Circle select | Viewport |
| A | Select all | Viewport |
| F | Frame selected | Viewport |
| O | Proportional editing | Viewport |
| 5 | Paint | Viewport |
| E | Extrude (modal) | Viewport |
| I | Inset | Viewport |
| K | Knife | Viewport |
| Escape | Select none | Viewport |
| Ctrl+B | Bevel (modal) | Viewport |
| Alt+E | Push / pull (modal) | Viewport |
| Ctrl+R | Loop cut | Viewport |
| Alt+N | Flip faces | Viewport |
| Alt+M | Mirror tool | Viewport |
| Shift+C | Reset 3D cursor and frame all | Viewport |
| Tab | Add node menu | Viewport |
| Home | Frame selected | Viewport |
| Space | Play / pause | Viewport |
| Left | Previous frame | Viewport |
| Right | Next frame | Viewport |
| Shift+Left | First frame | Viewport |
| Shift+Right | Last frame | Viewport |
| Shift+R | Repeat last | Viewport |
| Shift+R | Repeat last | Network |
| Tab | Add node menu | Network |
| Alt+1 | Front | Viewport |
| Ctrl+Alt+1 | Back | Viewport |
| Alt+3 | Right | Viewport |
| Ctrl+Alt+3 | Left | Viewport |
| Alt+7 | Top | Viewport |
| Ctrl+Alt+7 | Bottom | Viewport |
| Alt+5 | Toggle perspective / ortho | Viewport |
| Alt+9 | Opposite view | Viewport |
| Alt+Period | Frame selected | Viewport |
| Alt+2 | Orbit down | Viewport |
| Alt+4 | Orbit left | Viewport |
| Alt+6 | Orbit right | Viewport |
| Alt+8 | Orbit up | Viewport |
| Ctrl+Alt+4 | Pan left | Viewport |
| Ctrl+Alt+6 | Pan right | Viewport |
| Ctrl+Alt+8 | Pan up | Viewport |
| Ctrl+Alt+2 | Pan down | Viewport |
| Alt+Plus | Zoom in | Viewport |
| Alt+Minus | Zoom out | Viewport |
| F | Frame all | Network |
| L | Tidy network | Network |
| O | Network box | Network |
| C | Modeling pie menu | Network |
| D | Display flag | Network |
| M | Mute flag | Network |
| E | Template flag | Network |
| W | Selectable template flag | Network |
| Ctrl+Z | Undo | Everywhere |
| Ctrl+Y | Redo | Everywhere |
| Ctrl+Shift+Z | Redo | Everywhere |
| Ctrl+S | Save | Everywhere |
| Ctrl+Shift+S | Save as | Everywhere |
| Ctrl+O | Open | Everywhere |
| Ctrl+D | Duplicate nodes | Network |
| Ctrl+X | Dissolve nodes | Network |
| Ctrl+C | Copy nodes | Network |
| Ctrl+V | Paste nodes | Network |
| Ctrl+A | Select all | Network |
| Ctrl+F | Find node | Network |
| Delete | Delete nodes | Network |
| Backspace | Delete nodes | Network |
| X | Delete nodes | Network |
| X | Delete nodes | Viewport |
| Delete | Delete nodes | Viewport |
| Ctrl+Alt+L | Lasso select | Viewport |
| T | Tidy network | Network |
| Alt+A | Select none | Viewport |
| Q | Select tool | Viewport |
| Z | Shading pie | Viewport |
| Shift+Z | Next shading | Viewport |
| Shift+Alt+Z | Previous shading | Viewport |
| Alt+Z | X-ray | Viewport |
| Ctrl+Tab | Toggle Object / edit mode | Viewport |
| Ctrl+Alt+Plus | Select more (grow selection) | Viewport |
| Ctrl+Alt+Minus | Select less (shrink selection) | Viewport |
<!-- keymap:end -->

## Viewport display

The cube dropdown follows Houdini's twelve display modes in menu order: wire/shaded object bounds, wireframe and wireframe ghost, invisible/ghost hidden line, then MatCap, flat and smooth shading with optional wire variants. Bounds use each displayed object's full-resolution point range. Hidden line writes surfaces to depth without coloring them; its ghost variant adds faint occluded edges. MatCap samples a built-in CPU-generated neutral clay sphere by view-space normal. Flat uses face normals; smooth uses interpolated point normals. Wire overlays retain the density fade.

The adjacent two-sphere dropdown shows all objects, ghosts unselected displayed objects, or hides them. Ghosts use faint alpha-blended surfaces and wires, do not write depth, and cannot be picked or outlined. Both controls persist in preferences; old Solid/Shaded/Wire/ShadedWire names still load. Z opens a four-way shading pie (4 wire, 6 flat, 8 smooth, 2 smooth wire), with All modes opening the full list. Shift+Z cycles forward through all twelve modes; Shift+Alt+Z cycles backward. Alt+Z toggles X-ray. X-ray leaves pure wireframe surface-free.

The clay lighting, icon artwork and ghost opacity approximate Houdini's look rather than copying its assets. Ghost transparency uses standard alpha blending, and clustered LOD wires follow the triangles of the current cut.

## Geometry visibility and overlays

The overlapping-circles header control has two independent targets: its icon toggles all decorative overlays, and its chevron opens the grouped popover. General contains Grid, Axes / Floor, Statistics, Bounds, Normals, Wireframe overlay and X-ray (Alt+Z). Geometry contains Points, Lines (curves / edges) and Faces, all enabled by default, plus point-size (1?16 physical pixels) and curve-line-width (1?10 physical pixels) sliders. These settings and X-ray persist in preferences. The master switch preserves each checkbox; standalone geometry remains visible when decorative overlays are disabled.

Standalone points and loose vertices use depth-tested screen-space squares; mesh vertices appear when the Points overlay is enabled. Selected points are orange in Points selection mode. Curves and open primitives use screen-space segment quads, including the sampled Bezier geometry supplied by the core. Their index lists survive regular proxy reduction and clustered LOD, with the same displayed-object ranges used for ghost/hide behavior. Closed Curve, Circle and Bezier sources carry a primitive attribute through merges so their boundary segments remain visible independently of wire shading. Existing closed-polygon surface semantics are preserved; Faces hides those surfaces too. Mesh wire shading retains its existing one-pixel, density-faded rendering. Faces visibility does not remove picking geometry or the selected silhouette.

The popover follows Blender's split control and grouping using MiniHou's theme and icon artwork. Curves use the core's sampled segments with square ends rather than analytic splines or joined strokes. The floor is a finite camera-scaled XY grid with XYZ axes.

## Modal transforms

With the pointer in the viewport and an object or edit-mode elements selected, G, R, and S begin a modal transform. The object updates as the pointer moves. The bottom viewport strip shows values, constraint, and snap state. The coloured constraint line passes through the pivot. Confirm creates one undo step; cancel restores the previous graph. The existing toolbar gizmo handles remain available. The following table is checked against the modal key list by a unit test.

In edit mode, E starts PolyExtrude on selected faces, edges, or points. Faces produce walls, edges produce quads, and points produce lines. Drag along the projected surface normal; X, Y, or Z constrains the extrusion to a world axis. Alt+E starts Push/Pull: hover a face, click to pick it, then drag along its normal. I and Ctrl+B start Inset and Bevel. Shift gives precision and Ctrl quantises to 0.1. Bevel uses wheel or PageUp/PageDown to change Divisions from 1 to 64. Digits and decimal point enter an exact value. LMB or Enter confirms as one undo step; RMB or Escape restores the graph. The bottom viewport strip shows the current distance or offset and divisions where available.

Alt+M (or the shelf's Mirror button) starts the Mirror tool (Houdini's Mirror tool state plus a HardOps-style axis gizmo; details in `src/ui/mirror_tool.rs`). Six cubes surround the selected object (+X/-X red, +Y/-Y green, +Z/-Z blue). Clicking a cube adds a `mirror` node after the selected node (before the output) that keeps that side (Cut) and mirrors it across the bounds centre; Shift+click mirrors about the world origin. The negative cube sets Reverse. A second axis chains another mirror node; clicking the other cube of an axis already mirrored flips its side, clicking the same cube again removes that node. The plane of the active node is drawn with an origin dot (drag: move the plane in the view), an arrow on the kept side (drag: slide along it; Shift finer) and a ring (drag: rotate the direction; Shift snaps to 15 degrees). F then drag draws the plane (direction along the drawn line, through the start point; F then click on a face uses its normal). G toggles the input wireframe, H the plane. The Parameters panel edits the same node live. Enter or a click on empty viewport confirms the whole session as one undo step; Esc or RMB restores the graph exactly. In edit mode with faces selected the node uses Mirror Selected on those faces. Selecting an existing mirror node and pressing Alt+M edits it.

<!-- modal:start -->
| Input | During modal transform |
| --- | --- |
| G / R / S | Begin move / rotate / scale while the pointer is in the viewport |
| LMB / Enter / Space | Confirm |
| RMB / Escape | Cancel |
| X / Y / Z | Axis; again local axis; third press clears |
| Shift+X / Y / Z | Plane excluding axis |
| Middle mouse | Choose closest motion axis; again clears |
| Digits / . / - / Backspace | Enter exact distance, angle in degrees, or scale factor |
| Tab | Next move numeric field |
| Ctrl | Invert snapping |
| Shift | Precision movement |
| O | Toggle proportional editing |
| Wheel / PageUp / PageDown | Change proportional radius |
| B / C / Ctrl+Alt+L | Confirm transform and begin box / circle / lasso selection |
| A / Alt+A | Confirm transform and select all / none |
| Ctrl+Shift+LMB while moving | Pick snap reference point |
| R while rotating | Toggle trackball |
| Alt while scaling | Individual origins is not yet supported for multiple displayed objects |
<!-- modal:end -->

## Known limitations

Sketch & Measure uses displayed vertices, edge midpoints, edges and faces as pointer targets within 12 pixels, with endpoint priority. A coloured marker and label identify the target. Red, green and blue axis guides appear near the previous point; hovered edges also provide parallel and perpendicular directions. X/Y/Z and the right/left/up arrows lock an axis, down releases it, and Shift locks the current axis inference. Line stays active for successive points; clicking its first point closes a coplanar face, while an open coplanar stroke through an existing polygon splits that polygon. Enter commits the open chain.

Measure stores annotations in the graph document. Vertex picks store the node identity and point index, so a label follows a moved point while its topology remains valid. An unavailable anchor turns the label grey and keeps its last session position. Click a label to select it, then press Delete; right-click a label for Delete Measurement or Clear all, or choose Clear all under the Edit menu or use Clear Measurements near the bottom left of the viewport. These document edits use normal Undo. Edge and face measurement picks currently remain fixed points.

Hold RMB over the viewport canvas to fly: drag to look, W/A/S/D to move along the view, E/Q or Space/Ctrl to move along world Z, Shift to move faster, and wheel to change speed. Flight starts only after a drag or movement key, leaving plain RMB clicks available to cancel operators. Ctrl+RMB remains lasso selection. Disable **Fly navigation (RMB + WASD)** in Viewport preferences if desired.

Selection shortcuts finish a running transform before selecting. A confirming LMB click also selects a different object under the pointer; clicking the current selection or empty space only confirms. Select-tool region drags and Ctrl+RMB lasso can take ownership from handle drags. Choosing Select releases sketch and handle input state.

Element transforms write a `group` of point numbers. Edge selection includes both endpoints; face selection maps each picked display triangle to its three point numbers. Object transforms leave `group` empty. Proportional radius, falloff, and connected mode are saved on the transform node when proportional editing is enabled.

Modal rotation uses the nearest Cartesian axis to the view direction for its default axis. Face-normal sketch rotation, cursor orientation rotation, and scale individual origins across multiple displayed objects are not available yet. The scale cage creates `match_size` for one displayed object when it can represent the result with nonnegative scales; otherwise it keeps the transform node.

The Rust core currently cooks only the nodes marked with a green dot on the shelf. Copy and paste operate on the selected network set. egui does not distinguish numpad digits from the number row, so Alt+digit represents numpad keys; Preferences can emulate the numpad with plain digits. Lock and template flags are session state. Auto proxy reduces the displayed mesh; the Blender cap is a separate setting for the link.
