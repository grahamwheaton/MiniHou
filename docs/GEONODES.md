# MiniHou inside Geometry Nodes

MiniHou wired groups process the mesh connected to their Geometry input through the external engine. Their output can feed native nodes and further independent MiniHou groups. This is a staged bridge: Blender evaluates the actual upstream socket on disposable object/group copies, the engine computes the result, and a private persistent mesh publishes that result to downstream Geometry Nodes. The saved result is a cache of that computation; it is not a substitute for reading the input.

## Run the example

From the MiniHou checkout, start the existing external engine in one terminal. No engine packages are installed in Blender.

```powershell
.\build\runtime\python312\python.exe -m minihou serve examples/modifiers/transform.json --port 8765
```

Use your existing engine Python path if the runtime lives elsewhere. Embedded Python distributions can have a fixed `_pth` pointing to another checkout. For those, explicitly prepend this checkout:

```powershell
$repo = (Get-Location).Path
& 'C:\Users\Graham\Documents\MiniHou-rbd\build\runtime\python312\python.exe' -c "import sys,runpy; sys.path.insert(0,r'$repo'); sys.argv=['minihou','serve','examples/modifiers/transform.json','--port','8765']; runpy.run_module('minihou',run_name='__main__')"
```

In another terminal:

```powershell
& 'C:\Program Files\Blender Foundation\Blender 5.2\blender.exe' --background --factory-startup --python tools/blender_geonodes_demo.py -- --port 8765 --save build/geonodes-demo.blend
```

Omit `--background` to keep the example open interactively. `--save` is optional. The script adds a cube and material without deleting existing scene objects, registers the checkout addon for the session, verifies both placements cooked, and prints `MINIHOU_GEONODES_DEMO_OK`. Its network is:

```text
Group Input -> native scale -> MiniHou A Scale -> native translation
            -> MiniHou B Translate -> native rotation -> Group Output
```

Change the native scale, A's Scale, or B's Translate independently. The cube carries `demo_weight` on its points and a material slot through both placements. The script leaves the timer registered for interactive changes. For later sessions, install/enable the latest addon ZIP and configure the external endpoint. Opening the saved blend without the engine still displays its last valid result.

## Use in an existing network

1. Start the external engine and configure its host/port in MiniHou's connection controls.
2. Open the Geometry Nodes editor's sidebar (`N`), select **MiniHou**, and choose the graph JSON.
3. Click **Add MiniHou Group**, then wire both Geometry sockets. The engine graph must use `modifier_input` on its output dependency path.
4. Set each placement's exposed parameters on its group node. Click **Cook Wired Groups / Stack Now** for deterministic refresh.
5. Select a placement and use **Edit Network** in its right-click menu or MiniHou sidebar. You can also Tab inside the group and use the sidebar action. This opens that occurrence live in the external MiniHou editor with its actual wired input; edits update that occurrence. Linked parameter sockets remain controlled by their enclosing Blender graph. Start the MiniHou app/HTTP editor first. **Reload Group Graph** remains an explicit way to adopt a saved template.

**Group layout (add-on 0.1.7+).** A MiniHou modifier group (and a new wired placement group) contains `Group Input -> MiniHou Plumbing -> Group Output`. The nested plumbing tree holds the generated cache and parameter wiring; do not edit its internal generated nodes. You can duplicate the **MiniHou Plumbing group node** in the outer tree, wire the copies in series, and cook. Each copy then receives its own identity, graph snapshot, parameter sockets, and cache. Current modifier parameter values are copied to the group nodes when this conversion happens. Detaching a placement's Geometry input leaves it waiting for a connection. An untouched single-node modifier retains the 0.1.6 stack behavior. Flat groups saved by older add-ons still migrate in place on the next cook or reload.

Existing MiniHou modifier groups dropped into another GN tree are recognized and converted to private wired placements on the next cook. The original modifier stays independent when it still uses the source group. This also repairs the previously inert arrangement `Group Input -> MiniHou -> Subdivide Mesh -> Group Output`.

Object-input nodes in the MiniHou graph create object pickers directly on the MiniHou group. Pick independent Blender mesh objects there; for native instances use `@source` or another `@binding` in the MiniHou Instance node. Object inputs can also be linked through enclosing Group Inputs/reroutes. The 5.2 picker activation uses reference comparisons rather than evaluating the chosen object's geometry; unsafe self/cross-MiniHou references are rejected before cooking. On 5.1 Blender may still draw unused object sockets dimly, but values are read by the bridge. Source transforms are ignored, matching the existing source-local import convention. Native modifiers and materials are included.

For a small usable graph, select `examples/modifiers/wired_object_copies.json`: connect a mesh of points to Geometry, choose the source in **Object: asset**, then feed the result into native Subdivide/Transform nodes. Every point receives a realized copy of the source.

