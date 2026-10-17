# Loop Cut and Slide — 17 October 2026

## Morning report

The previous native modal already found visible quad rings and changed the cut count with the wheel, PageUp/PageDown and +/- keys. It differed from Blender after the first click: its slide value was a 0..1 split position, multiple cuts stayed fixed, RMB and Esc removed the pending cut, and Even, Flip, Clamp, numeric entry, Smoothness and Falloff were absent.

Ctrl+R now previews every cut at equal spacing while hovering. The first LMB starts a signed Factor (-1..1) slide; all new cuts move along their rails. The slide preview is cooked by the same `polysplit` path used on confirmation, including Even, Flipped and Clamp. Shift reduces pointer sensitivity, Ctrl snaps Factor to tenths, E/F/C toggle their options, and typing enters Factor. LMB, Enter or drag release confirms. RMB or Esc **during slide** creates the centred cuts with Factor 0 as one undoable node edit. RMB or Esc **during hover** exits without an edit.

The `polysplit` and `loop_cut` alias now expose Number of Cuts (`loops`), Smoothness, Falloff, Factor, Even, Flipped, Clamp and Correct UVs in Parameters. Editing a value re-cooks. The older `position` parameter remains for Python oracle and saved-graph compatibility; when Factor is nonzero the new slide starts from the centred cut. Smoothness applies a Laplacian displacement to interior cut vertices, with the chosen falloff shaping its strength. This is an approximation of Blender's surface smoothing, not an exact Blender kernel.

The quad ring walk stops at boundaries, non-quad faces and non-manifold edges; an edge shared with an adjacent ngon is subdivided to avoid a T junction. A single displayed, cooked mesh stream is required. The visible preview is not a permanent document change. Confirmation inserts one `polysplit` node, using the existing graph edit helper, and is one undo step. For multiple cuts, the cut lines stay distinct over the allowed Factor range.

## Validation and limits

Core tests cover quad and cube cuts, ring stopping, Python `polysplit` compatibility, multi-cut motion and cook-backed preview. Modal tests cover count, hover, slide, confirm, RMB/Esc phase behavior, toggles and numeric input. The native app must still be tried interactively for pointer focus, text events and visual behavior. Mirror editing has no symmetry context in this node. The remaining Blender differences are documented in [RUST_EDGE_SLIDE.md](RUST_EDGE_SLIDE.md).
