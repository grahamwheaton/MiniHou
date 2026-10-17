# QUEUE (ordered, highest priority first)

1. **Viewport select arrow on group and element fields** (purple highlight). Shared widget, see below.
2. **Advanced Flood Fill** (`flood_fill`).
3. **Full Array parity** (shapes, count methods, gizmos).
4. **Image-based nodes** (Image Density + Scatter option first).
5. **Controller node** (section-grouped driver panel, optional Blender exposure).
6. **Low priority:** L-system is 2D (match the Python fix).

---

# TODO: Image-based procedural nodes (parked)

Status: parked by the user (2026-10-17). Not started. Build order when resumed: Image Density + Scatter density option, Image Sample, Image Mask, Image Displace, Image to Curves.

---

Work only in a git worktree: git worktree add -b opus/image-nodes ..\opus-image HEAD, then work in that worktree. Rust core (rust/crates/minihou-core) and native app (rust/crates/minihou-app). Read only what you need; print short output. Set CARGO_BUILD_JOBS=2 and CARGO_TARGET_DIR=rust/target_image. Run only targeted tests; no workspace tests, no release build, no push. Commit in small commits ending with: Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>. Keep the keymap table test green; regenerate param meta when params change (python tools/dump_rust_param_meta.py; every param needs a doc string). Reuse the existing image_file node and the Blender image path; do not duplicate image decoding. Do not change unrelated engine code.

BUILD ORDER: (a) Image Density, plus a Scatter density option; (b) Image Sample; (c) Image Mask; (d) Image Displace; (e) Image to Curves. Stop after any step if a blocker needs the user; report it.

(a) IMAGE DENSITY: new node `image_density`. Input: geometry with a colour attribute Cd (as produced by image_file). Params: channel (luminance, red, green, blue, alpha), invert (default true: dark = high density), contrast (1.0), brightness/offset (0.0), threshold (optional hard cutoff, 0 = off), output attribute name (default `density`). Output: the same geometry plus a per-point float `density` in 0..1. Luminance uses Rec.709 weights. Handle greyscale images (R=G=B), missing channels and invalid values (NaN -> 0, clamp). Preserve all attributes. SCATTER DENSITY OPTION: extend the existing scatter node (find it in nodes/) with an optional `density_attribute` param (empty = off). When set, scatter weights point placement by the interpolated density across each surface; with constant density the result must match the unweighted scatter exactly (no bias). Tests: luminance of black and white, invert, contrast, threshold, channel selection, missing Cd, NaN, preservation of other attributes; scatter with constant density equals unweighted; scatter with dark-half density puts more points in the dark half (count check with a seed). Demonstration: add a test graph JSON under rust/crates/minihou-app/src/test_data (image_file -> image_density -> scatter -> copy_to_points) and a test that cooks it.

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

# Queued feature: Controller node

A "Controller" node that acts as a control panel for top-level driver parameters.

- **Add parameters of any type** (float, int, bool, vector, colour, string, menu, and so on) from the node's own panel.
- **Drag to rearrange** parameters, and **group them into sections** that can be named and reordered.
- **Purpose:** one place to hold the top-level drivers that other nodes reference, so a whole network can be tuned from one panel.
- **Link to existing work:** it should reuse the Add Input machinery (dynamic typed inputs) and the value-linking references (`ch(...)`), so other nodes can link to its parameters.
- **Sections are saved in the graph file** (decided by the user).
- **Parameters can optionally be exposed to Blender** (decided by the user): each Controller parameter has an 'Expose to Blender' flag. Exposed parameters appear in the Blender modifier panel, inside a section that matches the Controller's section, and reuse the Add Input socket machinery. Unexposed parameters stay inside MiniHou.

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
