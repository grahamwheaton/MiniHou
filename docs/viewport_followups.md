# Viewport follow-up controls

- Mouse-wheel zoom moves toward the pointer. Over geometry it uses the surface hit; over empty space it uses the view-target plane. Perspective and orthographic views retain the pointer anchor. Fly mode keeps wheel speed adjustment.
- Changing between face, edge and point selection converts the selection. Faces convert to perimeter edges/points; conversion back to faces requires the complete boundary. View mode hides overlays while preserving the selection.
- Alt-click follows a unique ngon boundary, or a clearly continuing edge direction. Ambiguous branches stop rather than selecting unrelated wires.
- I starts interactive Inset: move away from the starting pointer position to increase the amount; move back to decrease it. Enter/left click confirms, Escape/right click cancels. Type a number for an exact amount. Mouse-driven modelling amounts use 0.001-unit increments (1 mm when one geometry unit is one metre). Geometric snapping remains exact.
- Viewport-created modelling nodes are placed below their source. Existing downstream nodes in that column move down in the same undo transaction; unrelated columns are preserved.
- Drop Null into a viewport with displayed geometry to insert a pass-through Null after that geometry. This is not a standalone world-space empty.
- Newly created GUI L-systems use a spatial branching grammar. Existing saved grammars remain unchanged: a grammar containing only yaw turns can still be planar. See examples/lsystem_3d.json for a 3D turtle example.
- Preferences includes a persistent user copy-allocation budget (16–16384 MiB, default 256), cook proxy level, automatic refinement and Blender triangle cap. This budget limits estimated realised-copy allocation, not total process memory. Graph files cannot raise it.
- Array helpers expose distance, relative bounds spacing and end-point controls plus handles for primary positions/dimensions. See docs/array_helpers.md and the array examples. These are practical controls, not complete Blender parity: combined constant+relative offsets, object-offset transforms, fit-to-curve, caps and seam merging are not implemented.

Claude commit 02935b0 is merged in this checkout. Source tests use small fixtures and isolated preferences; no huge realised-copy workload is used.
