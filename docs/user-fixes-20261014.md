# Eleven viewport and modelling fixes

Based on main `765ab48` (including Claude PR #8). This feature branch is intended for lead review and integration before the next release build.

| Request | Usage and resulting behavior |
| --- | --- |
| Opposite view | Hover the viewport and press Numpad 9. Front/back, left/right and top/bottom toggle consistently. Text fields keep keyboard focus. |
| Uniform inset | Use Inset on a supported planar face or connected region. Amount is perpendicular boundary distance, including rounded corners and hole boundaries. Zero amount retains existing topology. |
| Editable measurements | Create Dimension or Tape measure, then click its label or select it under Sketch & Measure. Edit endpoint coordinates, label and label pixel offsets; Pick endpoint reattaches an endpoint. Delete removes just that measurement. Graph undo/redo and save/reload preserve annotations. |
| Attached measurements | Vertex, edge and face snaps retain node identity, connectivity and interpolation weights. Measurements follow transforms/deformation, including hidden targets. Invalid topology or deleted/replaced targets visibly detach rather than choosing another element. |
| Length units | User Preferences → Length display defaults to mm; Unit scale defaults to 1. One model unit displays as 1000 mm. Geometry is never resized by preferences. Explicit length parameters, measurement editing, modelling feedback, snap increments and proportional radius use these units; counts, angles and scales do not. |
| Drawing chain break | Right-click commits a valid Line/Draw Curve chain and stays in the tool for another disconnected chain. A lone pending point is discarded. Escape cancels the pending chain and exits; Enter commits and exits. |
| Point primitive | Create → Point, or drag its shelf icon into the viewport. Edit Position or use its move handles. It produces one point and no edges/faces; `examples/graphs/point_target.json` demonstrates Copy to Points. |
| Knife robustness | Existing Knife workflow now handles degenerate projected ears and consecutive snapped segments without the reported NoneType division. Actual seven-segment beveled-mesh and multi-cut browser regressions cover this path. |
| Isolated display | View → Isolated dims other displayed objects and references. Clicking another object changes the active object. User Preferences → Isolated knockback controls the amount. It does not remove geometry or disable picking. |
| Select all | A in point/edge/face mode selects the matching components of the active mesh. In Object mode it selects all selectable displayed mesh objects. Invisible objects and nonselectable references are excluded. |
| Flip Faces | Select faces and press Alt+N, or use the Flip Faces menu action. Only selected faces reverse; corner attributes follow the winding, selection remains, and undo/redo works. |

## Boundaries

- Planar inset preserves topology and rejects collapsed/self-intersecting offsets and unsupported nonplanar connected regions. It is not a topology-changing polygon clipping solver. Existing UV values are preserved, not automatically reparameterized.
- Attachments do not guess correspondence after topology changes. During a session they retain the last valid location; after reload of an already-invalid target they use the last committed fallback, clearly marked detached. Point reordering with identical connectivity cannot be distinguished from deformation without persistent element IDs.
- Reduced proxy previews do not create precise attachments. Individual impostor instances are not independently selectable/editable attachment targets; existing mesh-based Object selection remains in use.
- Isolated primitive colors use an explicit primitive owner for every unwelded corner, so Morton/chunk triangle order does not affect ownership. Primary instance batches and their point LODs use the primary node identity for dimming; additional instance outputs and reference templates use their existing realized mesh/wire representation. Object-mode selection requires faces; point-only and open-curve objects use Point mode for component selection.
- Length conversion uses an explicit parameter allowlist. Unclassified/custom node parameters retain their existing raw semantics. Unit settings are display/interpretation preferences, not geometry conversion commands.
- Browser validation uses real headless Chromium interactions and rendered screenshots. It does not constitute a packaged Windows EXE or real Blender host smoke test.

See [measurement details and Blender documentation references](measurements-and-units.md) for attachment and unit semantics.
