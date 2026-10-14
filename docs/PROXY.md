# Proxies for very large scenes

Two separate things keep a big scene usable. Both are on by default or one click away, and neither changes what a graph
means at full size.

## 1. The viewport always draws within a budget (display proxy)

`minihou/proxy.py` `decimate()` reduces any geometry to a budget of triangles and points. The 3-D view asks for
4,000,000 triangles / 4,000,000 points (`preview.DISPLAY_TRIS`, `DISPLAY_POINTS`); "Full res" in the view raises that to
16,000,000 / 16,000,000 (`MAX_*`); past those the same reduction applies, so a request can never hang the browser. The Perf
menu's **Display budget** lowers the default for a weaker GPU (100k to 16M). Instances still turn into impostors from 250,000
triangles (`preview.INSTANCE_TRIS`), as before.

* **Meshes**: vertex clustering. Points that fall in the same cell of a 3-D grid merge into their mean; the cell size is
  searched (on a sample of the points, then checked on all of them) so the triangles that survive are just under the budget.
  The whole shape stays visible. The old preview cut the triangle list off after the first N triangles, which showed
  part of a mesh and nothing of the rest.
* **Curves**: every k-th curve. **Loose points**: an even sample.
* **Attributes** follow: float point attributes average over a cluster (`N` is renormalised), everything else (ints, groups)
  takes the first member; primitive and vertex attributes follow the triangle that survived.
* The preview header says what happened: `proxy: {active, points, tris, shown_points, shown_tris}`; `point_src` and
  `tri_prim` map what is drawn back to original point and primitive numbers.
* The view shows "Reduced view: 4,000,000 of 31,996,002 triangles" with a **Full res** button. Selecting needs the real points, so
  the **Detail** setting in the viewport's display options decides what happens (it is remembered): **Auto** (default) draws
  big geometry reduced but loads the full detail the moment you turn a selection mode on (1-4 or the Pts/Edges/Faces buttons)
  and goes back to the reduced view when you leave it; **Full** always asks for everything the viewer can hold; **Fast** always
  reduces and keeps selection off. A geometry over the cap even at full detail (`preview.MAX_TRIS` / `MAX_POINTS`) cannot be
  selected on: the view says how big it is and the selection modes stay off until you cut it down (Blast, Group Create with
  a bounds box). Nothing re-reduces the view behind your back: a node inserted from a selection keeps the full view.
* The **cook proxy** (Full / Auto / 50% ... 1%, which scales node parameters) is also in the viewport's display options, next
  to Detail, and in the status bar's Proxy menu. It never changes how a USD file is read: only parameters marked as
  proxy-able scale.
* Measured (Linux sandbox, preview request incl. reduction, quad grid): 1M quads 1.6 s and 7.6 MB on the wire (was 5.1 s,
  31 MB, truncated); 4M quads 6.3 s and 8.4 MB (was truncated at 48 MB). Cooking those grids takes 0.2-0.4 s, so the
  preview is the slow part. Results are remembered per cooked geometry, so a second request for the same node is instant.
  Meshes with more than 100,000 primitives are fan-triangulated instead of ear-clipped (a concave polygon may show the
  wrong diagonal in that case; nothing else is affected).

`POST /api/preview` takes `full`, `tris` (a custom triangle budget) and `have` (the `topology` token of what the viewer holds).

### Keeping big views fast

* **Edges and drawing order** are built by linear-time numba kernels (`topo.edges`, `preview.chunk_triangles`, fan
  triangulation): the same edge numbering as the numpy path, which stays as the fallback. `bench/bench_viewport.py` measures it.
* **Positions only.** Triangles, their drawing order and edges depend only on the topology, and a deformer (noise, transform,
  smooth, mountain...) keeps the topology of its input. The preview remembers them per topology (`topology` in the header: its
  digest and point count) and only redoes the run bounds from the new positions. The viewer sends back the token it holds as
  `have`; when it matches, the reply has `same_topology: true` and no `tri` / `tri_prim` / `edge`, and the viewer swaps the
  positions into its GPU buffer. A reduced view never does this (clustering depends on positions).
