# Rust viewport element selection: loops, rings, paths, mode conversion

Behaviour spec for the native (egui/wgpu) editor, ported from the pre-rust browser
viewport. Implementation: `rust/crates/minihou-app/src/element_select.rs` (pure logic,
unit tested) wired into `main.rs` (click, hover, mode switch, region select).

## Sources in the pre-rust build

| Behaviour | Reference |
|---|---|
| Click modifiers | `minihou/web/static/preview.js` `beginCanvasDrag` / `endDrag` (`loop: altKey, ring: ctrlKey, path: ctrl && !alt && !shift`) |
| Loop / ring walk | `preview.js` `quadStrip(edge, ring)` |
| Path | `preview.js` `pathPick`, `selection-tools.js` `MiniSelectionMath.directPath`; `docs/viewport-selection.md`, `docs/blender-interactions.md` |
| Mode conversion | `preview.js` `api.setMode`; `docs/viewport_followups.md` |
| Regression tests | `tests/test_selection_flow_browser.py` |

The Python helpers named in the task (`edges.py`, `loops.py`, `paths.py`) do not exist; all
of the logic lived in the browser viewport.

## Topology model

The displayed scene holds fan triangles and the polygon edge list (`preview::layout`). A
**polygon** is rebuilt by joining triangles that share an edge that is *not* in the polygon
edge list (a fan diagonal). An edge is a **real edge** when it is in that list. Open
polylines contribute real edges with no faces. Face selection, face loops and face paths
work per polygon; `Selection.tris` still stores triangle numbers, always whole polygons.

## Click mapping (Select tool, Points / Edges / Faces modes)

| Input | Action | Origin |
|---|---|---|
| LMB | pick one element (replace) | existing |
| Shift+LMB | toggle element (polygon-wide in face mode) | existing |
| Alt+LMB | loop through the nearest edge of the clicked polygon (replace) | pre-rust |
| Shift+Alt+LMB | loop, added | pre-rust |
| Ctrl+Alt+LMB | ring (replace) | pre-rust |
| Shift+Ctrl+Alt+LMB | ring, added | Blender (pre-rust replaced) |
| Ctrl+LMB | path from the active (last selected) element to the clicked one, added | pre-rust |
| Ctrl+Shift+LMB | subtract the clicked element | pre-rust |

* Alt+LMB is only a selection gesture with **Blender** navigation. With Houdini navigation
  Alt+LMB orbits and an Alt click selects nothing (pre-rust `selecting` was false).
* Ctrl+LMB drag remains a subtracting box; Ctrl+RMB remains lasso. Only a click (no drag)
  runs the path.
* Ctrl+click with nothing selected selects the clicked element (Blender). Pre-rust did a
  no-op subtract; that was an accident of the modifier mapping.
* Holding Alt (Blender navigation) previews the loop (Ctrl+Alt: ring) under the pointer.
* There is no selection undo in the Rust app, so none is recorded.

## Loop walk (`quadStrip(edge, false)`)

Breadth-first from the picked edge. An edge with more than two faces is kept but not
expanded. At each end vertex `v`, candidates are the other edges at `v` with at most two
faces. Then, in order:

1. `v` has exactly four edges: take the single candidate that shares no face with the
   current edge (regular quad flow).
2. The current edge borders an n-gon (more than four vertices): take the single candidate
   lying on that n-gon (cap perimeters).
3. Otherwise continue only in a clearly forward direction: best cosine between the incoming
   direction and the candidate > 0.5 and at least 0.15 better than the runner-up.

Consequences: poles (valence 3 or 5 between quads) stop unless one edge is clearly straight
ahead; open-grid borders are followed (valence-3 border vertices continue straight) and
stop at corners; ambiguous wire branches stop. A cube edge loop is the single edge.

## Ring walk (`quadStrip(edge, true)`)

Breadth-first: for each face (at most two) of the edge, if the polygon has exactly four
vertices take its edge sharing no vertex with the current edge. Triangles and n-gons stop
the ring. A cube ring is four edges.

## Element modes for loops and rings

* Edges: the loop/ring edges.
* Points: the points of those edges.
* Faces: Alt and Ctrl+Alt both select the **face loop**: the quads crossed by the ring of
  the picked edge (Blender face loop; equals the pre-rust Ctrl+Alt result). Pre-rust Alt in
  face mode selected the two face strips either side of an edge loop; not kept.

## Path (Ctrl+click)

Start is the active element: the last entry of the current selection (in face mode the
polygon of the last triangle). Elements of the route are added; the target becomes active,
so repeated Ctrl+clicks chain. Disconnected endpoints add nothing.

* Points: adjacency over real edges; position = point.
* Edges: edges sharing a vertex; position = midpoint.
* Faces: polygons sharing a real edge; position = vertex average.

Default method is **most direct** (pre-rust default `interaction.pathMode='direct'`):
Dijkstra over (previous, current) states, step cost
`max(L/scale, 1e-9) * (1 + 8 * mean chord distance of both ends) + 0.75 * (1 - cos turn)`,
`scale` = start-to-target distance, chord distance normalised by `scale`, the first step's
turn measured against the chord. The **shortest** (fewest-hops BFS) variant is implemented
as `Method::Shortest` but has no UI switch yet.

## Mode conversion (switching 1/2/3)

Applied to the selection held in the old mode (pre-rust `setMode`):

| From \ To | Points | Edges | Faces |
|---|---|---|---|
| Points | - | real edges with both ends selected | polygons with every vertex selected |
| Edges | edge end points | - | polygons whose real edges are all selected |
| Faces | polygon vertices | real edges with both ends in one selected polygon | - |

Going up requires complete containment, so a single point or edge never selects a face.
Switching to Object mode stashes the element selection and hides it; entering an element
mode again converts the stash from the mode it was made in (pre-rust `lastMode`).

## Approximations

* Edge picking for loops/rings uses the clicked polygon: the real edge of the hit polygon
  nearest the 3D hit point. Wire-only edges (no faces) cannot be clicked, so wire loops
  are reachable only from a polygon edge.
* Plain Edge-mode clicks and box/lasso/circle edge selection now use real edges only (no
  fan diagonals); face region selection expands hits to whole polygons.