Each placement stores a graph snapshot in the blend. Editing a graph file does not silently replace existing snapshots. Duplicate objects/groups receive independent placement identities and caches. Ancestor groups that contain MiniHou placements become private when shared, so independent objects and nested placements cannot overwrite one another's results. Ordinary native groups without placements can remain shared.

Acyclic branches and nested ordinary groups are supported. Connected placements cook in dependency order; disconnected placements report that they are not cooked. Native modifiers before and after the GN modifier and legacy MiniHou stack modifiers can be combined. Cross-object dependencies on another MiniHou result are rejected to prevent feedback.

## Supported data and parameters

The bridge accepts mesh geometry. Instances are realized at the input boundary. Remaining curve, point-cloud, volume, grease-pencil, or instance components are rejected instead of silently treated as mesh. Native object instances are supported through contained @binding pickers. Add native Realize Instances downstream when a mesh-only operation requires it. Arbitrary object names are rejected: use bindings.

Named numeric attributes on POINT, CORNER and FACE domains travel as engine point, vertex and primitive attributes. Supported reader types include float, float2/vector, colors, integer, boolean, int-pair and quaternion data. Booleans become integer flags; byte colors become float colors. Render UVs use `uv`; render colors use `Cd` and optional `Alpha`. Material slots and face material indices are preserved. Loose edges become open two-point primitives; their newly added corner/primitive attribute values are zero. Authored custom normals are read as corner `N`; preservation through a particular engine operation depends on that operation.

There is no engine edge domain: edge attributes are skipped. String/matrix attributes and internal dot-prefixed attributes are skipped. Vertex groups above 250,000 points are skipped; use Store Named Attribute upstream for those. **Anonymous attributes do not cross this boundary**: store them by name upstream and read that name downstream. Exact Blender storage types/UI metadata are not guaranteed to survive numeric transport.

Parameters support unlinked literal values, Value constants, reroutes, and Group Input bindings in value mode. Modifier attribute mode and arbitrary computed expressions/fields connected to MiniHou parameter sockets are rejected. Native nodes can still evaluate fields while building the mesh before/between/after MiniHou groups.

## Staging limits

This is not a synchronous Python geometry-node evaluator. Interactive changes propagate through a main-thread timer. Cook explicitly before a still render or save when the latest result matters; animation/background render automation must cook each frame before rendering. The bridge pauses updates during rendering and in edit mode. It does not mutate geometry from a depsgraph callback.

Native simulation/repeat/foreach/bake zones in bridged trees and preceding GN modifiers are rejected. Use MiniHou's external repeat-loop graph support where appropriate. RBD works with static wired mesh inputs and independent placement worlds; cook explicitly after changing frame. Animated objects/upstream node trees are rejected for RBD. The RBD Solver itself cannot be inside an external repeat body; place it before or after the block. Self Object and dependencies that can feed the destination or MiniHou caches back into the probe are rejected. Local editable mesh objects/node groups are required; linked library placements are unsupported. Muted MiniHou placements or containing groups are rejected; disconnect them instead.

Missing-engine or cook failures retain the last valid result and show an error status; dependent placements wait. Before a first successful cook the result is empty and honestly marked waiting. Saved caches remain displayable without the engine, but do not represent newly edited upstream geometry until a successful cook.

Input probing evaluates a disposable native subgraph for every connected placement on each refresh. Content hashes suppress unchanged engine cooks and mesh writes, but do not eliminate that Blender evaluation cost. Large graphs with many placements can be expensive. Scripted users can unregister only `minihou_link.modifiers._timer` through `bpy.app.timers.unregister` and cook explicitly; keep the addon's render/lifecycle handlers registered.

## Why this bridge exists

Blender's [GeometryNodeCustomGroup API](https://docs.blender.org/api/5.0/bpy.types.GeometryNodeCustomGroup.html) provides a node-tree wrapper and UI/update callbacks, not a Python geometry execution callback. The bridge uses evaluated objects and [Object.to_mesh](https://docs.blender.org/api/5.0/bpy.types.Object.html) with `preserve_all_data_layers=True`, and inspects [GeometrySet components](https://docs.blender.org/api/5.0/bpy.types.GeometrySet.html) before mesh extraction. These APIs were checked against installed Blender 5.2.0 LTS. [Fields require a geometry/domain evaluation context](https://docs.blender.org/manual/en/latest/modeling/geometry_nodes/fields.html), so a linked socket's default value cannot be used as its computed value.

Engine contracts are covered by `tests/test_geonodes_engine.py`; actual Blender socket evaluation, independent placements, nesting/branching and lifecycle behavior are covered by `tests/test_blender_geonodes.py`. The engine tests model native array changes explicitly and are not a substitute for the Blender suite.
