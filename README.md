# BlenderHoudini ("MiniHou")

A small, standalone **procedural geometry engine** in Python (numpy only) that **live-links into Blender 5.1**.
You describe a node graph as JSON, the engine cooks it, and the result streams into Blender as a mesh you can
drive with Geometry Nodes. Edit the JSON, or drag a modifier slider in Blender, and only what changed is
recooked and re-sent.

```
 graph.json ──►  engine process (minihou)  ──── TCP 127.0.0.1:8765 ────►  Blender add-on (minihou_link)
                 pull-based cooking                changed arrays only        timer -> foreach_set -> mesh
                 hash-keyed cache                  topology only if changed   proxy object + Geometry Nodes
                 ◄──── set_param (slider drags) ───────────────────────────  modifier inputs = exposed params
```

Status: **milestone 1**. See [docs/PLAN.md](docs/PLAN.md) for the design and what comes next.

## Quick start (Windows)

You need Python 3.11+ with numpy 1.26+ (`py -m pip install numpy`) and Blender 5.1.

```powershell
cd path\to\BlenderHoudini

# 1. run the engine tests (no Blender needed)
py -m unittest discover -s tests -t .

# 2. launch engine + Blender together (auto-detects Blender under Program Files / Steam)
py tools\launch_blender.py --engine examples\graphs\demo.json
```

Blender opens with the add-on registered and connects itself. In the 3D viewport press **N** and open the
**MiniHou** tab: you will see the connection state, cook/write timings, and the exposed parameters. A
`MiniHou_Proxy` object (with a Geometry Nodes modifier) appears; drag its inputs and the engine recooks.
Open `examples/graphs/demo.json` in an editor, change a number, save, and the mesh updates (the engine
hot-reloads the file).

If Blender is not found, point at it explicitly:

```powershell
$env:BLENDER_EXE = "C:\Program Files\Blender Foundation\Blender 5.1\blender.exe"
py tools\launch_blender.py --find         # prints what it will use
```

Manual launch (two terminals), if you prefer:

```powershell
py -m minihou serve examples\graphs\demo.json
& "C:\Program Files\Blender Foundation\Blender 5.1\blender.exe" --python tools\blender_bootstrap.py
```

Other commands: `py -m minihou nodes` (list node types and parameters), `py -m minihou cook graph.json`
(cook once, print timings and cache behaviour).

### Headless Blender checks (what Claude can run for you)

```powershell
py tools\launch_blender.py --selftest                       # addon tests inside Blender, exit code = result
py tools\launch_blender.py --bench -- --points 1000000      # the 1M-point benchmark inside Blender
```

These run `blender --background --factory-startup --python ...` and print everything Blender prints, so
errors are visible in the console. The engine runs as a separate process under the Python that launched it
(override with `MINIHOU_ENGINE_PYTHON`).

To install the add-on permanently instead of using the launcher: run `py tools\sync_addon.py`, zip the
`blender_addon\minihou_link` folder, and use *Edit > Preferences > Add-ons > Install from Disk*. (Only the launcher path
has been exercised so far.)

## Layout

```
minihou/                  the engine (numpy + stdlib only; never imports bpy)
  geometry.py             AttrTable / Topology / Geometry: point, vertex, prim, detail attribute tables
  sdk.py                  @node decorator, Float/Int/Bool/Str/Vec3/Enum params, Input/Output ports, Registry
  graph.py                Graph model, validation, JSON load/save, exposed parameters
  evaluator.py            pull-based cook, hash-keyed LRU cache, per-cook stats
  nodes/                  grid transform noise scatter copy_to_points merge output
  link/protocol.py        wire format  (shared verbatim with the add-on)
  link/client.py          threaded client (shared verbatim with the add-on)
  link/diff.py            per-client "what changed" logic
  link/session.py         graph + evaluator + cook thread + file watcher
  link/server.py          TCP server
blender_addon/minihou_link/
  runtime.py              main-thread timer, depsgraph handler, feedback-loop guards
  mesh_writer.py          foreach_set writer (fast paths, see benchmark)
  proxy.py                MiniHou_Output / MiniHou_Proxy objects, node group, modifier values
  state.py                bpy-free inbox + parameter forwarder (unit tested outside Blender)
  protocol.py client.py   COPIES of minihou/link/* (python tools/sync_addon.py; a test fails on drift)
tools/  bench/  tests/  examples/graphs/  docs/
```

## Writing a node

```python
from minihou.sdk import node, Float, Vec3

@node("my_offset", inputs=["geo"], params=[Vec3("offset", (0, 0, 0)), Float("scale", 1.0, min=0)])
def my_offset(geo, offset, scale):
    out = geo.copy()                      # shallow: untouched arrays stay shared (and stay un-resent)
    out.point["P"] = geo.P * scale + offset
    return out
```

