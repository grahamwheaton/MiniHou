# Edge Slide — 17 October 2026

## Morning report

`edge_slide` is a Rust core node with `group`, signed `factor` (-1..1), `even`, `flipped`, `clamp` (on), `mirror_editing` (on), `correct_uvs` (on), and an optional output edge group. Empty Group leaves geometry unchanged. The viewport uses Shift+V in edge mode with selected edges; the existing G key starts Move, so Shift+V is the dedicated, conflict-free modal binding. LMB/Enter confirms an inserted node in one undo step; RMB/Esc cancels without one. Shift gives precision, Ctrl snaps tenths, E/F/C toggle modes, and typed input sets Factor. The wheel has no slide action.

The kernel walks selected edge chains in deterministic point order. Each adjacent polygon face provides the rail at its selected edge endpoints. Positive and negative factors choose opposite sides; Flip exchanges them. Boundary chains can move only toward their available rail. Clamp stops at the neighbouring edge. Even uses the first valid selected rail as the travel distance reference. Point float attributes interpolate from the original points along the same rails; integer attributes, groups and topology are preserved. With Correct UVs, float corner attributes interpolate only to a rail corner in the same face, preserving distinct UV island values across seams. The viewport preview calls the same kernel as the node.

Loop Cut's slide phase calls this rail kernel on the new cut-edge group. Its `polysplit` node keeps the older `position` parameter for saved graphs and Python oracle compatibility; Factor 0 leaves those results unchanged. The Loop Cut preview cooks the same node path during slide.

## Limits

This implementation handles simple edge chains, closed loops and boundaries. At poles or branching selections it chooses a deterministic outgoing edge; that choice may differ from Blender's interactive side selection. `mirror_editing` is exposed and defaults on, but has no effect without a symmetry context. The UV correction follows each face's local rail and does not infer an external unwrap. The UI binding implements edge slide only; Blender's separate vertex slide gesture is not present. The host app modal and visual output require an interactive smoke test beyond automated tests.

## Validation

Targeted Rust core tests cover edge rail sign, boundary, Flip, cube loop equivalence with `polysplit`, attributes, UV seams, deterministic cooking and multi-cut preview. The app's modal and keymap tests exercise the binding and phase transitions. `python tools/dump_rust_param_meta.py` and `python -m unittest tests.test_rust_param_meta` keep Parameters metadata in sync.
