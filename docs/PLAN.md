# MiniHou plan and design notes

## Goal

A standalone procedural geometry engine (Python, numpy only for now) that live-links into Blender 5.1 on Windows.
Houdini-flavoured: attribute tables, a DAG of typed nodes stored as JSON, pull-based cooking with a cache, nodes
that are plain Python functions.

## Milestone 1 (this repo state)

1. Engine core: geometry model, node decorator/registry, cached evaluator, JSON graph load/save.
2. Nodes: grid, transform, noise, scatter, copy_to_points, merge, output.
3. Socket server + protocol that sends only changed attributes and skips unchanged topology.
4. Blender add-on: listener thread -> timer -> `foreach_set`, proxy object + Geometry Nodes modifier, parameter forwarding with
   feedback-loop guards.
5. Benchmark: 1M-point position array over the socket, timing the Blender-side write.
6. Unit tests for evaluator, caching, nodes (plus protocol, link, add-on).

## Key design decisions

**Geometry** (`minihou/geometry.py`). Points, vertices, primitives and detail; each attribute a numpy array
(float32 / int32 / bool; shape `(n,)` or `(n, k)`). Topology is `vertex_point` + CSR `prim_offsets`, kept separate from attributes
so "topology unchanged" is one comparison. Cooked geometry is **frozen** (arrays read-only). A node modifies a
shallow `copy()` and *replaces* arrays; untouched arrays remain the same object. That makes the cache safe to share
and lets the link layer detect "unchanged" by identity.

**Caching** (`minihou/evaluator.py`). Merkle keys: `hash(node type, node version, canonical params, input keys)`. Keys are
computed for the whole upstream set first, without cooking. Cooking then pulls from the target and stops at the first cache hit, so an
unchanged branch is never visited. LRU with a byte budget that counts shared arrays once; the live link
prunes entries that are no longer upstream of the display node after each cook. Contract: nodes are pure functions
of (inputs, params).

**Node SDK** (`minihou/sdk.py`). `@node("name", inputs=[...], params=[Float(...), ...])` on a function
`fn(*inputs, **params) -> Geometry`. Signature mismatches fail at import. Param kinds: float, int, bool, str, vec3, enum
with hard `min/max`, soft UI hints, labels. `multi=True` inputs receive a list (merge).

**Wrangle readiness.** The API is already sufficient for `@P.z += sin(@P.x)`: a node with one `Str("snippet", kind="code")`
parameter and one geometry input. The snippet is part of the cache key. The node compiles it to a numpy function (memoised on the
source text) and runs it over the attribute arrays. `tests/test_nodes.py::WrangleReadinessTests` exercises that shape with a
toy implementation. Left for the real thing: the expression parser/compiler, attribute reads/writes metadata on node types (for graph analysis
and UI), and an `Input`/`Output` port-type system beyond `geometry`.

**Live link** (`minihou/link/`). TCP on 127.0.0.1 (portable on Windows; no named pipes / AF_UNIX).
Frame = `MHL1 | header_len | blob_len | JSON header | 8-byte-aligned raw array blob`; the receiver reads into one bytearray and exposes arrays
via `np.frombuffer` (no copies). The `geo` message carries only changed arrays, a manifest of all current arrays (so removals are
explicit), counts, and the two topology arrays only when they changed. Diffing happens **per client at send time**
(against what that client last received) in a writer thread that sends the latest result only, so slider drags coalesce
correctly. Cooking runs on its own thread and also coalesces edits.

**Blender side** (`blender_addon/minihou_link/`). The socket thread only enqueues (merging pending `geo` updates). A
`bpy.app.timers` function drains the queue and does all bpy work; `mesh_writer.assert_main_thread()` guards the bpy entry points.
Output goes to a hidden `MiniHou_Output` mesh; `MiniHou_Proxy` has a Geometry Nodes modifier whose group reads the output
object with *Object Info* (relative transform). The group's inputs are the exposed parameters.

Feedback-loop guards: (1) the depsgraph handler only sets a flag; (2) forwarding is a diff against the values the engine is known
to have, so updates triggered by our own mesh writes forward nothing; (3) the engine never echoes parameter changes to the client that sent them; (4) engine-pushed
values are recorded as known before they are written to the modifier; (5) forwarding is rate limited and a circuit breaker trips at
>120 forwarded values per second; (6) the tick is not re-entrant. (2) has a regression test for the float32 round-trip
(Blender stores 0.4 as 0.4000000059604645; an exact comparison would forward a phantom edit after every connect).

## Findings from measurement

* `mesh.vertices.foreach_set("co")` is ~170x slower than writing the `position` attribute's data; `loops.foreach_set("vertex_index")`
  is ~250x slower than the `.corner_vert` attribute. Both fast paths are used, with fallbacks.
* At 1M points the Blender end costs ~1 ms and the socket ~6 ms; the engine's numpy noise (~276 ms) dominates a slider edit.
  A 100k-point edit is ~28 ms end to end.
