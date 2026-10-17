# Rust viewport Knife — 17 October 2026

## Morning report

K draws cut points on the active object's visible mesh. The shared viewport inference engine supplies vertex, on-edge and on-face snaps; hold Ctrl to include edge midpoints, or Shift to pick the surface without a point snap. Its marker and tooltip show the inferred feature. A segment previews from the last point to the hovered point. Right-click or E stages the current chain and leaves Knife ready for another. Enter or Space inserts one `knife` node with every staged chain in one document action; Escape discards the pending operation. Backspace removes the last point. Middle mouse navigation remains available. C toggles 45-degree screen angle stepping; Z toggles hidden-face cutting. The tool acts on the active object's point range and inserts after that object's source.

The node stores `cuts` as JSON chains. Each point has a numeric world `position`, its view-projected `screen` position and optional inference `snap` descriptor; each chain has the projection matrix. Legacy JSON segments (`a`, `b`, `matrix`) still cook. `cut_through` and `snap_tolerance` are node parameters. The output contains `edge_group_knife` and `point_group_knife`. The existing perspective-correct projection kernel intersects the screen segment with every crossed projected face, including non-coplanar neighbours, and interpolates point and corner attributes. Parameters lists chains with a Clear action.

## Behaviour and limits

The native tool follows Blender's staged-chain confirm model and SketchUp's endpoint, midpoint and on-edge markers. The older browser Knife uses the same projected-cut kernel but has different modifier bindings. Cut points are captured as world positions plus snap references. Vertex, edge and face references follow changed point positions while the input point count remains the same; a topology change falls back to the captured screen position. Camera navigation during drawing reprojects the pending points to the new view. The preview is an overlay, not a live cooked topology preview. Screen angle stepping is 45 degrees; edge-axis and X/Y constraints are absent. The node exposes the stored snap tolerance, while the viewport uses the shared inference preference. As with the existing projection kernel, overlapping front/back faces depend on its depth and cut-through selection.

## Validation

Targeted verification: 18 viewport cut tests, 7 core knife tests (including non-coplanar faces and hidden-face cutting), 20 keymap tests, and 16 Python knife/parameter-metadata tests passed. The branch brief excludes a workspace test run. Interactive viewport behaviour still needs a host-app smoke test.
