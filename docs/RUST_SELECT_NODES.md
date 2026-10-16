# Selection nodes: grow, select_path, select_loop, select_ring

Rust-only nodes (no Python counterpart) that make Blender's edit-mode selection operators procedural.
Implementation: `rust/crates/minihou-core/src/nodes/select_nodes.rs`. The adjacency, loop/ring walks, paths and
grow/shrink steps are in `rust/crates/minihou-core/src/select_topo.rs`. The viewport (`minihou-app/src/element_select.rs`)
uses the same code, so a node gives the same result as the matching viewport gesture on the same mesh.

All four nodes pass the geometry through unchanged and only write groups. They are listed under **Groups** in the
Tab menu and on the shelf.

## Conventions

* `domain`: `points`, `edges` or `primitives`, matching Blender's vertex / edge / face select modes. Point and
  primitive groups are `group_<name>` attributes. Edge groups are `edge_group_<name>` vertex attributes, the same
  ones `edge_group` writes, so `blast`, `edge_group_promote` and the others can read them.
* Element patterns work like the existing group nodes. Points and primitives take numbers, ranges, group names,
  wildcards and `^` exclusions. Edges take edge ids, `p<a>-<b>` point pairs, edge group names and wildcards.
* Edge ids are numbered as in `topo::edges`, sorted by (lower point, higher point).
* Polygons are the closed primitives with at least 3 points. Open curves add edges but are not faces.
* `combine` is `replace` (default) or `union`. `union` adds to an existing group with the same name.

## grow (Select More / Less)

| Param | Default | Meaning |
|---|---|---|
| `group` | `""` | Selection to grow or shrink. Empty means everything. |
| `domain` | `primitives` | Element type. |
| `mode` | `grow` | `grow` = Select More, `shrink` = Select Less. |
| `steps` | `1` | Number of steps, 0..100000. 0 leaves the selection unchanged. Stops early once nothing changes. |
| `face_step` | `true` | Blender's *Face Step*: step through polygons that share a point. With it off, neighbours share an edge (points, faces) or a point (edges). |
| `keep_selection` | `true` | Write the whole result to `result_group_name`. |
| `result_group_name` | `""` | Empty uses the input group's name when `group` is a plain name, otherwise `selection`. |
| `output_group` | `false` | Also write only the change: the elements growing added, or the elements shrinking removed. |
| `output_group_name` | `grown` | Group name for the change. |

Neighbours with `face_step` on:

* points: the points of every polygon touching the point.
* edges: the edges at either end of the edge, plus the edges of polygons touching it.
* faces: polygons that share a point.

Shrink is Blender's Select Less: an element stays selected only if all of its neighbours are selected. In practice:

* An open border does not erode by itself, so a fully selected open grid stays full.
* Isolated elements (no neighbours) keep their state.
* On non-manifold edges every polygon on the edge counts as a neighbour.

Adjacency is built once per cook, and each step is O(elements + adjacency).

## select_path (Select Shortest Path)

| Param | Default | Meaning |
|---|---|---|
| `domain` | `points` | Points share an edge, edges share a point, polygons share an edge. |
| `start`, `end` | `0`, `1` | Endpoint numbers, used when `waypoints` is empty. |
| `waypoints` | `""` | Ordered chain of elements to route through, for example `0 17 42` or, for edges, `p0-1 p7-8`. Plain ids keep their order. Group names add their members in element order. |
| `method` | `shortest_distance` | `shortest_distance`: Dijkstra by the distance between element centres (Blender's default). `fewest_steps`: fewest connections (Blender's topological distance). |
| `closed` | `false` | Also route from the last waypoint back to the first. |
| `group_name` | `path` | Output group. |
| `combine` | `replace` | `union` adds the path to an existing group. |

Errors:

* An element number out of range: `there is no point 99 (the input has 25)`.
* Two waypoints that are not connected: `point 0 and 3 are not connected`.

## select_loop and select_ring

| Param | Default | Meaning |
|---|---|---|
| `seeds` | `""` | Seed edges. Empty writes an empty group. |
| `domain` | `edges` | Output elements: the walk's edges, their points, or primitives. Primitives give the face loop, which is the polygons of the ring through the seed (the same as Alt+click in face mode). |
| `mode` (loop only) | `edge_loop` | `edge_loop` follows edges straight through valence-4 points and also follows n-gon cap perimeters and straight wires. It stops at poles (other valences), borders and non-manifold edges, or closes into a ring. `boundary_loop` follows the open border from a boundary seed, stops where the border branches, and skips interior seeds. |
| `direction` | `both` | `forward` / `backward` start only towards the seed's higher / lower point (for rings, its second / first face). A closed loop is still found whole. |
| `group_name` | `loop` / `ring` | Output group. |
| `combine` | `replace` | `union` adds to an existing group. |

`select_ring` crosses quads to the opposite edge. It stops at triangles, n-gons, borders and non-manifold edges, and
wraps around closed strips (a cube ring has 4 edges).

## Viewport rules

* Selection gestures change the viewport selection only. They never create these nodes:
  * Alt+click selects a loop.
  * Ctrl+Alt+click selects a ring.
  * Ctrl+click selects a path. The viewport keeps its "most direct" path heuristic.
  * Ctrl+Numpad +/- (Ctrl+Alt+Plus/Minus) selects more / less, with face step on.
* A node exists only when you choose it from the Tab menu, in the viewport or the network editor. Tab in the viewport
  opens that menu in every selection mode. Ctrl+Tab toggles object / edit mode.
* A node added while elements are selected starts from that selection. It is wired like any menu-added node (after
  the selected node, before the output). Its parameters are filled in as follows:
  * `grow`: `group` = the selected elements, `domain` = the selection mode.
  * `select_path`: `waypoints` = the first two selected elements.
  * `select_loop` / `select_ring`: `seeds` = the selected edges. Point and face selections are converted to edges.

Viewport ids are the displayed geometry's ids, as with Extrude and Bevel. With several displayed objects, the ids
of the second and later objects are offset.
