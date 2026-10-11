# BlenderHoudini ("MiniHou")

A small, standalone **procedural geometry engine** in Python (numpy, optionally numba) that **live-links into Blender 5.1/5.2**.
You build a node graph in a **browser node editor**, by hand in JSON, or by **asking an AI** (built-in MCP server); the engine cooks
it and the result streams into Blender as a mesh you can drive with Geometry Nodes. Change anything, anywhere (editor, AI, JSON
file, a modifier slider in Blender) and only what changed is recooked and re-sent; every other view follows live.

```
 node editor (browser) ──┐  HTTP + live events     engine process (minihou)     TCP 127.0.0.1:8765
 AI assistant (MCP)  ────┼─ 127.0.0.1:8766 ──────►  pull-based cooking   ───────────────────────►  Blender add-on
 graph.json  ◄───────────┘  (edits, undo, inspect)  hash-keyed cache       changed arrays only      proxy + Geometry Nodes
                            ◄── saved / hot-reloaded                  ◄─── modifier slider drags ──  inputs = exposed params
```

![An AI assistant built this crater-with-boulders system through MCP; it streamed live into Blender](docs/images/blender_ai_crater.png)

Status: milestone 1 complete; milestone 2 well under way (wrangle, normal, delete, primitives, numba backend, node editor, MCP).
Every push runs CI on Linux and Windows, including the add-on inside a real Blender 5.1.2 on Windows
(`.github/workflows/ci.yml`). Design notes and next steps: [docs/PLAN.md](docs/PLAN.md).

## Quick start (Windows)

You need Python 3.11+ and Blender 5.1.

```powershell
cd path\to\BlenderHoudini
py -m pip install numpy numba          # numba is optional: 4-24x faster heavy nodes; everything works without it

py -m unittest discover -s tests -t .  # engine tests (no Blender needed)
py tools\launch_blender.py --engine examples\graphs\demo.json --editor
```

The launcher finds Blender (Program Files, per-user installs, Steam, `PATH`, or `$env:BLENDER_EXE`), starts the engine,
and opens Blender with the add-on registered and connected. Press **N** in the viewport and open the **MiniHou** tab:
connection state, cook/write timings and the exposed parameters. The `MiniHou_Proxy` object carries a Geometry Nodes
modifier whose inputs are the same parameters (also editable, keyable and drivable in the modifier panel). Edit the
graph JSON in any editor and save: the engine hot-reloads it and only the changed nodes recook. `--editor` also opens the
**node editor** at <http://127.0.0.1:8766/> (the URL is printed either way).

```powershell
$env:BLENDER_EXE = "C:\Program Files\Blender Foundation\Blender 5.1\blender.exe"   # if auto-detection misses it
py tools\launch_blender.py --find
```

Manual launch: `py -m minihou serve graph.json` in one terminal, then
`blender.exe --python tools\blender_bootstrap.py` (add `-- --port N` for another port). Permanent install:
`py tools\sync_addon.py`, zip `blender_addon\minihou_link`, *Edit > Preferences > Add-ons > Install from Disk*.

Errors in Blender's console from *other* add-ons (common on a new Blender version) are not MiniHou's; MiniHou's own lines
start with `[minihou]`. `--factory` starts Blender without your add-ons and preferences for a clean console.

Other commands: `py -m minihou nodes` (all node types and parameters), `py -m minihou cook graph.json` (cook once,
print timings and cache behaviour), `py -m minihou serve graph.json --open` (engine + editor, no Blender).

## Node editor

![The node editor showing the AI-built crater graph](docs/images/editor.png)

Served by the engine itself (plain HTML/JS, nothing to install or build). Houdini-style: **Tab** (or double-click) adds a node,
wired to the selected one; drag from a node's bottom dot to another node to connect, drag a connected input to re-route or
disconnect; the blue bar is the **display flag** (what Blender shows); **L** tidies the layout, **F** frames, **Ctrl+Z** undoes.
The right side shows the selected node's parameters (sliders, drag a label to scrub, a code box for wrangles, **B** exposes a
parameter to Blender) and an **inspector** with the node's point/prim counts, bounds and attributes. Cook errors are shown on
the failing node. Everything is live: drag a slider in Blender and the editor follows; edit in the editor and Blender follows.
Edits are saved to the graph file (and are undoable, including edits made by an AI). Press **?** for all controls.

## Build systems with AI (MCP)