* Review findings fixed after the first push: float32 round-trip phantom edits; `ensure_objects()` re-assigning the modifier node group on every update
  (Blender returns a new Python wrapper per access, so `is` checks are always False; it tagged the depsgraph each time); unmeasured
  performance claims in the README; the benchmark ignoring `MINIHOU_ENGINE_PYTHON`; restarting the engine on a busy port.

## Milestone 2 progress (done overnight)

* Numba backend for the heavy nodes, with numpy fallback, parity + golden-value tests.
* Wrangle node: sandboxed VEX-like snippets compiled to numpy (`minihou/wrangle.py`).
* New nodes: normal (Newell, numba), delete (expression blast with compaction), box, sphere, circle, fuse.
* CI on Linux + Windows, including the add-on in real Blender 5.1.2 for Windows (selftest, benchmark, GUI smoke test).
* Real-Blender GUI smoke test; fixes it found (greyed modifier inputs, interface rebuilds, depsgraph churn).
* Blender 5.2 support (modifier inputs moved from ID properties to `mod.properties.inputs`), found on a Windows desktop.

## Node editor + MCP (milestone 2)

One editing model, three front ends. `minihou/edit.py` defines JSON edit operations (add, connect, set, expose, layout...)
applied atomically to a copy of the graph; `Session.edit` commits a batch, records undo (slider drags merge into one step), bumps
the revision, pushes interface changes to Blender and notifies listeners. The engine serves an HTTP API with Server-Sent Events
(`minihou/web/server.py`, stdlib only) that the browser editor and the MCP server both use, so an edit from any of the editor, an
AI, Blender or the JSON file shows up in all the others.

* **Persistence**: the session saves edits back to the graph file (debounced 0.25 s, atomic replace, Windows-safe) and ignores
  its own writes when watching for external changes; unfinished graphs save and reopen; cooks validate only what the display
  node needs.
* **Editor**: vanilla JS served by the engine (no build, no CDN). Text from graph files is only ever inserted as text (a test
  feeds it HTML). Node DOM is rebuilt only when a node's content changes, so clicks never land on a node being replaced.
* **MCP**: newline-delimited JSON-RPC over stdio, protocol revisions 2024-11-05 to 2025-06-18, stdlib only, stdout reserved for
  protocol messages. Tool replies are written for a model: what was created, whether the cook worked, the failing node and
  message, nodes not feeding the display. `open_graph` can start an engine (its own console window on Windows).
* **Localhost hardening**: Host-header check (DNS rebinding), JSON-only POSTs (no simple cross-site forms), Origin check,
  file access limited to `.json` under the workspace root, connection closed after error replies.
* **Layout**: layered top-to-bottom layout with barycentre sweeps (fewer crossings, straight chains), applied automatically when
  most of a graph is new (an AI just built it) and on demand (`layout` op, **L** in the editor).

## Status and next steps

Done (milestones 1-3): the engine, live link, Blender add-on, node editor, MCP server, node libraries and systems, attribute
wrangle with native Blender data, and the Houdini feature sets below (groups, polygon modelling, attributes, curves,
instancing, time/solver, UVs and volumes, bevel, UV unwrap, fracture, polyreduce, path deform, exact booleans, PCG points).
Since then: atomic library reloads (`Registry.batch`), compiled kernels for mesh closest-point and ray queries, the editor
frame slider, and the `delaunay`, `convex_hull`, `dissolve` and `skin` nodes.

Worked through after that (all with tests; the bpy ones run with the `bpy` wheel as well as inside Blender):

1. **Topology rebuild cost** in Blender: measured instead of rewritten. For 1M quads Blender's own edge computation takes about
   120 ms (writing the arrays 56 ms); computing the edges with numpy takes 1.9 s, so engine-side edges would be slower, not
   faster. Left as it is.