* Measured in this sandbox (4 cores, no GPU; quad grid with noise, full resolution):

  | | new topology | after a deformer |
  |---|---|---|
  | 1M quads (2M triangles) | 1.1 s, 60 MB | 0.13 s, 12 MB |
  | 4M quads (8M triangles) | 4.7 s, 240 MB | 0.5 s, 48 MB |

  Before, 4M quads took 8.5 s and were reduced to 2.4M triangles (87 MB) for every edit. Drawing speed depends on the GPU:
  the Perf menu's statistics show triangles drawn and frame time.
* Colouring by a primitive attribute (paint) unwelds the triangles, so the view refuses it past 4M triangles; points and edges still work.

## 2. Cook a lighter version (cook proxy)

A **proxy level** between 0.001 and 1 (1 = full). Node parameters that set the amount of work are marked in the node
definition with `proxy="..."` and scale with the level:

| kind | scaled by | used for |
|---|---|---|
| `count` | level | scatter `count`, `points_per_m2`, circle / line / helix points, revolve divisions, fracture points `count` |
| `res2` | sqrt(level) | grid columns and rows, sphere / tube / torus bands, box divisions (work grows with the square) |
| `res3` | cbrt(level) | (for 3-D resolutions) |
| `step1` / `step2` / `step3` | size / level, / sqrt, / cbrt | resample length, PCG spline spacing, remesh / points-to-surface / SDF voxel size |

A proxy never asks for more than the real value and respects each parameter's minimum. Mark your own library node's
parameters the same way: `Int("count", 1000, proxy="count")`. Exposed parameters of a system are not scaled at the
outside (the nodes inside scale themselves, so nothing is scaled twice).

How it stays correct:

* The cache key is computed from the **scaled** values, so a 10% cook never serves a full result or the other way round,
  and switching back to Full finds the full results still cached.
* Nodes that depend on nothing scaled keep one key at every level, so they are cooked once.
* Systems, `for_each` and `solver` also put the level in their key (they cook a sub-graph at run time).
* The level is a context variable like the frame, so loops, systems and for-each see it.

### Choosing a level

*Proxy* menu in the top bar (or `POST /api/proxy {level, auto, link_tris}`, or the MCP tool `set_proxy`):

* **Full**: the real thing.
* **50% ... 1%**: always cook that level. The status reads "proxy 10%" and everything (inspect, preview, Blender) shows it.
* **Auto**: if the last full cook was slower than 800 ms (or is not known yet), a change is cooked at 10% and published
  immediately, then, once edits pause for 0.3 s, at full size. Typing or dragging a slider never waits for a full cook:
  a newer edit cancels the running full cook between nodes (the nodes already cooked stay cached). If nothing in the
  graph scales with the level, auto skips the extra pass. `wait_idle()` (tests, AI tools) returns after the full result.

### Blender

Whatever the session cooked is what Blender receives, so a proxy level also lightens Blender. Separately, the
**Send Blender at most** list in the Proxy menu (`link_tris`) caps what the link sends: bigger geometry is reduced by the
same clustering (as triangles, with averaged attributes). The cooked geometry itself is untouched, and it is off by default.
Not yet verified in a real Blender (no Blender in the build sandbox for this change): the add-on receives ordinary geometry
messages, so nothing on its side changed.

## 3. Instances as impostors (display only)

A hundred thousand trees are a hundred thousand copies of a few shapes. Clustering the whole scene (part 1) turns every tree into the same
blob, and sending every copy is too much for the browser. `minihou/impostors.py` finds the instances and the viewer draws them like Houdini's
packed primitives in the viewport: each **shape once, at three levels of detail**, plus **one transform per instance**.

What counts as an instance:

* **Instance points** (`instance` node, USD point instancers, PCG): points with an `instance` attribute and the detail `instance_objects`, with `rot` /
  `scale` / `orient` / `N` read exactly as `copy_to_points` reads them. With a live Blender link the shapes are the Blender objects' geometry; USD point
  instancers also bring the **size** of each prototype (detail `instance_bounds`), so without a shape each instance is a box of the right size. With neither,
  an instance is a unit box times its scale and the Perf statistics say "sizes guessed". The points stay in the view (selecting them works).
  With a live link, instead of realising every copy for the preview (which stops at 2 million vertices) the engine keeps the points and sends the sources
  once whenever the copies would be over the display budget.
* **Copies that were really made** (`copy_to_points`, anything repeated end to end): geometry that is a block of points / vertices / primitives repeated, where
  every repetition is an affine copy (least-squares fit, within 0.2% of its size) of the first. Nothing is stored on the geometry, so nothing can go stale; edit a
  copy by hand and it stops being recognised. At least 8 copies of at least 8 points and 4 primitives. Only in a view that would be reduced anyway:
  with **Full res**, a selection mode or paint mode the real geometry is sent as before.

