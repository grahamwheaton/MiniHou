# Curve transform controls

Select an anchor or tangent by clicking it. The viewport Move, Rotate and Scale modes act on that selected part. Moving an anchor carries both tangents. Rotation and scaling pivot about the anchor: selecting the anchor transforms both tangents; selecting a tangent transforms that tangent, respecting mirrored/aligned modes. Auto/vector handles become Free when manually transformed.

Move has coloured axis tips and XY/YZ/ZX plane squares. Squares use a ray intersection with the actual oriented plane, not a screen-delta approximation. X/Y/Z locks an axis; Shift+X/Y/Z excludes that axis (plane movement or plane scaling). Repeating the constraint releases it. All use the viewport orientation frame. If a plane is exactly edge-on to the view, movement waits for a usable view rather than inventing a displacement.

Rotate rings measure angular movement in their oriented plane. An edge-on ring falls back to screen displacement. Unconstrained rotation uses the orientation Z axis. Scaling uses a positive exponential factor, so this gesture does not mirror handles. Shift precision is continuous when engaged/released during movement, rotation or scale; Ctrl temporarily inverts viewport snapping. Move uses the existing geometry/increment snapping settings; rotation and scaling use angular/scale increments.

Escape, right-click or pointer cancellation restores the original control data without saving. Release saves a single edit for Undo. Hover brightens handles; titles identify targets. A geometry snap outside a locked plane is projected back to that plane, preserving the constraint.

This is single-control editing, not Blender multi-point selection, trackball rotation, numeric transform entry or negative-scale mirroring. The browser regression uses small deterministic geometry and verifies actual SVG pointer/key interaction; it does not exercise a graphics driver stress load.