2. **Wrangle** reads its extra inputs: `@opinput1_P` (same element number in input 1, zero beyond its end), `@near1_Cd`,
   `@near1_dist`, `@near1_ptnum` (nearest point of input 1). **Attribute flow** is worked out, not declared: `minihou/flow.py`
   compares a node's cooked output with its cooked inputs (added / modified / moved / removed, count and topology changes), so it is
   right for built-in, library and system nodes; `inspect` returns it as `flow` (plus a wrangle's reads and writes) and the editor's
   inspector lists it.
3. **Several outputs**: an `output` node with an `object` name that is not the display node becomes a mesh object of that
   name in Blender (`geo` messages carry `output`, an `outputs` message lists the live names, per-output diff state per
   client; the add-on only touches objects it marked). **Detail attributes** arrive as `mh_<name>` custom properties on the
   output objects.
4. **Editor**: copy/cut/paste of nodes as JSON through the clipboard, network boxes (`boxes` in the graph file, ops `box_add`
   / `box_set` / `box_remove`, cosmetic and outside the cache keys), and a WebGL2 3-D preview of the selected or display node
   (`POST /api/preview`, binary arrays, `minihou/preview.py` + `static/preview.js`).

5. **Python node** (`nodes/python_node.py`): real Python in the graph (inputs `geo`/`input1..3`, variables `a b c d i j v s flag`,
   named `Variables`, `Frame`/`Time`, you set `out`). Because a graph file can then carry code, it is **off unless the engine is started
   with `--allow-python`** (or `MINIHOU_PYTHON_NODES=1`); nothing in a graph can turn it on, the MCP instructions tell assistants not
   to ask, and `engine_status` reports the switch.

6. **Selection tools in the 3-D view**: `POST /api/preview` sends `tri_prim` (the primitive of each triangle); the viewer picks
   points, edges and faces on the CPU (nearest projected element with a ray test for visibility, box select, thick edge overlay) and the
   editor turns a selection into a node through the new `insert` op (new node fed by `after`, taking over its readers). `polybevel` got
   an `edges` selection (point pairs) and a check that a bevel does not end inside a surface. **Handles**: `minihou/handles.py` providers
   per node type (transform, polyextrude, polybevel, box, sphere) return arrows (origin, direction, parameter, scale, limits) with the
   preview; the viewer draws them as an SVG overlay and a drag becomes a `set` edit (vector parameters keep their other components).

7. **Dockable editor layout**: the 3-D view left the side column and is a pane of its own beside the graph (`#work`), with draggable dividers,
   Graph / Split / 3D presets, stacked or side by side, swap, a hideable side panel, Ctrl+Space maximise, all saved in the browser. The view
   got Front/Right/Top buttons, an axis gizmo (built once, so clicks survive redraws), orthographic (a very narrow lens far away, so picking and
   handles use the same maths), frame selection, Numpad keys, and framing that fits the narrower side of the pane.

8. **Packed app and add-on installer**: `minihou/blender_setup.py` (finding Blenders, the add-on zip, install through a one-off background Blender,
   launch with a private copy), `minihou/app.py` (workspace in `~/MiniHou`, welcome graph and examples copied once, busy ports fall back to free
   ones), the editor's Blender bar (draggable zip, Install add-on, Open Blender), `tools/build_app.py` (PyInstaller, numpy-only by default, with a
   smoke test that starts the built program) and `.github/workflows/app.yml` (Windows, Linux, macOS; zips attached to `v*` releases). The install
   script is tested for real with the `bpy` module; **not verified**: the Windows/macOS builds (only built on Linux here), and dropping the zip
   from a browser onto a Blender window (browser and OS dependent; the Install and Open buttons do not depend on it).

9. **Editor additions (requested)**: a normals overlay toggle, a play/step/loop timeline under the 3-D view (real-time frame skipping,
   `status.fps` reported by the engine), a breadcrumb path with double-click to open group (system) nodes, and a **Piece** selection mode (connected
   pieces, computed in the viewer by union-find over the triangles) with a move/rotate/scale gizmo on every selection whose first drag inserts a
   `transform` (group = the selected points, pivot = their centre) and carries the drag over to it; a handle's start may now return a promise.

10. **`fillet` and `blend`**: fillet rounds curve corners exactly (tangent arcs, radius limited to half of each neighbouring segment so arcs never overlap,
    attributes blended along the curve); blend morphs towards same-point-count targets with additive offsets, expression weights over `@Frame`/`@Time`
    (a small safe expression evaluator, no `eval`) and a per-point mask. Example graph `fillet_blend`.

11. **Output groups** (`minihou/outgroups.py`, `docs/output_groups.md`): a toggle + name per group as parameter pairs, an "Output Groups" section in the
    editor, prim/point/edge domains. PolyBevel's six from Houdini's panel, PolyExtrude front/back/side, and groups on mirror, boolean, clip, fuse,
    sweep, skin and the bridges; `polybevel` got a Group Type (points / edges / primitives) so one bevel's edge groups feed the next.

12. **From the shared Houdini panels**: Revolve (End Caps + End Caps Group, Start Angle), Subdivide (Creases field, crease weight override, Generate
    Resulting Creases group), PolyExpand2D (inside/outside loops, Keep Input, inside/outside groups, edgedist/edgespeed attributes), Group Create (the
    filter sections, edge groups, boundary groups).

13. **"C" pie menu with Houdini's modelling tools** (`web/static/pie.js`, wired in `editor.js`): eight categories (Adjust, Deform, Polygons, Group,
    Create, Build, Edges, Modify), each a sub-pie with a `»` page; hold C and flick, or tap C and click. A tool inserts its node after the node you
    are looking at, filled in from the 3-D selection. New nodes for the tools that had none (`minihou/nodes/modeling_tools.py`): `null`, `twist`,
    `linear_taper`, `squash_stretch`, `soft_peak`, `group_range`, `edge_collapse`, `edge_flip`, `edge_cusp`, `polysplit` (edge loops through quad
    strips, no T-junctions; cut a face between two points). The example library node `twist` became `twist_height`. Mapped onto existing nodes:
    Duplicate = `copy_transform`, Combine = `merge`, Extract = `blast` (keep selected), Mountain = `noise`, Group Expression = `group_create`,
    Point Weld = `fuse`, Dissolve = `dissolve_edges`, Edge Loop = `polysplit`. Not built: **Sculpt** and **Group Paint** (interactive brush tools;
    shown greyed out). The **Modify** page was not in the screenshots, so its tools are a guess (Blast, Fuse, Normal, Reverse, Divide, Subdivide,
    Resample, then attribute nodes): compare with Houdini. Houdini's pie tools start an interactive state; here a tool adds the node with sensible
    defaults and the parameters do the rest. Slot positions inside each sub-pie follow the order in the screenshots, clockwise from the top.

