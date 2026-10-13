# Proxies for very large scenes

Two separate things keep a big scene usable. Both are on by default or one click away, and neither changes what a graph
means at full size.

## 1. The viewport always draws within a budget (display proxy)

`minihou/proxy.py` `decimate()` reduces any geometry to a budget of triangles and points. The 3-D view asks for
250,000 triangles / 500,000 points (`preview.DISPLAY_TRIS`, `DISPLAY_POINTS`); "Full res" in the view raises that to
600,000 / 2,000,000 (`MAX_*`); past those the same reduction applies, so a request can never hang the browser.

* **Meshes**: vertex clustering. Points that fall in the same cell of a 3-D grid merge into their mean; the cell size is
  searched (on a sample of the points, then checked on all of them) so the triangles that survive are just under the budget.
  The whole shape stays visible. The old preview cut the triangle list off after the first N triangles, which showed
  part of a mesh and nothing of the rest.
* **Curves**: every k-th curve. **Loose points**: an even sample.
* **Attributes** follow: float point attributes average over a cluster (`N` is renormalised), everything else (ints, groups)
  takes the first member; primitive and vertex attributes follow the triangle that survived.
* The preview header says what happened: `proxy: {active, points, tris, shown_points, shown_tris}`; `point_src` and
  `tri_prim` map what is drawn back to original point and primitive numbers.
* The view shows "Reduced view: 250,000 of 3,996,002 triangles" with a **Full res** button. While the view is reduced,
  point/edge/face selection is off (the drawn points are not the real ones); Full res turns it back on.
* Measured (Linux sandbox, preview request incl. reduction, quad grid): 1M quads 1.6 s and 7.6 MB on the wire (was 5.1 s,
  31 MB, truncated); 4M quads 6.3 s and 8.4 MB (was truncated at 48 MB). Cooking those grids takes 0.2-0.4 s, so the
  preview is the slow part. Results are remembered per cooked geometry, so a second request for the same node is instant.
  Meshes with more than 100,000 primitives are fan-triangulated instead of ear-clipped (a concave polygon may show the
  wrong diagonal in that case; nothing else is affected).

`POST /api/preview` takes `full` and `tris` (a custom triangle budget).

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

* Instances are not reduced separately (an instance node with a million placements counts by its points).
* Cook parameters are marked on the commonest count / resolution parameters only; others run at full size at any level.
* The disk cache is off by default (flag, environment variable, or the Proxy menu in the editor).
* Packed / instanced RBD pieces and the solver always cook the real thing in their input; do not use a proxy to judge a simulation.
