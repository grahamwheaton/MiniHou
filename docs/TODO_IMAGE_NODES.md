# QUEUE (ordered, highest priority first)

1. **Viewport select arrow on group and element fields** (purple highlight). Shared widget, see below.
2. **Advanced Flood Fill** (`flood_fill`).
3. **Full Array parity** (shapes, count methods, gizmos).
4. **Image-based nodes** (Image Density + Scatter option first).
5. **Low priority:** L-system is 2D (match the Python fix).

---

# TODO: Image-based procedural nodes (parked)

Status: Image Density and the Scatter density option implemented. Remaining build order: Image Sample, Image Mask, Image Displace, Image to Curves.

---

Work only in a git worktree: git worktree add -b opus/image-nodes ..\opus-image HEAD, then work in that worktree. Rust core (rust/crates/minihou-core) and native app (rust/crates/minihou-app). Read only what you need; print short output. Set CARGO_BUILD_JOBS=2 and CARGO_TARGET_DIR=rust/target_image. Run only targeted tests; no workspace tests, no release build, no push. Commit in small commits ending with: Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>. Keep the keymap table test green; regenerate param meta when params change (python tools/dump_rust_param_meta.py; every param needs a doc string). Reuse the existing image_file node and the Blender image path; do not duplicate image decoding. Do not change unrelated engine code.

BUILD ORDER: (b) Image Sample; (c) Image Mask; (d) Image Displace; (e) Image to Curves. Stop after any step if a blocker needs the user; report it.

(b) IMAGE SAMPLE: node `image_sample`: samples a colour or brightness at each point's UV (uv attribute) from the image attached to the geometry (per image_file); output attribute `sample_colour` (vec3) and `sample_value` (float). Tests: UV corners sample the corresponding pixels with the row-origin convention documented in docs/RUST_TEXTURE_RENDERING.md; out-of-range UVs use the wrap mode.

(c) IMAGE MASK: node `image_mask`: creates a point group (or face group) from image brightness/colour/alpha with a threshold and an invert toggle; output group name param. Tests: group membership on a 2x2 known image.

(d) IMAGE DISPLACE: node `image_displace`: moves points along an axis by image brightness at their UV: params axis (normal, x, y, z), direction (positive/negative), strength (float), midlevel (0.5), channel. Tests: zero strength is identity; brightness 1 with strength s moves by s along the axis.

(e) IMAGE TO CURVES: node `image_to_curves`: traces black/white boundaries of a binary image (threshold) into polyline curves with simplification (tolerance) and optional smoothing. Only if (a)-(d) are finished and time remains; otherwise document it as a follow-up.

ACCEPTANCE: load a black-and-white PNG, generate points concentrated in its dark areas with image_density -> scatter, and changing density interactively cooks without errors (test the cook path with two different density settings).

Report in 6 lines: what each node does, params, tests, the demo graph path, what was not built and why, and how to try the acceptance flow.

---

# Queued node-editor fix (not the image work)

- **Tab with nothing selected places the new node at the mouse position** in the node editor (not at the default spot). Attach to the next node-editor fix; check the existing Tab-menu insertion path (ui/network.rs, network_menu.rs) so a node created with a selection still follows its current rule.

---

# Queued node: Advanced Flood Fill (`flood_fill`)

Full brief supplied by the user (2026-10-17). Not started. Category: Groups / Selection.

- **Purpose:** select connected points, edges or faces from one or more seeds, spreading under constraints (barriers, attribute differences, limits), unlike Grow Selection.
- **Seeds and groups:** seed group, seed elements, Avoid group, Allowed group, Barrier edge group, Target group. Optional viewport **Pick Seed** (reuse existing picking, no new engine).
- **Connectivity:** domain (points, edges, faces); connectivity rules (shared edges default for faces, shared vertex optional). Must handle islands, open curves, isolated points, non-manifold geometry.
- **Limits:** max steps, max geodesic distance (follows connectivity), max elements (deterministic order), stop at boundary, sharp edges, max face angle, UV seams, material changes.
- **Attribute constraints:** scalar and vector attributes; comparison modes (equal, greater, less, range, tolerance); reference modes (seed, neighbour, user value); colour uses a documented distance rule. Must block traversal, not just filter results.
- **Directional flood:** direction vector, mode (any, positive, negative, along vector), max deviation, min alignment.
- **Flood to Target:** stop at first target or find all; include or exclude targets; deterministic with several paths.
- **Connected Regions mode:** no seed; outputs region_id, region count, optional largest/smallest region groups. Structure the region detection so a separate Connected Regions node can reuse it.
- **Outputs:** flood group, step distance, surface distance, frontier group, region id, reached-target flag, reached-element count. Output group combine: replace, union, intersect, subtract.
- **Performance:** BFS for steps, Dijkstra for distance, cached adjacency, no recursion, deterministic results, zero-copy where unchanged.
- **Stages:** A essential (domains, multiple seeds, avoid/allowed/barrier, step limits, output group); B constraints (distance, sharp/face angle, UV/material, attributes, directional); C modes (flood to target, regions, outputs); D interaction (viewport seed pick, live preview, barrier and frontier display). Complete and test each stage before the next.
- **Validation:** tests for each domain, islands, groups, barriers, non-manifold, limits, constraints, directional, target, regions, empty and invalid input, attribute preservation. Demo graphs: barrier-restricted face flood, distance-limited flood, attribute-tolerance flood.
- **Must not break** existing Grow and Group Expand.