The engine includes an [MCP](https://modelcontextprotocol.io) server, so an AI assistant can build and tweak graphs in the running
engine while you watch in the editor and in Blender. One-time setup:

```powershell
py tools\setup_mcp.py      # writes .mcp.json for Claude Code, prints the Claude Desktop config, checks the server starts
```

Then start `claude` in this folder (approve the `minihou` server) and ask, for example:

> Make a new graph graphs/crater.json: a 12 m dune field with a crater in the middle and boulders scattered on the crater rim.
> Expose the boulder count and dune height to Blender.

The assistant gets these tools: `engine_status`, `list_node_types` (every node, input and parameter with ranges and docs),
`get_graph`, `edit_graph` (atomic batches of edit operations; the reply says whether the cook worked, which node failed and why,
and point/prim counts), `inspect_node` (counts, bounds and attribute ranges at any node), `undo`, `list_graphs`, `open_graph`
(also starts the engine if it isn't running), `wrangle_reference` and `node_library` (write brand-new node types in Python,
see below). The screenshots above are from exactly that loop.
The same edit operations are available to scripts over HTTP (`POST /api/edit`, see `minihou/edit.py` and `minihou/web/server.py`).

## Examples

| file | what it shows |
|---|---|
| `examples/graphs/demo.json` | grid + noise hills + wrangle ripple, scattered spikes via copy_to_points |
| `examples/graphs/spiky_sphere.json` | sphere + vector noise, normals, scatter, delete by expression, per-point `pscale` from a wrangle, box spikes |
| `examples/graphs/terrain_rocks.json` | 200x200 terrain with 5-octave noise, a valley wrangle, rocks scattered only above a height |
| `examples/graphs/crater.json` | built by an AI through MCP: dunes, a crater wrangle, boulders kept only on the rim by a `delete` expression |
| `examples/graphs/spiral_stairs.json` | uses the example library nodes `stairs` (spiral treads around a column) and `twist` |
| `examples/graphs/rocky_hills.json` | hills plus the `rock_scatter` **system** (`nodes/rock_scatter.json`, a graph used as a node) |

All examples are cooked by the test-suite, so they keep working as nodes evolve.

## Nodes

| node | inputs | what it does |
|---|---|---|
| `grid` | - | XY grid of quads; `uv` point attribute |
| `box` | - | closed box, optional divisions (welded seams) |
| `sphere` | - | UV sphere (triangle-fan poles), closed and outward-facing |
| `circle` | - | one filled n-gon |
| `transform` | geo | scale / rotate (Euler XYZ) / translate about a pivot; rotates `N` |
| `noise` | geo | fractal Perlin noise; displaces `P` or writes any attribute (scalar or vector mode) |
| `wrangle` | geo | VEX-like snippet over points (see below) |
| `normal` | geo | area-weighted point normals or face normals (Newell, n-gon safe) |
| `scatter` | geo | area-weighted random points on the surface (`P`, `N`, `primnum`) |
| `copy_to_points` | source, points | stamp `source` on every point: orient +Z to `N`, scale by `pscale`, inherit point attributes |
| `delete` | geo | delete points matching an expression (and primitives that use them), compacting all attributes |
| `fuse` | geo | weld points closer than a distance |
| `merge` | geos (any number) | concatenate; missing attributes are zero-filled |
| `output` | geo | marks what the live link publishes (validated) |

### Wrangle snippets

```
@P.z += sin(@P.x * 2) * 0.5                       # read/modify attributes; .x .y .z components
v@Cd = set(fit(@P.z, -1, 1, 0, 1), 0, 1)          # f@ v@ i@ create float / vector / int attributes
f@mask = 1 if length(@P) < 3 and @P.z > 0 else 0  # conditionals and boolean logic
h = noise(@P * 0.3); @P += @N * h                 # locals; several statements (; or new lines)
i@id = @ptnum                                     # @ptnum, @numpt; constants PI, E
```

Functions: `sin cos tan asin acos atan atan2 sqrt exp log abs floor ceil frac pow sign min max clamp lerp fit smooth length
normalize dot cross set noise rand select radians degrees`. A float times a vector works as in VEX. Snippets are compiled once
(cached by text) to numpy and are **sandboxed**: they are parsed and checked against a whitelist, so a shared graph file cannot run
arbitrary Python (imports, attribute access, strings, builtins... are rejected before anything runs; 42 attack patterns are tested).
Typical snippets take 20-50 ms on 1M points. The `delete` node takes a wrangle *expression* (`@P.z < 0`).

## Writing a node

### Node libraries (hot-reloaded)

![A spiral staircase from the example library node nodes/stairs.py, live in Blender](docs/images/spiral_stairs.png)

Any `*.py` file in the workspace's `nodes/` folder is loaded by the engine and **re-loaded whenever you save it**: its `@node`
functions appear in the editor's Tab menu (and to AI tools) within a second, and graphs using them recook. Nothing to restart.

```powershell
py -m minihou new-node ripple_rings        # writes nodes\ripple_rings.py from a template; edit and save
```

* Cache-safe: each library node's cache key includes a hash of its file, so editing code never serves stale results.
* Failure-safe: a syntax error or exception keeps the previous version loaded; the error (file and line) shows in the editor
  and in AI tool replies. Runtime errors inside a node name the file and line too.
* A library node cannot take the name of an existing node. Graphs that use a node type which disappeared still open (the
  cook reports the missing type) and round-trip unchanged.
* `nodes/twist.py` (a deformer) and `nodes/stairs.py` (a generator that builds topology with numpy; spiral mode above) are
  examples; `examples/graphs/spiral_stairs.json` uses both. `serve --nodes DIR` adds folders.
* Library files are Python running inside the engine, trusted like any script you run. Opening a graph never runs code.

AI assistants use the same mechanism: the MCP `node_library` tool explains how to write a node and reports load errors, and the
assistant writes the file with its normal (permission-checked) file tools. There is deliberately no MCP tool that writes code.

### Systems: graphs as nodes

![A system node in the editor: its parameters are the system graph's exposed parameters](docs/images/system_node.png)

A graph file saved in the node folder is itself a node type, named after the file, like a Houdini digital asset:
`input` nodes inside it are its inputs, its **exposed parameters become the node's parameters**, its display node is the
output and its top-level `"doc"` is the description. `nodes/rock_scatter.json` is an example (scatter rough rocks of varied
size over whatever ground you wire in); `examples/graphs/rocky_hills.json` uses it. Build one by opening
`nodes/<name>.json` in the editor (File > New graph) or asking an AI; while editing, wire a stand-in into an `input` node's
`preview` port. Select a system node and press **Edit system** to open it; saving re-loads it everywhere. Systems can use
other systems and Python library nodes; each system's cache key covers the code of everything inside it, so editing a
nested system or a Python node updates every graph that uses it. A system that uses itself is reported, not looped.

### The SDK

```python
from minihou.sdk import node, Float, Vec3

@node("my_offset", inputs=["geo"], params=[Vec3("offset", (0, 0, 0)), Float("scale", 1.0, min=0)])
def my_offset(geo, offset, scale):
    out = geo.copy()                      # shallow: untouched arrays stay shared (and are not re-sent to Blender)
    out.point["P"] = geo.P * scale + offset
    return out
```

A node is a pure function of its inputs and parameters (the cache depends on that). Inputs are frozen (read-only arrays),
so mutating one raises immediately instead of corrupting the cache. Bump `version=` when you change a node's behaviour.

## Graph JSON

```json
{
  "version": 1,
  "nodes": {
    "ground": {"type": "grid", "params": {"columns": 60, "rows": 60}},
    "hills":  {"type": "noise", "params": {"amplitude": [0, 0, 1.2]}, "inputs": {"geo": "ground"}},
    "out":    {"type": "output", "inputs": {"geo": "hills"}}
  },
  "display": "out",
  "exposed": [{"name": "Hill Height", "node": "hills", "param": "amplitude"}]
}
```

`exposed` entries become modifier inputs in Blender (float, int, bool and vec3 parameters; the parameter's doc string
becomes the tooltip). Nodes may also carry editor data (`"pos": [x, y]`, `"comment": "..."`), which never affects cooking.
The running engine saves every edit (editor, AI, Blender slider) back to the file within a quarter second, and reloads the file
when something else changes it, so the file and every view stay in sync. Saved files stay diff-friendly: short arrays on one
line, whole numbers as ints, and values from Blender's float32 sliders snapped to their shortest decimal (`0.22`, not
`0.2199999988079071`). Inputs that survive a reload keep their identity, so
keyframes and drivers on them keep working. A graph may be unfinished (an unwired node): it still opens, and only the nodes
feeding the display node have to be valid for it to cook.

## Speed

Measured with `bench/bench_link.py` (1M points, Blender 5.1.2). Windows numbers are from the GitHub Actions Windows runner
(re-measured on every CI run; see the "Blender 5.1 on Windows" job log), Linux from Claude's 4-core sandbox.

| 1M points | Windows (CI) | Linux |
|---|---|---|
| positions over the socket (12 MB) | 2.4 ms | 6 ms |
| Blender write (`foreach_set`) | 0.4 ms | 1.0 ms |
| raw update, end to end | **2.9 ms** (~350/s) | 7 ms |
| slider edit, grid + noise, end to end (numba) | **52 ms** | 29 ms |
| first full frame: Blender mesh build (1M quads) | 406 ms | 240 ms |

Smaller graphs on Linux (slider edit, end to end): 10k points 1.5 ms, 100k 4 ms, 250k 6.6 ms (numba); numpy-only is 2-11x slower.
Per node at 1M points with numba vs numpy: noise 12.7 vs 283 ms, 4-octave noise 46 vs 1106 ms, scatter 160 vs 1012 ms,
copy_to_points (1M copies) 134 vs 536 ms, normal 63 vs 662 ms.

Two findings made the Blender side fast: `mesh.vertices.foreach_set("co")` walks per-vertex RNA structs (165 ms for 1M points) while
`mesh.attributes["position"].data.foreach_set("vector")` is a memcpy (~1 ms); likewise `.corner_vert` (1.3 ms) vs
`loops.foreach_set("vertex_index")` (337 ms for 4M corners). The writer uses the fast paths and falls back to the classic API if a
Blender version changes them. Topology rebuilds (only when connectivity changes) are dominated by Blender's own edge computation.

### Reliability of the numba backend

* **Optional**: without numba (or with `MINIHOU_NO_NUMBA=1`) the numpy code runs; the whole suite runs both ways in CI.
  `py -m minihou serve` prints the active backend.
* **Same algorithm**: kernels use the same hash, RNG stream and float32 coordinate math as the numpy code; parity tests (mutation-checked)
  and **golden values** pin results across platforms, numpy versions and backends.
* **A broken JIT cannot fail a cook**: a kernel error switches acceleration off for the process and the cook finishes on numpy.
* **No timing-dependent results**, **thread-safe launches** (numba's default threading layer would abort on concurrent launches).
* **Start-up**: kernels compile once (~4 s, in the background after `serve` starts) and are cached on disk (~0.15 s afterwards).

## Automated checks (what Claude runs, what you can run)

```powershell
py -m unittest discover -s tests -t .                  # 339 tests; Blender ones skip without bpy, browser ones without playwright
py tools\launch_blender.py --selftest                  # add-on tests inside `blender --background`, engine as a separate process
py tools\launch_blender.py --bench -- --points 1000000 # the 1M-point benchmark inside Blender (add --mode engine for slider edits)
py tools\launch_blender.py --gui-test                  # interactive smoke test in a real Blender window (~30 s)
```

`--gui-test` opens a normal Blender window and lets Blender's own event loop drive the add-on: Connect operator, panel drawn by the
real UI, a 30-step slider drag forwarded by the depsgraph handler, an idle period proving there is no feedback loop, and two graph
reloads (adding, removing, reordering inputs) while the sidebar and Blender's modifier panel show them. It prints PASS/FAIL per check,
saves screenshots to `gui_test_output\`, and closes Blender. `-- --reload-cycles 40` adds a stress test. On Linux it clicks the
sidebar's MiniHou tab with xdotool; elsewhere (no OS input) it shows a copy of the panel in the active sidebar tab, so the
add-on's draw code always runs in Blender's real sidebar and the "panel drew" checks need real draws on every OS.

The editor is tested in a real headless Chromium (`tests/test_editor_e2e.py`: clicks, Tab menu, wire drags, undo, wrangle
errors, live updates from other clients, and that text from graph files can never become HTML); install
`py -m pip install playwright` and `py -m playwright install chromium` to run those locally.

CI (`.github/workflows/ci.yml`) runs on every push: engine tests on Ubuntu and Windows (Python 3.11-3.13, with and without numba,
`EncodingWarning` as an error); on Windows, Blender 5.1.2 is downloaded and `--selftest`, `--bench` and `--gui-test` run (the GUI one
with Mesa's software OpenGL next to `blender.exe`). Screenshots and logs are uploaded as artifacts.

## Verified vs not verified

Verified:
* **Windows**: engine tests (Python 3.12 + numba, 3.13 numpy-only), the add-on self-test in real Blender 5.1.2 for Windows, the
  benchmark, and the interactive GUI smoke test (software OpenGL) all pass in CI.
* **Linux**: the same, plus repeated GUI runs and a 40-reload stress run in a real Blender window with screenshots; Python 3.11
  (numpy 1.26), 3.12, 3.13.
* **A Windows 11 desktop** (Python 3.14, numba 0.68): engine tests, `--selftest` and `--gui-test` in Blender 5.1.1 and 5.2.1 LTS.
  Blender 5.2 removed ID properties from modifiers; the add-on reads/writes Geometry Nodes inputs through
  `proxy.get_input`/`set_input`, which use `mod.properties.inputs.<Socket>.value` there and `mod["<Socket>"]` before. Keyframe/driver
  paths differ accordingly (`proxy.input_data_path`).
* **Editor + MCP + Blender together** (Linux, Blender 5.1.2 window): an MCP client built the crater system from a starter graph,
  then changed it; Blender showed each step (exposed values pushed into the modifier, 29 ms end to end) while the editor followed.
* Not fooled by flaky timing: Blender tests wait for convergence; cross-platform determinism is pinned by golden values.

Found by these checks and fixed: float32 round-trips causing phantom parameter edits; depsgraph churn from re-assigning the node group;
greyed-out ("unused") modifier inputs; interface rebuilds dropping keyframes/drivers; Windows text-encoding hazards; tests that assumed
fixed timing.

Not verified:
* **Your machine**: a real GPU driver, your Python/numba install, Blender 4.2-5.0 (the add-on declares 4.2+).
* Known Blender behaviour, not reachable through the add-on's UI: showing the MiniHou panel as a *keep-open popover*
  (`wm.call_panel(..., keep_open=True)`, which nothing in the add-on does) during a graph reload crashes Blender 5.1.2 in its UI code
  (reproducible with `--gui-test -- --popover-during-reload`; seen on Windows CI as well, when an earlier version of the test
  left its popover open). The sidebar and the modifier panel are safe across 100+ reloads. The GUI test now detects a Blender crash
  report and stops at once instead of waiting behind the Windows error dialog.

## Known limits

* One output mesh and one exposed-parameter set per engine. Detail attributes are not sent to Blender. Point-domain wrangles only.
* Polygons only (no curves/edges-only geometry). No UV maps (a `uv` point attribute is sent as a generic attribute).
* The socket and the HTTP API are unauthenticated and bound to loopback by default. Do not bind them to a public interface.
  The HTTP API refuses requests from other web sites (Host/Origin checks against DNS rebinding and cross-site requests) and
  only opens `.json` files inside its workspace folder.
* Editing an example graph saves into it (use the editor's *New graph...* or copy it first; `git checkout` restores it).
* Ints with 3 components, strings and 64-bit attribute types are skipped with a warning.

## Layout

```
minihou/                  the engine (numpy + stdlib, optional numba; never imports bpy)
  geometry.py             AttrTable / Topology / Geometry: point, vertex, prim, detail attribute tables
  sdk.py                  @node decorator, Float/Int/Bool/Str/Vec3/Enum params, Input/Output ports, Registry
  graph.py                Graph model, validation, JSON load/save, exposed parameters
  evaluator.py            pull-based cook, hash-keyed LRU cache, per-cook stats
  wrangle.py              sandboxed snippet compiler (AST whitelist -> numpy)
  accel.py                optional numba backend: availability, safe kernel launch, fallback, warm-up
  nodes/                  one module per node family; nodes/_kernels.py holds the numba kernels
  edit.py                 graph edit operations (add/connect/set/expose/... + layout), atomic batches
  summary.py              geometry summaries for the inspector and AI tools
  link/                   protocol.py + client.py (shared with the add-on), diff.py, session.py, server.py
  web/                    HTTP API + live events (server.py) and the node editor (static/)
  mcp.py                  MCP server for AI assistants (stdio, stdlib only)
  library.py              node libraries: load and hot-reload nodes/*.py, authoring guide
nodes/                    example node library: twist.py, stairs.py (Python) and rock_scatter.json (a system)
blender_addon/minihou_link/
  runtime.py              main-thread timer, depsgraph handler, feedback-loop guards
  mesh_writer.py          foreach_set writer (fast attribute paths)
  proxy.py                output/proxy objects, node group sync (incremental), modifier values
  state.py                bpy-free inbox + parameter forwarder
tools/   launch_blender.py, blender_bootstrap.py, blender_selftest.py, gui_smoketest.py, sync_addon.py,
         setup_mcp.py, mcp_server.py
bench/   bench_link.py, raw_sender.py
tests/   unit, integration, Blender (bpy), golden values, portability
examples/graphs/  docs/  .github/workflows/
```