14. **RBD fracture toolbox** (Houdini's fracture menu; `nodes/fracture.py`, `nodes/fracture_tools.py`, `rbd.py`, `docs/RBD.md`): Voronoi Fracture got
    output groups (inside / outside / cut points / cut edges); new `voronoi_split`, `voronoi_fracture_points`, `voronoi_adjacency`,
    `boolean_fracture`, `convex_decomposition`, `assemble`, `glue_cluster` and `rbd_material_fracture`. The solver gained **compound
    (cluster) bodies** and a third `constraints` input: **breakable glue** (MuJoCo welds that snap past a strength, glued pieces do not
    collide until the last link breaks). Honest caveats: no packed primitives or string attributes exist, so Assemble writes an int
    `piece`, a vector `pivot` and the detail `piece_names`; group sets and the material glue strengths are my own tuning (no panels to
    copy); glue strength is a break force, not Houdini's constraint-network model (no hinges, springs, glue-to-ground rules or
    per-piece strength painting); `convex_decomposition` is a plane-cut search (reflex-edge planes), not V-HACD, and cuts along a
    face plane can leave sliver faces on curved meshes. MuJoCo is now installed in this environment, so the two RBD tests that used to fail run.

15. **Fracture parity with Houdini's docs** (from search summaries of the SideFX pages; I could not open the site): Voronoi Fracture got Create
    Inside Surfaces and Cluster Pieces (cells merged by the `cluster` point attribute that Voronoi Fracture Points now writes, with a
    k-means `merge_clusters`) and keeps inside faces from an earlier fracture; Voronoi Fracture Points got an impacts mode; RBD Material
    Fracture got Fracture Level, a Custom material, Convex Parts (proxy: compound bodies with `rbd_hull`) and Interior Detail; new
    `rbd_constraint_properties`. Behind them: `convex_decomposition` now repairs awkward cuts (coincident points welded, bow-tie loops
    split, candidate cuts that leave a sliver or non-flat face are rejected), and the cap code splits pinched loops. Not done:
    Houdini's secondary strength, constraints from rules (connection modes beyond touching faces), Edge Detail (boolean noise on
    the outside edges), packed proxy transfer for detailed pieces.

