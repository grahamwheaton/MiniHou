# Measurement, units and sketch updates

Measurements created with Dimension or Tape measure are saved in the graph, with graph undo/redo. Click their viewport text or choose a measurement under Sketch & Measure. Edit the label, endpoint XYZ values, or label pixel offsets; Pick endpoint reattaches it to visible geometry. Delete measurement removes only the selected annotation. Numeric endpoint edits intentionally detach that endpoint.

Vertex snaps retain point indices; edge snaps retain endpoint indices and interpolation weights; face snaps retain triangle point indices and barycentric weights. Node identity, connectivity digest and point count must still match. Coordinates follow transforms and deformation, including non-active and hidden nodes. Deleting/replacing the target or changing connectivity makes the attachment unavailable/detached instead of reassigning it. Undo restores valid references; node renames update references. Reduced preview geometry does not create attachments. Snap inference is perspective-correct and prefers frontmost equal-distance candidates.

During a session, unavailable attachments retain their last valid location. Graph saves triggered by a measurement edit include that last resolved fallback coordinate. Reloading a graph whose target is already unavailable uses the last committed fallback coordinate, visibly marked detached. There is no topology-remapping guess. Arbitrary point reordering that preserves exactly the same connectivity cannot be distinguished from deformation without persistent mesh element IDs. Geometry instance source vertices are not independently editable attachment targets.

User Preferences now offers Length display and Unit scale. Defaults are mm and scale 1: a model distance of 1 displays as 1000 mm. Changing either setting never resizes geometry. Bare values in converted length inputs use the selected display unit; measurement endpoint fields also accept explicit mm, cm, m, km, in and ft suffixes. Dimensionless values remain unchanged. The optional scale changes the physical interpretation of coordinates (scale .001 means one model unit displays as 1 mm).

This follows Blender's documented distinction between display units and internal coordinates:
https://docs.blender.org/manual/en/latest/scene_layout/scene/properties.html#units
https://docs.blender.org/manual/en/4.4/editors/3dview/toolbar/measure.html

While drawing a line or curve, right-click commits the valid current chain and stays in the tool, ready for a disconnected chain. A single pending point is discarded by right-click. Escape cancels only the pending chain and exits; previously committed chains remain available to Undo. Enter commits and exits. Draw Curve keeps creating Bezier chains after right-click.
# Following viewport edits

Viewport transforms carry snapped endpoints to the transformed output. When a drag
combines displayed objects, their verified point ranges are remapped into the merged
geometry. These changes are part of the same gesture transaction, including undo,
redo, cancellation and save/reload. Flip Faces preserves point correspondence too.
Other modelling operations keep attachments only when correspondence can be verified;
otherwise the endpoint is marked detached and can be picked again. Network edits do
not automatically retarget measurements to arbitrary branches.

