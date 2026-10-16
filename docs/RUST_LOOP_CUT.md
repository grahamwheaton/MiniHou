# Loop Cut: pre-rust behaviour and the Rust port

Sources read: `minihou/web/static/loop-tool.js` (the whole tool), `preview.js` (`pickEdge`, `quadStrip`, `edgeAt`),
`editor.js:1142,1883-1888` (wiring), `minihou/nodes/modeling_tools.py` (`polysplit`, `_walk`),
`minihou/edges.py` (`mask`), `tests/test_modeling_tools.py`, `tests/test_editor_e2e.py::test_loop_cut_modal_commit_and_cancel`,
`docs/VIEWPORT-TOOLS.md`, `docs/KEYMAP_AUDIT.md`. `minihou/nodes/loop.py` is the unrelated Loop Begin/End repeat block;
`INTERACTION-TOOLS.md`, `modeling_tools.md`, `modal-modeling.md` and `blender-interactions.md` don't describe loop cutting.

## The node: `polysplit` (Rust also registers it as `loop_cut`, same params and cook)

| Param | Default | Meaning |
| --- | --- | --- |
| `mode` | `edge_loop` | `edge_loop` inserts loops; `points` cuts faces between point pairs (`split_points`) |
| `split_edges` | `""` | Edges the loop crosses: ids/ranges, `pA-B` pairs, edge groups. Empty = no-op |
| `loops` | 1 (1..64) | Number of loops |
| `position` | 0.5 (0..1) | Where a **single** loop crosses each edge |
| `split_points` | `""` | `points` mode only: `a-b` pairs |
| `outgrp_points[_name]`, `outgrp_edges[_name]` | off, `splitpoints` / `splitedges` | Output groups for the new points / new edges |

What it cooks (`edge_loop`):
* For each selected edge with 1 or 2 faces, walk into each face. While the face is a quad, step to the opposite edge and
  continue into the next face. Stop at a non-quad, a boundary, a non-manifold edge, or back at the start (closed ring).
  Every edge the walk visits is a "ring edge". Its direction (`from -> to`) is set on the first visit, chaining from the
  seed so all ring edges run "parallel".
* The seed's direction is always **lower point id -> higher**. `edges.mask` sorts `pA-B` pairs, so `p4-1` == `p1-4`.
  The doc string's "direction p0 -> p1 decides" doesn't happen in Python, and Rust matches Python (tested).
* Fractions: `[position]` for one loop, otherwise `(k+1)/(loops+1)`, evenly spaced, with `position` ignored.
* New points are lerped from `from` to `to` at each fraction, appended after the old points, ordered by sorted ring edge.
* Each walked quad becomes `loops+1` quads in a strip, keeping the original winding. A non-quad face next to a cut edge
  gets the new points inserted into its outline, so there are no cracks or T-junctions.
* Vertex attributes are lerped, prim attributes copied from the source face, and edge groups are remapped.

## The pre-rust tool (`loop-tool.js`)

| Input | Hover stage | Slide stage |
| --- | --- | --- |
| Ctrl+R (viewport) | start (count = 1, position = 0.5) | |
| move | `edgeAt`: nearest edge within 14 px; among the 8 nearest, the first whose closest point is visible (depth test) | `position = clamp(0.5 + dx / max(100, width/2), .001, .999)` from the click X |
| Ctrl while moving | | snap position to 0.1 |
| wheel | count +-1, clamped 1..64 | same |
| LMB | on an edge: enter slide | commit |
| RMB | cancel (no node) | commit with position 0.5 |
| Enter | commit (position 0.5) | commit |
| Esc | cancel | cancel (no node) |

Preview: orange 3 px lines across each quad of the ring, at `position` for one cut and evenly spaced for several. The
status line shows the count and the stage hint. Commit inserts **one** `polysplit` node after the displayed node with
`{mode:'edge_loop', split_edges:'pA-B', loops, position}`. That's one undo step, and the camera is kept. There were no Shift
fine, even or flip options.

## Rust (`rust/crates/minihou-app/src/ui/mesh_cut.rs`)

Modal toolbar tool (`Tool::LoopCut`, Ctrl+R). It works on one displayed, successfully cooked stream.

* **Ring/preview**: uses `minihou_core::nodes::LoopTopology` / `LoopRing::cut_lines`, the same walk and fractions
  `polysplit` cooks with. The preview is therefore exactly the cooked edges (unit-tested).
* **Hover**: as `pickEdge`, 14 px radius, the 8 nearest candidates, the first visible one. Visibility ray-casts the
  displayed faces (`picking::pick_triangle`) along the camera ray through the edge point, so it works in perspective and
  ortho. X-ray skips the test. Only edges with a quad strip qualify; the old tool also accepted others and drew nothing.
* **Count**: wheel (or PageUp/PageDown or +/-) while Loop Cut is active, 1..64. The count shows beside the cursor
  and in the status chip. The wheel changes the count throughout the Loop Cut session.
* **Slide**: LMB on a previewed edge starts it at 0.5. The pointer motion is projected on the hovered edge's screen
  direction (lower id -> higher id) and divided by its projected length, so the loop follows the cursor. Shift is 10x
  finer and Ctrl snaps to 0.1. The factor is clamped to 0.001..0.999 and rounded to 1e-4. A seed seen end-on falls back to the old
  100 px horizontal mapping. The factor shows beside the cursor and in the chip. With several cuts, spacing stays even (node semantics).
* **Confirm**: LMB click, Enter, or releasing a press-drag (Blender style). RMB while sliding cancels.
  Enter while hovering commits centred.
* **Cancel**: Esc or RMB exits without inserting a node from either stage.
* **Edit path**: `UiAction::MeshCutCommit` -> `graph_edit::add_viewport_cut` -> `commit_document`. That's one undoable document
  edit inserting `polysplit` with `{mode, split_edges: "pA-B", loops, position}`. Editing those params later in the params
  panel re-cooks the same result.

Differences from pre-rust: the slide follows the edge rather than raw screen X, and Shift fine, PageUp/PageDown and drag-release
confirm are new. The node type is `polysplit` (Rust also retains the `loop_cut` cook alias).
Multi-display editing is not supported. Even/flip edge-slide modes don't exist in either build.
