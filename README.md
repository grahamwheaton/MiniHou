# BlenderHoudini ("MiniHou")

A small, standalone **procedural geometry engine** in Python (numpy, optionally numba) that **live-links into Blender 5.1**.
You describe a node graph as JSON, the engine cooks it, and the result streams into Blender as a mesh you can
drive with Geometry Nodes. Edit the JSON, or drag a modifier slider in Blender, and only what changed is
recooked and re-sent.

```
 graph.json ──►  engine process (minihou)  ──── TCP 127.0.0.1:8765 ────►  Blender add-on (minihou_link)
                 pull-based cooking                changed arrays only        timer -> foreach_set -> mesh
                 hash-keyed cache                  topology only if changed   proxy object + Geometry Nodes
                 ◄──── set_param (slider drags) ───────────────────────────  modifier inputs = exposed params
```

Status: milestone 1 complete, plus the first milestone-2 pieces (wrangle, normal, delete, primitives, numba backend).
Every push runs CI on Linux and Windows, including the add-on inside a real Blender 5.1.2 on Windows
(`.github/workflows/ci.yml`). Design notes and next steps: [docs/PLAN.md](docs/PLAN.md).

## Quick start (Windows)

You need Python 3.11+ and Blender 5.1.

```powershell
cd path\to\BlenderHoudini
py -m pip install numpy numba          # numba is optional: 4-24x faster heavy nodes; everything works without it

py -m unittest discover -s tests -t .  # engine tests (no Blender needed)
py tools\launch_blender.py --engine examples\graphs\demo.json
```

The launcher finds Blender (Program Files, per-user installs, Steam, `PATH`, or `$env:BLENDER_EXE`), starts the engine,
and opens Blender with the add-on registered and connected. Press **N** in the viewport and open the **MiniHou** tab:
connection state, cook/write timings and the exposed parameters. The `MiniHou_Proxy` object carries a Geometry Nodes
modifier whose inputs are the same parameters (also editable, keyable and drivable in the modifier panel). Edit the
graph JSON in any editor and save: the engine hot-reloads it and only the changed nodes recook.

```powershell
$env:BLENDER_EXE = "C:\Program Files\Blender Foundation\Blender 5.1\blender.exe"   # if auto-detection misses it
py tools\launch_blender.py --find
```

Manual launch: `py -m minihou serve graph.json` in one terminal, then
`blender.exe --python tools\blender_bootstrap.py` (add `-- --port N` for another port). Permanent install:
`py tools\sync_addon.py`, zip `blender_addon\minihou_link`, *Edit > Preferences > Add-ons > Install from Disk*.

Other commands: `py -m minihou nodes` (all node types and parameters), `py -m minihou cook graph.json` (cook once,
print timings and cache behaviour).

## Examples

| file | what it shows |
|---|---|
| `examples/graphs/demo.json` | grid + noise hills + wrangle ripple, scattered spikes via copy_to_points |
| `examples/graphs/spiky_sphere.json` | sphere + vector noise, normals, scatter, delete by expression, per-point `pscale` from a wrangle, box spikes |
| `examples/graphs/terrain_rocks.json` | 200x200 terrain with 5-octave noise, a valley wrangle, rocks scattered only above a height |

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
becomes the tooltip). The JSON file is the source of truth: slider changes live in the running session and are replaced when
the file is reloaded. Inputs that survive a reload keep their identity, so keyframes and drivers on them keep working.

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
py -m unittest discover -s tests -t .                  # 261 tests; Blender ones skip without bpy
py tools\launch_blender.py --selftest                  # add-on tests inside `blender --background`, engine as a separate process
py tools\launch_blender.py --bench -- --points 1000000 # the 1M-point benchmark inside Blender (add --mode engine for slider edits)
py tools\launch_blender.py --gui-test                  # interactive smoke test in a real Blender window (~30 s)
```

`--gui-test` opens a normal Blender window and lets Blender's own event loop drive the add-on: Connect operator, panel drawn by the
real UI, a 30-step slider drag forwarded by the depsgraph handler, an idle period proving there is no feedback loop, and two graph
reloads (adding, removing, reordering inputs) while the sidebar and Blender's modifier panel show them. It prints PASS/FAIL per check,
saves screenshots to `gui_test_output\`, and closes Blender. `-- --reload-cycles 40` adds a stress test.

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
* The socket is unauthenticated and bound to loopback by default. Do not bind it to a public interface.
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
  link/                   protocol.py + client.py (shared with the add-on), diff.py, session.py, server.py
blender_addon/minihou_link/
  runtime.py              main-thread timer, depsgraph handler, feedback-loop guards
  mesh_writer.py          foreach_set writer (fast attribute paths)
  proxy.py                output/proxy objects, node group sync (incremental), modifier values
  state.py                bpy-free inbox + parameter forwarder
tools/   launch_blender.py, blender_bootstrap.py, blender_selftest.py, gui_smoketest.py, sync_addon.py
bench/   bench_link.py, raw_sender.py
tests/   unit, integration, Blender (bpy), golden values, portability
examples/graphs/  docs/  .github/workflows/
```