A node is a pure function of its inputs and parameters (the cache depends on that). Inputs are frozen
(read-only arrays), so mutating one raises immediately instead of corrupting the cache. Bump `version=` when
you change a node's behaviour.

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

`exposed` entries become modifier inputs in Blender (float, int, bool and vec3 parameters). The JSON file is the
source of truth: slider changes live only in the running session and are replaced when the file is reloaded.

## Measured: does the speed hold up?

Benchmark: `bench/bench_link.py`, **1,000,000 points, Blender 5.1.2, Linux, 4 cores, loopback socket**
(this is the machine Claude ran on, not yours: run the `--bench` command above on Windows for your numbers).

Blender-side cost of one position-only update (12 MB):

| stage | median |
|---|---|
| socket transport (send -> received) | 6 ms (~1.9 GB/s) |
| decode into mirror | 0.02 ms |
| position write (`foreach_set`) | **1.0 ms** |
| `mesh.update()` | 0.01 ms |

The first version of the writer used `mesh.vertices.foreach_set("co", ...)`, which took **165-175 ms** for 1M
points (68 MB/s: it walks per-vertex RNA structs). Writing through `mesh.attributes["position"].data.foreach_set("vector", ...)`
is a memcpy: ~1 ms. The same applies to corner indices (`.corner_vert`: 1.3 ms vs 337 ms for 4M corners). The writer uses
the fast paths and falls back to the classic API if Blender changes them.

Rebuilding topology for a 1M-quad mesh (only needed when connectivity changes): ~300 ms
(of which ~150 ms is `mesh.update(calc_edges=True)`).

End to end for a slider edit (`set_param` -> engine cook -> diff -> socket -> Blender write) on a grid + noise graph, median of 10
edits; position-only updates, topology untouched:

| points | end-to-end | engine noise cook | Blender write |
|---|---|---|---|
| 10k | 3 ms | 2.4 ms | 0.08 ms |
| 100k | 28 ms | 26 ms | 0.25 ms |
| 250k | 75 ms | 71 ms | 0.5 ms |
| 1M | 285-330 ms | 276-318 ms | 1.7 ms |

So the link is not the bottleneck at any size; the engine's numpy noise is (per-corner gradient lookups are ~80% of its cost). Interactive
(~30 fps) holds up to roughly 100k points for noise-heavy graphs. A prototype that splits the noise over a stdlib thread pool gave
**2.8x on 4 threads** with bit-identical output (and was slower at 8 threads); it is not in the engine yet.

## Verified vs not verified

Verified here:
* 154 tests pass on Python 3.11 (numpy 1.26.4), 3.12 (numpy 2.5) and 3.13 (numpy 2.5). The 18 Blender tests are skipped without `bpy`
  and pass under Blender 5.1.2 (`bpy` module, headless).
* The Blender tests run the add-on against a real engine subprocess over a real socket and compare results on the *evaluated*
  Geometry Nodes proxy: parameter round trip, no feedback loop, burst coalescing, engine restart/reconnect, hot reload of the graph file.
* `tools/blender_bootstrap.py` runs; the four operators run; the panel `draw()` runs against a stub layout; the add-on zip installs and
  enables through Blender's own `addon_install` / `addon_enable`.

Not verified:
* **An interactive Blender session.** Headless Blender never fires `bpy.app.timers`, so tests call `Runtime.tick()` by hand, and the
  bootstrap's deferred connect timer has not fired. The panel has not been rendered by a real UI. This is the biggest remaining unknown.
* **Windows.** Everything above ran on Linux. Windows-specific code that has never run: exclusive port binding
  (`SO_EXCLUSIVEADDRUSE`), Blender discovery paths in `launch_blender.py`, `blender.exe` console behaviour.
* Python 3.10 (grammar-checked only), numpy 1.24/1.25, Blender 4.2-5.0 (the add-on declares 4.2+ but was only run on 5.1.2).
* A real `blender` executable: Claude had only the `bpy` PyPI module, not the binary, so `launch_blender.py --selftest/--bench` have
  been run through the `bpy` module equivalents, not through `blender --background`.

Please try the quick start and send me whatever the Blender console prints.

## Known limits (milestone 1)

* One output node/mesh; one exposed-parameter set. Detail attributes are not sent to Blender. No normals/UV maps
  (a `uv` *point* attribute is sent as a generic attribute).
* The socket is unauthenticated and bound to loopback by default. Do not bind it to a public interface.
* Ints with 3 components, strings, and 64-bit types in attributes are skipped with a warning in the panel/console.