16. **Add Node menu and graph/library browser** (`minihou/menu.py`, `minihou/subgraph.py`, `web/static/nodemenu.js`, `editor.js`): the menu groups every node
    (one table in `menu.py`, also on `describe()` as `menu`/`label`/`keywords`), ranks a search over names, Houdini/Blender aliases and docs, folds groups,
    keeps favourites and recent nodes. The file menu became a browser (search, recent, folders, library nodes, Save As, duplicate/rename/delete, insert a
    graph's nodes) and **Ctrl+G** makes a library node from a selection (`/api/make_system`, one undoable batch). Not done: a selection that leads out through
    several nodes (a system has one output), exposing parameters automatically when saving, a thumbnail/preview of graphs, tags and a shared (non-workspace)
    library, renaming the open graph.

17. **Tool shelf** (`web/static/shelf.js`): a Houdini-style ribbon of tabs and icon tools (hand-drawn icons for the common tools, tinted initials for the rest) sharing the
    pie menu's tool definitions (selection prefill, presets). Drag-and-drop onto the graph; the Library tab follows the engine's library nodes; PCG lists every `pcg_*` node.
    Not done: user-made tabs and tools (drag a node into a custom tab), shelf tools that run a small graph recipe (Houdini's Quick Shapes, Spray Paint), and icons for every node.

Open:

- Output groups other than PolyBevel's are from memory of Houdini's nodes: compare with Houdini and adjust names/sets.
- Extra outputs have no modifier or exposed parameters (only the display branch does).
- More handles (bevel segments, copy_to_points scale). (Done: `@node(handles=[Handle(...)])` for any node, library nodes included, used by tube, helix, line and bend; transform Move / Rotate / Scale tools: rings
  turn about each Euler angle's real axis, scale cubes follow the rotated axes, drags multiply so scale never crosses zero.)
- Edge selections are point pairs, so they only stay valid while the geometry upstream keeps its point numbering (as in Houdini).

Not scheduled:

5. Investigate the Blender popover crash further (only reachable via `wm.call_panel(keep_open=True)`), or report it upstream.
6. Extension packaging (manifest + licence are the owner's call).
7. Booleans with exactly coplanar curved features (a sphere's equator in a box face): closed and exact in volume, but may leave
   tiny folded slivers.

## Node libraries (milestone 2)

`minihou/library.py` loads every `*.py` in the node folders (default `<workspace>/nodes`) and re-executes a file when its
contents change. Design points:

* **Cache safety**: `NodeType.code_hash` (hash of the file) is appended to the node's cache key, so editing a node's code can
  never serve a stale cached result, and nodes upstream of it stay cached.
* **Atomic per file, with rollback**: a file's node types are replaced as a unit; if executing the new version fails (syntax
  error, exception, a name clash) the previous version is restored and the error is kept with file and line.
* **No shadowing**: while a library file executes (`sdk._LOADING`), `@node` refuses names that already exist, so a library
  cannot silently replace `noise`.
* **Swaps between cooks**: the session refreshes libraries under the cook lock, bumps the revision and sends a `nodes` event
  (editor Tab menu updates live) before the graph event.
* **Graphs survive missing types**: unchecked loads keep nodes of unknown types verbatim; the cook reports the missing type on
  that node, and saving round-trips it, so a half-edited library never breaks a graph file.
* **AI**: no code-writing MCP tool. `node_library` returns the folder, loaded files, errors and an authoring guide; the
  assistant writes files with its own file tools, under its client's permission prompts.

## Attribute wrangle and native Blender data

* **Domains**: `wrangle` runs over points, primitives or vertices. Reads from other domains are promoted (point/prim values
  gathered to corners; per-face means with `bincount`). An attribute name lives on one domain, as in Blender: writing it moves
  it to the domain being run over.
* **Engine -> Blender** (`mesh_writer.write_native`): `uv` becomes the UV map *UVMap*, `Cd` the color attribute *Color* (set as
  active and render color), `N` custom normals through the `custom_normal` attribute (7 ms for 1M points against 2.1 s for
  `normals_split_custom_set_from_vertices`; the legacy call is the fallback), `material_index` and `sharp_face` face attributes.
  Literal uses of the reserved Blender names warn instead of clashing.
* **Blender -> engine** (`blender_object`): the engine sends `watch_objects` (the names its graph reads) after `welcome` and on
  every graph change. The add-on's depsgraph handler only flags watched objects; `tick` reads the evaluated mesh
  (`mesh_reader`, foreach_get on attributes; ~25 ms for 250k points) at most every 0.1 s per object and sends `object_geo`,
  or `object_missing` with the reason. A blake2b digest of the message skips unchanged data, which also stops a loop when a
  watched object depends on MiniHou's output. The engine stores each object's newest data in `minihou.inputs.INPUTS` with a
  fresh token that `blender_object` folds into its cache key (`NodeType.key_extra`), so new data recooks exactly the nodes
  downstream of it. Vertex groups have no bulk API (~0.8 us per point) and are skipped above 250k points.
* `blender_object` is refused inside systems: its data changes outside the graph, and a system's cache key cannot see it.

## Houdini feature sets (milestone 3)

Worked through in order of how often Houdini users reach for them:

1. **Groups** (`minihou/groups.py`): int attributes `group_<name>`; patterns like Houdini's group field; nodes call
   `groups.restrict()` to limit their changes. Groups reach Blender as booleans.
2. **Polygon modelling** (`nodes/model.py`, `nodes/subdivide.py`): vectorised over all primitives; attributes follow
   one rule set (`nodes/_build.py`: floats blend, ints/groups take a source or the minimum). Tests check invariants
   (closed shells, signed volume, conserved area), not just counts.
3. **Attribute toolkit and ray** (`nodes/attrib_tools.py`, `nodes/ray.py`, `minihou/spatial.py`, `minihou/raycast.py`):
   exact nearest-point and closest-point queries (SciPy's KD-tree when installed, else numpy grids with exact
   ring-search bounds), 3-D DDA ray walks; all tested against brute force. Numpy closest-point queries cost ~45 us
   each near a surface; a numba kernel now runs them when numba is installed.
4. **Curves**: `Topology.closed` (None = all polygons, so existing geometry and digests are unchanged); the link sends
   `topo/prim_closed` only for geometry with curves; Blender gets loose edges (`PolygonView` filters corner/face
   attributes to the polygons) and loose edges come back as curves. Deleting points splits curves into runs.
   Sweep uses double-reflection rotation-minimising frames (Wang et al. 2008).

5. **Instancing**: the engine writes `instance`/`rot`/`scale` point attributes and the detail `instance_objects`; the
   proxy's Geometry Nodes branch (Object Info as instance -> Geometry to Instance -> Instance on Points) is rebuilt only
   when the object list changes.
6. **Time, for-each, solver**: the frame is a context variable set per cook (`minihou/timectx.py`); any string
   parameter mentioning `@Frame`/`@Time`/`@TimeInc` puts the time in that node's cache key (systems and for_each
   bodies propagate it). The add-on sends `set_frame` from a `frame_change_post` handler (flag only, sent by the
   timer). `for_each` calls the body node per piece; `solver` memoises frames per input geometry (weakly) so the result
   for a frame never depends on the playback path.

7. **UVs and volumes**: `uv_project` writes vertex UVs (seams and poles fixed per face). `minihou/sdf.py`: dense SDF
   grids -- narrow-band exact distances from closed meshes (closest points) signed by flood-filling the outside
   (SciPy's binary_propagation when available, else iterative dilation), metaball fields from points, box blur, smooth
   min/max booleans, and surface nets meshing with face-saddle resolution so the result is edge-manifold.

8. **Bevel and shading**: `polybevel` splits the faces around each point into wedges at bevelled edges; each wedge's
   copy of the point moves `distance` from its bounding bevelled edges (bisector in one face, sliding along unbevelled
   or border edges across several), bevelled edges become quads and points with 3+ bevelled edges get corner
   polygons. `normal` domain `vertex` averages only faces within the cusp angle, so Blender (custom normals) shows
   crisp chamfers on smooth surfaces. `tube` and `torus` join the sources.
9. **UV unwrap** (`minihou/lscm.py`): islands across non-seam edges; non-disc islands (Euler characteristic != 1) are cut
   by a plane through their centre along whichever principal axis leaves the most discs, repeatedly. One sparse LSCM
   least-squares system for all islands (two pins each): SciPy's direct solver, else Jacobi-PCG on the assembled
   normal equations (a Numba loop when available, numpy otherwise) warm-started from planar projections. Islands are
   scaled to their 3-D area, turned to their smallest bounding box and shelf-packed.
10. **Fracture and clean-up**: `voronoi_fracture` clips the mesh by bisector planes nearest-first per cell and stops once
    a plane is beyond the piece's farthest corner (exact cells); only loops lying on the cutting plane are capped, so
    open inputs stay open. `scatter` mode volume seeds cells inside a closed mesh (`raycast.inside`: crossing parity
    along three skewed rays, majority vote). `smooth` uses Taubin's lambda|mu steps (no shrinkage); `match_size`,
    `exploded_view`.
11. **PolyReduce** (`nodes/reduce.py`): quadric error metrics with stiff border planes, collapsed in vectorised rounds:
    an edge goes when it is the cheapest at both its points (a matching), among the globally cheapest edges, passes
    the link condition and flips no triangle with the round's surviving moves (re-checked until stable). Refused edges
    wait while others go; the cheap set widens only when nothing else can collapse.
12. **Path deform**: the length axis maps to arc length on the curve; cross-sections use the sweep's rotation-minimising
    frames (start normal matched to `up`) or `up` projected off each tangent; past the ends the geometry runs straight.
13. **Booleans** (`minihou/boolean.py`): intersection points are named by (mesh, edge, pierced triangle), so every
    triangle touching one uses the same point and the result is watertight without welding by distance. Each cut
    triangle is split in barycentric coordinates by its chains of segments; closed loops are cut out with two
    opposite bridges (Eberly visibility; directions tried until neither meets a later hole), so pieces are simple
    polygons; pieces of a planar source polygon are merged back into n-gons. Regions between cuts get one ray-parity
    inside test each. B is grown by 1e-6 of the size along its normals and nudged generically, so flush faces cut
    cleanly; points within 3 epsilon are welded afterwards. Exactly coplanar curved features (a sphere's equator in
    a face) stay closed and exact in volume but can leave tiny folded slivers.
14. **PCG points** (`minihou/pcg.py`, `nodes/pcg.py`): points are oriented boxes (rot, scale, bounds) with density and
    seed. Overlaps are exact separating-axis tests on candidate pairs from a grid of bounding boxes (parallel axes
    are not separating, so boxes turned only about Z work); covered fractions are sampled. Self-pruning keeps the
    lexicographically first independent set by priority, computed in parallel rounds (proven equal to the sequential
    greedy pass in the tests). Random values come from hashing each point's seed, so they follow the points.
18. **Proxies for very large scenes** (`minihou/proxy.py`, `preview.py`, `Session.set_proxy`, details in `docs/PROXY.md`): the
    viewport and the Blender link reduce geometry by vertex clustering (grid size searched on a sample, checked on all points)
    instead of truncating; a cook proxy level scales marked parameters (count / res2 / res3 / step kinds), is part of the cache key
    through the scaled values, and auto mode cooks a small version first and the real one when edits pause (cancellable between
    nodes). Not yet exercised in a real Blender; the link budget is off by default.
19. **USD** (`minihou/usd_io.py`, `nodes/usd_nodes.py`, `Session.export_usd`, details in `docs/USD.md`): a geometry is split into Mesh /
    BasisCurves / Points under one Xform per output; attribute tables map to vertex / faceVarying / uniform primvars with the domain
    stored on each primvar, plus `point_index` / `prim_index` primvars that make MiniHou round trips exact. `usd_import` applies world
    transforms, up axis and units, flips winding for left-handed or mirrored prims, and keys its cache on the file's mtime and size.
    Not opened in Houdini / Blender / Omniverse here; no PointInstancer yet.
20. **Scene objects** (`nodes/scene_objects.py`, add-on `scene_plan.py` + `scene_objects.py`, `docs/SCENE_OBJECTS.md`): a sink node whose points
    become real Blender objects (linked duplicates). The engine sends a JSON manifest (`scene_objects`, with `scene_nodes` for which
    nodes exist) once per change per client, through the live link and in the modifier response (`instance_result.scene`). The
    add-on keeps them by custom-property marker, plans create / move / retarget / show / hide in a bpy-free module (unit-tested), parks
    surplus objects instead of deleting them, and only touches the scene from its timer. Verified against Blender 5.1 in the tests.
21. **Materials by name** (`minihou/materials.py`, `nodes/materials_node.py`, add-on `material_slots.py` + `mesh_writer.apply_material_names`,
    `usd_io.write_looks` / `_read_material_bindings`, `docs/MATERIALS.md`): `material_index` plus the detail list `materials` give face
    materials a meaning; `set_material` assigns by name; `merge` / `boolean` unify the lists; the link carries the list in the geometry
    header (and `materials` in `object_geo` / `instance_cook`); the add-on fills slots by name without ever removing one. USD export
    binds `Material` prims (preview colour from `Cd`) and import reads bindings; materials read from a file are referenced, not
    rewritten, so shading networks survive. Checked against Blender 5.1's own USD exporter and importer locally.
22. **Geometry and image exchange** (`blender_addon/minihou_link/curve_eval.py`, `geometry_parts.py`, `image_exchange.py`, `minihou/images.py`,
    `nodes/image_nodes.py`, `docs/EXCHANGE.md`): curves (sampled like Blender, compared against it in the tests), point clouds and mixed
    components come in through `blender_object`; Scene Objects modes curves/points make real Blender curves and point clouds; images are pixel
    grids (`blender_image`, `image_file`, `image_out`, PNG codec with 16-bit support) carried by new `image_data` messages and the
    `scene_objects` manifest transport.
23. **Attribute paint, quick assign and the spreadsheet** (`minihou/paint.py`, `nodes/paint_node.py`, `spreadsheet.py`, the `paint_assign` edit
    op, `web/static/paint-tool.js`, `sheet.js`, `docs/PAINT.md`): one node, `attrib_paint`, holds sparse painted values (points, primitives,
    edge groups); the viewer's Paint mode (depth raster + brush in the page, one edit per stroke), the Quick Assign panel and spreadsheet cells all
    write it, and the view colours by the attribute. Vertex (corner) painting is left out (the preview has no corner numbering); edges are edge groups.

Closest-point and ray queries on triangle grids have numba kernels (`nodes/_kernels.py` `grid_closest`, `grid_intersect`;
remesh of a 4,800-triangle sphere 17 s -> 3.5 s). Surfaces from points and curves: `delaunay` (SciPy or a pure-Python
Bowyer-Watson), `convex_hull` (Qhull or an incremental numpy hull), `dissolve` (flat faces to n-gons, reusing the boolean
node's outline merging) and `skin` (loft across curves).

## Systems (graphs as nodes)

A `*.json` graph in a node folder becomes a node type (`library._system_type`): `input` nodes (sorted by `index`) are its
inputs, exposed parameters become params (`dataclasses.replace` of the inner `Param` with the exposed name, label and the
graph's current value as default), the display node is the output. Running it copies the inner graph, applies the params and
cooks it with `Evaluator.cook(feeds=...)`: fed `input` nodes take the outer geometry and their upstream (the preview) is
not cooked. Fed geometry is keyed by identity (a weakly held token per immutable cooked Geometry), so the inner cache works
across outer cooks; all systems share one bounded inner cache under a re-entrant lock.

* **Hashes**: a system's `code_hash` covers its file and the hash/version of every node type used inside, so edits to a nested
  system or a Python node propagate. Any library change re-derives all systems (cheap), in dependency order.
* **Recursion**: a dependency cycle between system files is reported for each file in it; a runtime guard also refuses
  re-entering a system already being cooked on the thread.
* **Unconnected inputs** fall back to the `input` node's preview, which is also what makes a system file editable alone.

## Acceleration design (added after the benchmark)

Profiling at 1M points showed noise (275 ms), scatter (1.2 s: many numpy temporaries) and copy_to_points (570 ms: per-copy rotation matrices) as the
engine-side costs; the link and the Blender write were already ~8 ms. `minihou/accel.py` + `nodes/_kernels.py` add fused Numba kernels behind the
existing node functions. Rules: optional; backend chosen by availability only; same algorithm and float32 coordinate math as the numpy code (kernel
tests mutation-checked); launches serialised by a lock; any kernel exception disables acceleration and re-runs the numpy path; seeds reduced to 32 bits in
both paths. A thread-pool-over-numpy alternative gave only 2.8x and was not adopted.

## Real-Blender verification (added)

No Blender was installable from blender.org in Claude's sandbox; the official 5.1.2 Linux build was taken from the
`linuxserver/blender:5.1.2` image layer (digest-verified) and run headless and as a GUI on Xvfb with Mesa llvmpipe
(Blender's bundled Mesa crashed in its LLVM JIT on that CPU, the system Mesa worked). Notes for anyone repeating this:
`Region.active_panel_category` is read-only (the test clicks the sidebar tab with xdotool, using the panel-draw counter as
feedback), `bpy.ops.screen.screenshot` returns black on Xvfb (X11 capture is used instead), and the factory-startup splash
must be dismissed. `tools/gui_smoketest.py` holds the scenario; `launch_blender.py --gui-test` wires engine, display and report.

## Findings from a Windows desktop run (2026-10-11)

Machine: Windows 11, Python 3.14.4, numpy 2.4.4, numba 0.68.0; Blender 4.3, 5.0, 5.1.1 and 5.2.1 LTS installed.

* **Blender 5.2 broke the add-on (fixed in 41004fc).** 5.2 removed ID properties from modifiers, so `mod["Socket_N"]` raised
  `TypeError: ... id properties not supported` on every tick and no geometry arrived. Inputs now go through
  `proxy.get_input/set_input/has_input/input_owner/input_data_path`: `mod.properties.inputs.Socket_N.value` on 5.2+, the ID
  property before. Animation paths differ too (`modifiers["MiniHou"].properties.inputs.Socket_N.value` on 5.2).
  CI only runs 5.1.2: **add 5.2.x to the CI matrix** so this stays covered. *Done in 404ed7b: CI runs 5.1.2 and 5.2.1.*
* `launch_blender.py --find` picks the newest installed Blender (5.2 here), not 5.1, which the docs target. Both work now.
* At 404ed7b (editor + MCP), everything passes on 5.1.1 and 5.2.1: engine tests 317 OK (36 skipped: 26 bpy, 10 that look like
  the playwright browser tests; playwright is not installed), `--selftest` 41/41, `--gui-test` 18/18, consoles clean. An
  interactive session (editor + Blender 5.2, including switching graphs from the editor) logged no MiniHou errors.
* **Open bug: float32 noise is saved into graph files.** After a modifier edit in Blender the editor saved `demo.json` with
  `"amplitude": [0.2199999988079071, 0.0, 1.2000000476837158]` (the user set X to 0.22 and did not touch Z = 1.2). Values arriving
  from Blender are float32 and are stored and saved unrounded. Proposed fix: when a value comes from the Blender link (or when
  saving), snap each float to the shortest decimal that round-trips the same float32 (`float(repr(np.float32(v)))` -> 0.22),
  with a test. The forwarder's float32 tolerance already stops this from causing phantom edits. Saving also rewrites ints as floats
  (`10` -> `10.0`) and adds `pos` per node; those are expected but make diffs of hand-written examples noisy.
  *Fixed: values arriving over the link are snapped with `session.snap_float32` (only floats that are exactly a float32;
  editor and AI values stay exact); saved files write whole numbers as ints and keep short arrays (vectors, `pos`, input
  lists) on one line. Tests in `tests/test_edit.py::FileFormatTests`. Also fixed on the way: an explicit `move`/`pos` in a
  batch was overwritten by the automatic tidy layout.*
* **GUI test gaps on Windows:** dismissing the splash screen and clicking the MiniHou sidebar tab use xdotool (Linux only), so on
  Windows the sidebar panel is never drawn ("panel drew without errors (0 draws)" still passes); only the popover draw is
  checked. Could use `ctypes` `SetCursorPos`/`mouse_event` on Windows, or make the check fail when the sidebar is expected.
  *Fixed without OS input: the test reads the active sidebar tab (`Region.active_panel_category` is readable, not settable)
  and registers a copy of the panel there, so Blender's real sidebar draws the add-on's draw code on every OS; the reload
  checks now require real draws everywhere. (`--gui-test -- --no-clicks` runs that path on Linux.) Findings: the copy must be an
  independent `Panel` class reusing `draw`; registering a subclass of the add-on's registered panel makes
  `wm.call_panel` stop drawing the original, even after the subclass is unregistered. Windows CI then showed two more: the
  factory-startup splash stays open on Windows (Linux closed it with an X11 Escape) and blocks sidebar drawing, so the test
  sets `preferences.view.show_splash = False` (the splash is shown after `--python` scripts run); and
  `Region.active_panel_category` reads `"UNSUPPORTED"` until the sidebar has drawn once, so the test waits for a real
  category. Lesson: the experimental GUI jobs are `continue-on-error`, so a green run can hide them: check their job
  conclusions, not just the run's.*
* Several of the user's own add-ons fail on 5.2 (`imagepaste` hits the same ID-property removal; `smart_fill_1.5.0.py` has a syntax
  error; `sketchup_importer` has a circular import; `BL_add_to_collection` is missing). These are not MiniHou's, but they show up
  in the console and can be mistaken for its errors. *`launch_blender.py --factory` now starts a clean Blender; README notes
  that MiniHou's console lines start with `[minihou]`.*
* The machine has no git identity configured; commits from it were made as `Claude <noreply@anthropic.com>`, as on the branch before.