---

# Low-importance todo: L-system is 2D

- **Problem:** the Rust `lsystem` node produces 2D output, but the pre-Rust (Python) version already fixed this. Make the Rust node match the Python behaviour (3D output, same turtle semantics as the fixed Python node).
- **Priority:** low. Not started.
- **How to start later:** compare the Python fix in `minihou/nodes` (grep `lsystem`) with the Rust node in `rust/crates/minihou-core/src/nodes/`, then add a golden test against the Python output.

---

# Queued: Array node full Blender parity (incl. gizmos)

The current `array` node (count, relative and constant offset, merge distance) is only a first step. Blender's Array modifier, per the user's screenshots (2026-10-17), needs:

- **Shape:** Line, Circle, Curve, Transform (4 modes).
- **Count method:** Count or Distance (fit copies to a distance).
- **Line:** count; offset method (Relative, Offset, End Point); relative X/Y/Z; constant offset X/Y/Z; per-copy rotation X/Y/Z and scale X/Y/Z.
- **Circle:** count or distance; central axis X/Y/Z; segment Full or Arc; radius; align rotation toggle.
- **Curve:** count or distance; per curve toggle; curve object (picked, with eyedropper, or a curve from the graph); relative space toggle; align rotation.
- **Transform:** transform reference Inputs or Object; translation, rotation and scale per copy.
- **Common:** Realize Instances; Randomize panel (seed, randomisation of offset/rotation/scale); Merge panel (merge distance, first/last copy merge).
- **Errors:** report "No Curve Selected" and similar in the panel, as Blender does.
- **Gizmos:** Blender-style viewport gizmos for the line offset (arrow per axis), circle radius and axis, and curve/transform handles, so the array can be adjusted in the viewport without typing numbers.
- **Parity target:** match Blender's Array output for each shape on the same inputs, with golden tests.

Priority: queued after the image nodes and Flood Fill. Not started.

---

---

# Queued: Viewport select arrow on group and element fields (Houdini parity)