Drawing: per camera move each instance gets a level from its **diameter on screen** (detailed above ~70 px, coarse above ~22 px, a bounding box above 3 px,
a dot above 0.8 px), the instances outside the view are skipped, and the total triangles are held to a budget (thresholds grow until it fits). The detailed
level is the shape reduced to 4,000 triangles, the coarse one to 300. The *Perf* menu sets Impostors (Auto / Always / Never), the detail bias and the budget.
`POST /api/preview` takes `instances: auto|always|never`; the header carries `instances` and the arrays `ip_P`, `ip_tri`, `inst_M`, `inst_proto`, `inst_sph`,
`inst_col`. Measured here (headless software GL): 300 copies of a 960-triangle sphere (288,000 triangles) draw 6,480 triangles at a middle distance (12 coarse
spheres, 288 boxes) and 86,076 close up (18 detailed, 273 coarse), and nothing once every instance is under a pixel. The statistics update in under
2 ms for 300 instances; a million instances is a few tens of milliseconds per camera move (a set over 150,000 refreshes a few times a second while moving).

## 4. Large-scene view settings (the Perf menu)

In the viewport's display strip. All remembered between sessions.

| setting | what it does |
|---|---|
| Impostors | Auto, Always, Never (above). |
| Impostor detail | Higher keeps the detailed level for smaller instances. |
| Triangles for instances | The most all instances draw together: 250k to 3M. |
| Display budget | The triangles a geometry may have before it is drawn reduced (default 250k; 100k to 2.5M). Full res and selecting still load everything the viewer can hold. |
| Skip what is outside the view | A triangle list over 100,000 is sent sorted along a space-filling curve with a table of runs of 8,192 triangles and their boxes; runs outside the view (or beyond the section plane) are not drawn. Selecting and painting still use all of them. |
| Skip pieces smaller than N px | Optionally also drop runs that are tiny on screen. |
| While the camera moves | **Balanced** draws at 60% resolution, **Fast** at 45% and without edges, while you orbit, pan or zoom; a sharp frame follows when you stop. |
| Camera clip | Near and far distance of the camera in perspective view; empty is automatic from the scene's size. |
| Section plane | Hides everything beyond a plane (axis X / Y / Z, offset, flip, *Centre*). What is cut away cannot be picked, box-selected or painted. |
| Statistics | Triangles drawn of the total, runs drawn, instances by level, frame time. |

Everything here is display. Nothing changes the cooked geometry, the Blender link or an export.

## Disk cache

`minihou/diskcache.py`, opt-in (`--disk-cache`, see the README). Because every result is keyed by a Merkle hash of its node type and
version, its library code, its parameters at the proxy level and the keys of its inputs, the key doubles as a file name:
`.minihou_cache/<key[:2]>/<key>/` holds `meta.json` and one `.npy` per array (written to a temporary folder, then renamed, on a
background thread). The evaluator asks the disk when memory misses, *before* cooking inputs, so one hit skips a whole upstream
branch. Proxy levels have their own entries, so Full is still there after a trip to 10%.

Never stored: results that took under 100 ms or that would take longer to read than half their cook time; anything downstream of
`blender_object`, `modifier_input`, `rbd_solver`, `solver` or geometry fed into a system (their keys are not stable between runs);
results whose detail attributes would not survive JSON unchanged (a tuple, a numpy value), so a hit is always identical to a fresh
cook. A damaged entry counts as a miss and is deleted. Least recently used entries go when the folder passes its cap.
Measured: remesh of a sphere at 0.03 voxels 4.5 s, 48 ms after a restart (the cheap `smooth` after it is cooked again).

## Not done

* Impostors are boxes and reduced meshes, not baked billboards. Copies are recognised only when they are one repeated block (a merge of several different shapes is drawn by the ordinary reduction); a copy that was edited by hand loses its status.
* Cook parameters are marked on the commonest count / resolution parameters only; others run at full size at any level.
* The disk cache is off by default (flag, environment variable, or the Proxy menu in the editor).
* Packed / instanced RBD pieces and the solver always cook the real thing in their input; do not use a proxy to judge a simulation.
