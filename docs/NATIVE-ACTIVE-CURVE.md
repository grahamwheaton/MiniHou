# Native viewport routing

The active object is the last object picked in the viewport or selected in the network. Shift selection keeps earlier objects selected but makes the most recently picked object active. Removing the active object from the selection falls back to a remaining selected object. Modelling tools use the active object's displayed stream and its local point numbering; other displayed streams stay available for object selection.

Curve editing uses the active Bezier node. Its selected tangent takes precedence over the anchor and Move gizmo when their hit areas overlap. Pressing a tangent or gizmo starts a drag, release records one controls edit, and Escape cancels it. Shift click and box or lasso gestures extend the selected controls; G/R/S acts on that set. Delete removes the active control's point when more than two points remain. The native curve overlay displays the tangents of the active anchor.