- **What:** any node parameter that takes a group or element selection (e.g. Subdivide's Group and Creases, and every group field across the node set) gets a Houdini-style **select arrow** button beside it. Clicking it enters a viewport pick mode that writes the picked elements into that field.
- **Highlight:** the picked elements are shown in **purple** in the viewport while the pick is active and once stored, so the user can see what the field refers to.
- **Scope:** a shared parameter-panel feature, not one node. It affects quite a few nodes (anything with a group or element list: Subdivide Creases, Group, Delete, Blast, Extrude, Bevel, Edge Slide and others). Build it once as a shared widget and wire it through the existing group and selection infrastructure, rather than per node.
- **Reuse:** the viewport picking and selection code already in place; no new picking engine.
- **Priority:** queued. Not started.

---

# Queued: Houdini parity - Copy to Curves, Orientation along Curve, Scatter and Align

Target: match Houdini's SOPs for these three nodes (screenshots supplied by the user, 2026-10-17).

## Copy to Curves (Houdini `copytocurves`)
- Inputs: source geometry, curve geometry. Params seen: Source Group, Source Group Type (Guess from Group), Curve Group, Piece Attribute (variant) with Pack and Instance, Tangent Type (Average of Edge Directions), Make Closed Curve Orientations Continuous, Extrapolate End Tangents, Transform Using Curve Point Attributes, Target Up Vector (Curve Normal), Target Up Vector at Start (else Average), Use Target End Up Vector, Rotate Order (Pitch, Yaw, Roll), Apply Roll or Twist / Yaw / Pitch, Normalize Scales, Uniform Scale, Stretch Around Turns, Max Stretch, Attributes (Reset Attributes from Target).

## Orientation along Curve (Houdini `orientalongcurve`)
- Params seen: Curve Group, Tangent Type, Make Closed Curve Orientations Continuous, Extrapolate End Tangents, Transform Using Point Attributes, Target Up Vector, Target Up Vector at Start, Use Target End Up Vector, Adjust Up Vectors by Curvature, Additional Rotations, Scales and Shears, Output Attributes (Class Point, X Axis `out`, Y Axis `up`, Tangent (Z Axis) `N`, Translation `P`, Quaternion `orient`, 3x3 and 4x4 transform).

## Scatter and Align (Houdini `scatteralign`)
- Mode (Scatter Points on Geometry Surface), Group, Tag Name (`$OS`), Global Seed, Coverage, Point Count Method (By Size), Scaling mode, Min/Max Radius, Uniform Scale, Orientation tab (Alignment; Rotation with Max Random Cone Angle; Rotation Around Normal with Min/Max Angle, Round to Multiple, Offset Angle, Remap Using Distribution Ramp), Point Generation tab, Attributes tab.

Priority: queued after the select-arrow work. Not started. Note: Copy to Curves and Orientation along Curve depend on a correct curve frame (tangent, up vector), so build the shared frame code once and reuse it in both.

---

# DONE (removed): shelf red dots on Rectangle, Polygon and Repeat

- **Rectangle and Polygon** show a red "no Rust version" dot, but they are viewport drawing tools (`sketch_rectangle`, `sketch_polygon`), not graph nodes. The Python app never had them as nodes, so there is nothing to port. Fix: give tools their own marker (no red dot), or hide the dot for tool entries.
- **Repeat** points at `loop_block`. The Python app's repeat region is `loop_begin` and `loop_end`, which Rust already runs. Fix: map the shelf entry to the existing loop region and show the normal marker.
- Priority: low (cosmetic, but it misleads about what is missing).

---

# Queued: node input port tooltips and wide multi-input ports

- **Port tooltips:** hovering a node input port should name what to connect there (e.g. "points", "geo", "cutters"), as the pre-Rust editor did. The editor already shows hover tooltips on header buttons (ui/network.rs uses on_hover_text), so the same mechanism applies to ports. Source of each name: the node's input spec labels.
- **Wide port for multi-input nodes:** nodes that take many inputs (e.g. Merge) should draw a wider, pill-shaped port so several wires can attach clearly, as the pre-Rust editor did. Ports stay single-width for ordinary one-input nodes.
- **Where:** ui/network.rs (port drawing, canvas_ports) plus the input spec labels. Reuse the existing port hit-testing; do not change connection behaviour.
- Priority: queued after the current deformer and Scatter and Align work. Not started.

Native `scatteralign` implementation: area-weighted surface sampling of polygon faces, primitive Group filtering, By Size (surface area / mean-radius disk area) or Fixed count, Coverage multiplier, seeded random or uniform `pscale`, integer tag point attribute (`$OS` resolves to `scatteralign`), and `N`, `orient`, `primnum` output. The orientation uses local +Z for the surface normal or world up; cone deviation and spin are independent. Spin remapping currently accepts only a linear two-point ramp (start and end values), not Houdini's editable multi-knot ramp. Point Generation supports these count and scale controls; Attributes copies selected source point attributes (`*` for all), interpolating floats by barycentric weights and choosing the dominant vertex for integers. It does not yet copy vertex/primitive/detail attributes or generate Houdini's additional density/relaxation options. Volume mode reuses the existing scatter volume sampler and does not yet emit alignment, scale, tag, or copied attributes. The tag is an integer marker because the core geometry has no string attribute columns. Surface points are independent random samples, not a radius-constrained packing.

Priority: Copy to Curves and Orientation along Curve remain queued. Scatter and Align has the native subset documented above. The curve nodes depend on a correct curve frame (tangent, up vector), so build shared frame code once and reuse it in both.

---

# Future: Persistent instance overrides (Scene Objects)

Full handoff: docs/TODO_PERSISTENT_OVERRIDES.md (from the user's HANDOFF.md, 2026-10-17). Not started.

- **Goal:** Blender edits to location, rotation and scale on instanced objects survive procedural recooks, with identities kept.
- **Modes:** procedural (default, current behaviour), preserve (a hand-edited channel stays fixed), offset (edits are kept as a delta from the generated placement).
- **Needs:** a Reset to Procedural action in the add-on, persisted per-object state, and tests for reopening the file, retargeting and multiple objects.
- **Limits to document:** point-index fallback when point order changes; migration of older files; keyframes and drivers on transform channels.
- Priority: future. Lower than the queued items above.

---

# Added 2026-10-17: items raised in conversation, not yet on the list

- **Stage 1: self-registering node architecture.** Largely done (2026-10-18): standard nodes live in nodes/packages/<name>/ and are discovered at build time; shared Tab menu and shelf catalogue; guide in docs/NODE_DEVELOPMENT.md; packages example_offset, pack, unpack. Remaining: older nodes still have hand-written registry and param metadata in tools/dump_rust_param_meta.py, so adding one still touches shared files. Migrating them to packages is the open part (plan first).
  - Catalogue assertions now check ordering, uniqueness, and baseline nodes without freezing the package list. No separate TODO item covered this test cleanup.
  - Blast and Group Create now register through packages; their picker domains and optional panel sections come from package parameter manifests. The broader legacy-node migration remains open.
- **Ctrl+G grouping** (node grouping into subnets). DONE: commit 201879e7f (subnet_edit.rs, keymap Ctrl+G). Live check pending.
- **Loop Cut extras:** numeric entry, E (even) and F (flip) keys, and the redo strip. Not started.
- **Inference Phase 2 and Move-tool integration:** multi-reference, through-point locks, guides, bounding-box and object-origin snapping; Move gizmo using the shared engine. Not started.
- **Multi-output sockets:** separate constraint and proxy outputs for fracture and RBD nodes, from the user's node review. Deferred by the user until later.
- **Python parity suites on this machine:** the stale prebuilt extension blocks test_rust_parity and test_parity_inputs. Rebuilding it needs the user's go-ahead.
- **Inset on non-planar regions (sphere):** decide between per-face fallback, an actionable error, or an Individual toggle that turns on automatically. Argon's recommendation is still to be reviewed.
- **Tri to Quads:** needs the user's own testing before we call it done.
- **Texture transport limits:** 16-bit and float image precision, non-identity Mapping transforms, and a live-window smoke test. Clip is approximated by edge clamping.
- **Blender Object picker and waiting state:** live check in the real Blender window still to do.
- **Knife limits:** no live cooked preview, and no edge-axis constraints.
- **Grid Fill limits:** loops with holes are not detected.

---

# Decisions 2026-10-17 and new work

- **Python parity suites:** APPROVED. Rebuild the stale minihou_rs extension from current Rust so test_rust_parity and test_parity_inputs run. In progress (Bob).
- **Inset on non-planar regions:** PAUSED by the user.
- **Tri to Quads:** user will test before it is called done.
- **Attribute Wrangle parity:** not Houdini-equivalent yet. Bring the wrangle to Houdini parity (VEX-style expressions for points, primitives and vertices, bindings to attributes, and the groups/attributes it reads and writes).
- **Polyfill full parity:** Houdini Polyfill parameters from the user's screenshot: Boundary Group, Fill Mode (Quadrilaterals and others), Auto-Complete Boundaries, Reverse Patches, Unique Points, Update Point Normals, Edge Loop, Smooth, Patch Construction (Corner Offset, Custom Corners, Deform Patch, Surface Offset, Tangent Strength), Output Groups (Patch Group, Edge Loop Group, Append to Groups).
- **Statistics nodes:** measure volume, distance between points, bounding box size, curve length, and similar measurements, as nodes with output attributes or detail values.
- **Houdini shelf gap audit:** the user's screenshots list the Model, Polygon, Deform and Texture shelves. Check each Houdini node against MiniHou and port the missing ones (Model: Revolve, Loft, Skin, Rails, Sweep, Creep, Ray, Shatter, Carve, Convert, Refine, Reverse, Sculpt, Cluster Points, Edit, Visibility; Polygon: PolyDraw, Curve Polygon, PolyBridge, Edge Loop, PolyReduce, PolyExpand2D, Edge Collapse, Edge Cusp, Edge Flip, Edge Divide, Clip, Facet, Point Weld, Fuse, Snap, Smooth, Curve from Edges, PolyHinge; Deform: Bulge, Fractal, Mountain, Waves, Ripple, Morph; Texture: UV Project, UV Edit, UV Brush, UV Fuse, UV Flatten, UV Pelt, UV Texture, UV Transform, UV Unwrap, UV Quick Shade, UV Smooth, UV Layout).
# PRIORITY (set by user 2026-10-17, full-auto while away)

Standing rules:
- Bob: use up to 20% of weekly allowance (stop when 80% remains).
- Local Codex: use freely (5-hour window is the only limit).
- Claude (Opus/Sonnet): keep session usage under 90%; Sonnet or Opus allowed, pace yourself.
- Argon: advisor only. Flash agents: use as much as possible.
- Merge and push to native-ui automatically when the full test run is green; build the test exe and run the add-on updater after each push. Stop and leave for the user if anything is red.

Order:
1. Finish in-flight: fence + MCP test (Bob), deformer parity (Flash), Python parity suites (Bob).
2. Stage 1 self-registering node architecture.
3. Ctrl+G grouping: DONE (201879e7f). Live check pending.
4. Attribute Wrangle parity.
5. Polyfill full parity, then Houdini shelf gap nodes (Flash batches).
6. Statistics nodes (volume, distance, bounding box, curve length).
7. Loop Cut extras.
8. Image nodes, Flood Fill, Array parity, rest of list.
9. When the MCP works and the queue has no runnable jobs: archviz asset generators (wall, fence, road, tree scatter, roof), each with a demo; check pre-Rust examples for reference.

Parked by user: inset on non-planar regions. Tri to Quad: user will test.

## Requested 2026-10-18 (not started)

- **Attribute create node:** add the purple group-picker arrow to the attribute name, like group fields have.
- **Attribute create, run from the Tab menu in the 3D view:** auto-fill the selection as context (selected points, edges or faces) instead of an empty node.
- **Attribute and group fields dropdown:** when existing attributes or groups are detected on the input, show a dropdown of them; choosing one sets the domain automatically (for example point, edge, face).
- **Blender attribute transport:** a cube from Blender lost its `bevel_weight_edge` attribute on the way through MiniHou. Make sure all attributes and creases (bevel weight, crease, sharp, seam, UVs, colours) survive a round trip.
- **Scene Objects, option B (done 2026-10-18):** each object has its own mesh parented to a placement empty, so moving the empty moves the object. The mode is named `objects`; saved `pieces` values remain accepted.
- **Copy to Points:** pack is the default now (done 2026-10-18).

## Future: app icon and top-left logo (requested 2026-10-18)

Use the blue "M" logo (rounded blue square with a white M) as the app icon (window and taskbar, and the exe icon) and in the top-left of the editor, in place of the current icon. Needs the logo as an .ico for the exe and a PNG for the in-app header.

## Future: call the "primitives" domain "faces" (requested 2026-10-18)

The domain option shown as "primitives" (group_create, blast, and other nodes with a domain param) should read "faces" in the UI. Saved graphs store "primitives", so the parser must keep accepting it as an alias (map it to faces), and the metadata options and param meta regenerated. Check every `p.e("domain", ...)` list in rust/crates/minihou-core/src/nodes for the "primitives" value, and the Python parity tests that use the name.

## Later: rename the "primitives" domain value properly (requested 2026-10-18)

The UI now shows "faces" (display label only, in ui/params.rs). The stored value is still "primitives" in about 50 places (node code, metadata, tests). Do a proper rename to "faces": accept "primitives" as a legacy alias when loading saved graphs, regenerate the param metadata, and update the Python reference and its tests. Keep the alias until saved graphs are migrated.

## Queued: deformer parity and gizmos (requested 2026-10-18)

Plan from Argon (argon-deform-45cb8b): (1) add missing lattice, path and point deform params to capture_deform.rs and curves.rs; (2) a shared capture gizmo in a new deform_gizmo.rs (capture box wireframe, origin, direction and up handles, pure drag tests); (3) per-deformer handles: Bend arc, Twist ring, Taper squares and centre, Squish pivot, lattice/path/point handles; (4) keys F/G/H/B/N/M only when a capture deformer is selected and no modal or gizmo is active, plus the HUD legend and parameter grouping. Acceptance: numerical parity test against a Houdini reference within 1e-4.

## Queued: LoopTools nodes (requested 2026-10-18)

Source: https://extensions.blender.org/add-ons/looptools/ (Blender's LoopTools add-on: circle, loft, relax, space, flatten, gstretch, bridge and similar loop operations). Plan the node set first (Argon), then Flash builds the nodes as packages, one per tool, with tests.

## Queued: migrate older nodes to packages (requested 2026-10-18)

Migrate the legacy nodes to the self-registering package system, following docs/NODE_MIGRATION_PLAN.md (Bob's plan) and the pilot (measure_volume, measure_bounds, measure_area, measure_distance, measure_length, flood_fill). Argon to plan the order and batches; Flash to move each batch with unchanged ids, parameters and output, plus the registry list update.

## Bugs from testing b35cd5767 (reported 2026-10-18)

8. **Push/pull does not always punch through** (see screenshot 72.png: a diagonal band of faces stays unpunched on the cube). Not reproduced in the node: a rotated box with an inset top punches cleanly (regression test through_hole_on_rotated_box_has_no_unpunched_band); needs the scene from 72.png.

## Features from testing (reported 2026-10-18)

9. **Dissolve flat edges** node, as Houdini's Dissolve Flat Edges (https://www.sidefx.com/docs/houdini/nodes/sop/labs--dissolve_flat_edges-1.0.html): remove edges between coplanar faces, merging the faces, with a tolerance.
10. **Clean node parity** with Houdini's Clean (https://www.sidefx.com/docs/houdini/nodes/sop/clean.html): Consolidate Points with distance, Remove Degenerate Primitives with tolerance, Delete Unused Groups, Orient Polygons, Reverse Winding, Fix Overlaps, Remove Unused Points, Remove Attribs and Groups (patterns), Remove NaNs, Manifold-Only Topology. Screenshot 73.png shows the Houdini parameter layout to match.

## Testing feedback on ce2c51c36 (2026-10-18)

Bugs:
- **Boundary select:** should select only the boundary edges, create an auto-named group, and switch the viewport selection to edges.
- **Select sharp edges:** tags no edges in testing. When the node is selected, every edge it tags should show purple.
- **Faces dropped by selection:** selection still drops faces (tested, not fixed).
- **Re-cook activity:** dragging or wheel-adjusting a value re-cooks many nodes; check the cook path for parameter drags (only the changed node and its downstream should cook, and only on release unless live preview is wanted). Compare with the mouse-wheel path.
- **Hover on the purple arrow:** should highlight the selection in the viewport. Also add a demo graph that shows it.
- **Auto group selection and purple arrows:** the new select nodes, and relax points, need an auto group name and the purple arrow on their group fields.
- **Image file / image plane:** the Blender image plane does not render (screenshot 79). Image file shows no image preview; the height map is correct but the image is not displayed (screenshots 77 and 78).
- **Input group selector:** an input group selector with a purple arrow (item 11).

Features:
- **Color node:** Houdini parity (Group, Group Type, Delete All Existing Color Attributes, Set Color Attribute, Class, Color Type, Color). Check why nothing shows in the viewport.
- **Asset shelf I/O tab:** Blender object, file image, object input, USD and similar.
- **Image file default:** a flat image plane by default, renamed "Tessellation", with a height map as an option (no "too dense" error by default).
- **Output target button:** a target button next to each output parameter. Hover shows the output on the mesh (a float output can show as a strength level); click selects the group. Suggest a colour (not purple, which is the input selection colour).
- **Dissolve flat edges and Clean parity** (Flash, in progress); if the Clean/dissolve nodes cannot be found in the Tab menu, add them to the shelf.

## Feature: Subnet, Houdini parity (requested 2026-10-18)

Implemented in the Rust editor: double-click enters a subnet, Escape/U and breadcrumbs return to parent scopes; inner canvas edits and promoted numeric controls cook through the outer subnet. Live editor interaction still needs a host-app smoke test before this TODO can be closed.

## Feature (non-urgent): node frames (requested 2026-10-18)

Add frames (comment boxes) to the node editor, created with the O key. A frame has a name label and one of five colours to choose from. Frames move with the nodes inside them. Add the O key to the keymap table in rust/crates/minihou-app/docs/NATIVE_EDITOR.md (the keymap test compares it with Command::ALL).


# Added 2026-10-18: viewport should display Cd

- The viewport ignores the point/vertex colour attribute `Cd` (only Single and Random colour exist, in Overlays -> Color).
- Add a **Cd** option to that menu: interpolate Cd across each face when the geometry has it; default to Cd when present, else Single.
- Affects Image File, Image Density and any node that writes Cd. Small, self-contained viewport change. Not started.

## Feature: Scatter node, Houdini parity (requested 2026-10-18)

Surface sampling now has the Houdini parameter names and opt-in output attributes: primitive Group, Generate modes, density from point/vertex/primitive/detail, count and area controls, seed and point order, triangle-local relaxation, texture-space sampling, sourceprim/sourceprimuv/density/pscale/id, and numeric attribute transfer. The legacy count, seed, and default P/N/primnum output remain available.

Remaining: true volume primitives and voxel-independent/2D-volume options (the current volume mode samples polygon interiors), and string detail-to-point attributes. Keep this section until those cases work; current polygon-interior mode explicitly rejects unsupported voxel and relaxation settings. Relaxation now crosses shared polygon edges; primitive-attribute Override Prim Area is supported.


## Feature: middle-mouse scale bar slider, Houdini parity (requested 2026-10-18)

In Houdini, a middle-mouse click on a number field opens a vertical scale bar with preset steps (for example 100, 10, 1, .1, 0, .01, .001, .0001). Dragging the mouse up or down along it moves the value, and releasing sets it. Add this to number fields (ui/params.rs), with the same step behaviour as the drag sensitivity work.

## Feature: copy and paste parameter values with the mouse (requested 2026-10-18)

Hover over a parameter value and press Ctrl+C to copy it. Hover over another parameter and press Ctrl+V to paste it. Works for number, vector (all channels), text and enum values, and for group fields. Keep the copied value in the app (not the system clipboard only), so it works between nodes.

## Testing feedback round 3 (2026-10-19)

Bugs:
- Transforming a selection only works with an Output node at the end of the stream (should work without it; see the display fix fix A).
- Add a toolbar icon for scale cage and a dedicated node (or check whether Match Size covers it).
- Ctrl+B with bevels selected should trigger the vertex bevel node, with an auto group.
- Copy to Points: with pack on, nothing reaches Blender (it looks right in MiniHou).
- Blender Object: clicking reset transform has no effect in MiniHou (Blender changes the result).
- Scene Objects: after a reload, moved objects jump back to their MiniHou positions. Overridden empty positions must persist.
- Mirror gizmo has gone; restore it.
- Deleting nodes from an embedded Blender graph can make Blender unresponsive.
- Inset needs to auto-merge (parity with Blender). Vertex bevel needs parity with Blender and must flow correctly.
- Subnet double-click navigation is implemented; live editor smoke test pending.
- Ctrl+click shortest route works on simple meshes only; it breaks with split edges.
- Promoted subnet control editing is implemented; live editor smoke test pending.

Nodes and parameters:
- All output groups optional: checkbox, label, then group name with target select.
- Delete node: full Houdini parity.
- Blender curves input (support).
- Array: full Blender parity.
- Boolean parameters not working when exposed to Blender (check supported parameter types).


Future:
- Extrude and push/pull as one node.
- Input slots need tooltips.
- Gizmos for all deformers.
- SVG to curves input and output.
- Pivot control.
- Auto-merge checkbox (adds a Fuse node automatically).
- Transform UVs; transform origin.
- Audit the Maya modelling tools.
- File > New and the whole File menu; should the Blender link icon be in one unified menu?

## Future: Maya modelling gaps (audit 2026-10-19)

From the Maya polygon tools audit, the items the user picked:
1. Triangulate: split n-gons into triangles (needed for export and engine targets).
2. Conform: snap vertices onto a target surface (fitting clothing or props).
3. Append to Polygon / Quad Draw: grow faces from a selected border edge, and draw quads on a mesh.
5. Offset edge loop: loop_cut needs an offset parameter.
6. Connect: edge-to-edge connection (polysplit covers only part of this).
9. Wrap: shrink-wrap to a target surface (retopology).
10. Blend shape: morph targets.

- **Done: new nodes take the viewport selection as group input.** Tab menu, shelf, network add and connected add all call `selection_node_params`; `element_select::find_input_group_field` picks the group field and the domain comes from the selection type. Generators and object mode are unchanged.

## Bug: object-mode display shows points that belong to faces or edges (reported 2026-10-19)

In object mode, points are still drawn even when they belong to a face or an edge. Wanted: points that are part of a face or an edge are hidden; edges that are not attached to a face are shown; points that are not attached to any face or edge (stray points) are shown. So in object mode you see faces, stray edges and stray points, and nothing that is only a corner of a face.

## Bugs and todos from live testing (reported 2026-10-19)

- **Bug: extrude is uneven even when Offset Even is set.** Polyextrude shows an uneven offset regardless of the Offset Even checkbox.
- **Bug: viewport extrude selects the face under the mouse on left click.** Applying the extrude in the viewport should keep the original selected face selected.
- **Bug: transform of points, edges or faces does nothing without an output node at the end of the stream.** Transform should work on the selection regardless of downstream output.
- **Todo: F2 renames a node.**
- **Bug: U closes the subnet while inside it.** Remove the U keymap.
- **Todo: Ctrl+G subnet creation adds an input and an output node to the stream automatically.**
- **Todo: subnet inputs and outputs take custom parameters.** Parameters on an input or output are exposed outside the subnet. Outputs can also add custom parameters, and can add new stream nodes. Dragging a noodle onto a subnet adds a new input or output automatically.
- **Todo: Ctrl+U on a subnet ungroups it.** Add this as a keymap that expands the selected subnet back into its inner nodes.
- **Todo: point mode points can be half size.** Let the point display size go down to 0.5 in point mode.
- **Bug: a point is missing in point mode.** In the screenshot (2026-10-19), a point is expected at the inner corner where the three visible faces meet near the grid origin, but none is drawn. Check whether the corner vertex is being hidden by the object-mode point rule or by the picking/draw filter.
- **Todo: null draws a crosshair in the viewport (parity with Blender).** Add a size parameter for the crosshair.
- **Future todo: parent constraint.** Not scheduled yet.
- **Todo: adding a lattice from the viewport Tab menu auto-fits the lattice grid to the object.** The lattice grid should be set up to match the bounds of the object it is added to.
- **Todo: snap distance measurements to the nearest mm.** Round the measured distance to the nearest millimetre in the measurement readout.
- **Future todo: viewport gizmo for the Helix node.** Not scheduled yet.
- **Future todo: L-System should generate 3D output, not 2D.** Not scheduled yet.
- **Future todo: Metaballs / Points to Surface node needs a rework.** Not scheduled yet.
- **Bug: Controller node parameters are not displayed.** The parameter panel for controller1 shows only the header, the "Add section" button and the ch() hint. Its own parameters should appear below.
- **Todo: empty network shows an empty viewport, not a cook-failed error.** With no nodes in the network editor, do not show the cook failed error. The viewport should just be empty.
- **Todo: Switch node needs multiple input slots.**
- **Todo: new Switch-If node with parity to Houdini.** The Houdini reference (2026-10-19) has: Use Second Input If (All Conditions True or similar), Test Input selector, Number of Tests with +/- and Clear, and per-test rows with an enable checkbox, Test Type (Expression), and an Enable value slider.
- **Future todo: look at the LOD node.** Not scheduled yet.
- **Future todo: Cache node with parity to Houdini (2026-10-19 reference).** Controls: Cache Any Frame, Max Frames to Cache (default 2500), Clear Cache When Change Upstream, Clear Cache, Clear Current Frame Cache.
- **Todo: Extract Centroid parameters should be exactly: Group, Domain, Method, Output, Attribute Name, Transfer Attribute.** Rework the node's parameter list to these six, in this order.
- **Future todo: proper description tooltips when hovering shelf items.** Not scheduled yet.
- **Fixed (confirmed by user live test 2026-10-19, build 25d48825d):** transform with the output node unselected in object and point/edge/face modes; adding a node in a stream pushes the output node down.
- **Todo: a newly placed subnet should not show cook failed.** It should cook cleanly with its default contents.
- **Todo: double-click a subnet to enter it.** Inside, the default contents are an input node piped into an output node.
- **Bug: Boundary Select lists faces twice in its domain output.** Also, it does not appear to store the output group. Check the boundary select node package for both the duplicate domain entries and the missing output group.
- **Priority todo: group parameter fields get a dropdown and accept wildcards.** Every group parameter field should offer a dropdown listing the groups detected on its input. The field should also accept wildcard patterns: `*` (zero or more characters), `?` (exactly one character), `[abc]` (one listed character, ranges like `[0-9]` allowed), `^pattern` (exclude), and a space to union several patterns (for example `leg_* arm_*`). Example: `* ^debris_*` selects all groups except those starting with `debris_`.
  - The UI dropdown is in `ui/params.rs` (wait for Bob's split to merge first).
  - The wildcard matching in group parsing (`element_select.rs` `parse_group` and the core group handling) can be done separately.
- **Todo: Color node domain should offer Points, Edges and Faces.** The node package is the place to change it; no shared UI files.
- **Todo: Clip node needs a viewport gizmo to move the clip plane.** Viewport gizmo work, so it waits for Bob's split to merge (`viewport.rs` and `main.rs`).
- **Bug: selection shows through mesh (no occlusion or backface culling) when X-ray is off.** Selection should only ignore occlusion and show backfaces when X-ray mode is on. Otherwise selection should be occluded by the mesh as normal. Viewport and picking code (`viewport.rs`, `picking.rs`), so it waits for Bob's split to merge.
- **Todo: with two nodes selected, Tab > Merge should connect both into the new Merge node.** Currently only one is connected. Likely in the Tab menu add path (`add_node_at` in `main.rs`), so it waits for Bob's split to merge.
- **Bug: Polybridge fails with "no edge group group1" even though two edge-loop groups were made.** Screenshot 2026-10-19: two Group Create nodes feed polybridge1 (Source `group1`, Destination `group2`). Check whether the Group Create nodes make point groups instead of edge groups (the domain setting), and whether Polybridge reads the group on the right domain. Node package work, likely safe.
- **Todo: drop the selected object highlight in edit mode (Blender parity).** In point, edge and face mode, the object outline should not be drawn.
- **Todo: make the purple and green group highlights thicker.** These are the group colours in edit mode. Viewport drawing code (`viewport.rs`), so it waits for Bob's split to merge.
- **Priority: disable the hard-coded Fence node for now.** The user does not want a fixed fence node; it is not adaptable enough. Remove it from the registry (`nodes/mod.rs`, `mod fence` and `fence::FENCE`) and its metadata, and update the registry test list. Do this after Bob's architecture branch (`bob/arch-review`) merges, since that also edits `nodes/mod.rs`.
- **Priority: a curve node should show its curve with editable gizmos even when not visible.** Selecting any node that has a gizmo should draw its gizmo and curve, whether or not the node is set to visible. Rule: this applies to every gizmo-bearing node. Viewport code (`viewport.rs`, `main.rs`). Repro: `garden_path` (a curve) selected while the output is shown; the curve and its points should be visible and editable.
- **Low priority: gizmos (including the transform gizmo) should draw under the viewport menu UI.** Currently the gizmo draws over the display-mode dropdown and the menu bar.
- **Bug: the viewport grid glitches when zoomed far out.** At large zoom-out the grid lines break into stepped, aliased segments and flicker. Likely the grid line spacing or anti-aliasing in the viewport shader (`shader.wgsl`, `viewport.rs`). Screenshot 2026-10-19. Viewport code, so it waits until the current viewport priority work is merged.
- **Todo: rework Vertex Bevel to match Blender's bevel result.** The current result (screenshot 2026-10-19, a bevelled box corner) has uneven, fanned faces at the corners and a cap that is not a clean polygon. Look at Blender's vertex bevel behaviour and match its topology: a clean corner cap, even edge loops, and the same face count as Blender for the same input. Node package (`nodes/packages/vertex_bevel`), so it can run alongside the viewport work.
