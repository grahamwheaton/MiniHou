# USD (Universal Scene Description) export and import

The graph stays JSON: it is a recipe (nodes, parameters, wires, wrangle code) that you diff, edit by hand and let an AI edit.
USD carries the **result**, so Houdini, Blender, Maya, Omniverse, Unreal and usdview can read what MiniHou made, and MiniHou can
read what they made. It needs the optional `usd-core` package (`pip install usd-core`); nothing else changes without it, and the
USD nodes and buttons fail with that instruction only when used.

## Export

* **Editor**: File menu > *Export USD...* (the selected node if exactly one is selected, else the display output).
* **API**: `POST /api/export_usd {path, node?, overwrite?, graph?}` (path inside the workspace root, `.usd` / `.usda` / `.usdc`).
* **MCP**: `export_usd`.
* **Python**: `Session.export_usd(path)`, or `usd_io.write_stage(path, {name: geometry})`.

The export always cooks at **full size**, whatever the proxy level or the Blender cap, and never uses the viewport's reduction. The
display output and every extra output (the `output` nodes that publish their own Blender object) become sibling prims. The file is
written beside the target and swapped in, so a failure never leaves half a file, and an existing file is only replaced with
`overwrite`.

### What is in the file

`/MiniHou` (the default prim; Z up, metres) holds one `Xform` per output with up to three children, because USD cannot mix these in
one prim:

| MiniHou | USD |
|---|---|
| closed faces with 3+ corners | `Mesh` `mesh`: `subdivisionScheme = none`, right handed (counter-clockwise from outside, as in MiniHou and Blender) |
| open primitives (curves) | `BasisCurves` `curves`, linear, not periodic; points expanded per curve (USD curves have no shared point table) |
| points no primitive uses | `Points` `points` |
| points that place Blender objects (`instance` node) | `PointInstancer` `instances`: protoIndices, orientations (quaternions from `rot`), scales; prototypes are empty Xforms named after the objects (`minihou:blender_object`) |

Closed primitives with fewer than 3 corners and open ones with fewer than 2 cannot be USD prims. They are left out and counted
(`customData minihou:dropped`, and in the export's report); the points they used are written as loose points, so the point table stays whole.

**Attributes** are primvars: point attributes `vertex`, corner (vertex) attributes `faceVarying` (`vertex` on curves), primitive
attributes `uniform`. `N` is written as `normals`, `Cd` as `displayColor`, `Alpha` as `displayOpacity`, `uv` as `primvars:st`, so other
tools pick them up; everything else keeps its name (made a valid USD identifier; the original name and MiniHou table are stored on the
primvar). More than 4 components use `elementSize`. **Groups** (`group_*`, `edge_group_*`) are int primvars, and prim and point groups
are also written as face / point `GeomSubset`s, which is what other tools look for. **Detail attributes** are JSON in the Xform's
`customData`; the **graph** is in the layer's `customLayerData` (`minihou.graph`).

**Round trip**: every prim also carries `primvars:minihou:point_index` and (mesh, curves) `primvars:minihou:prim_index`, so reading a
MiniHou file back rebuilds the exact point table and primitive order, including points shared between a mesh and a curve. Tested:
points, positions, topology, open/closed flags, every attribute in every table (dtype and shape), groups, edge groups and detail come
back identical.

## Import

The **USD Import** node (`usd_import`, in the Create group) reads any USD file:

* meshes (indexed or not, any orientation, any depth of parent transforms, mirrored transforms flip the winding back), curves
  (cubic curves come in as their control polygon), points and point instancers (as `instance` / `rot` / `scale`, with the
  prototype names in the detail `instance_objects`), merged into one geometry in world space;
* primvars become attributes (`st` as `uv`, `displayColor` as `Cd`, `displayOpacity` as `Alpha`, normals as `N`, the rest by name);
  indexed primvars are flattened; GeomSubsets become groups when the file has no group primvar of that name;
* `up_axis` (z: turn a Y-up file to Z-up, the default; keep) and `apply_units` (scale by an *authored* `metersPerUnit`; an unauthored
  one is taken as metres, although USD's own fallback is centimetres);
* `prim_path` to read part of a stage (only the points that were read come in); `time` = frame reads the file's animation at the current frame (the default reads the un-animated values, or the first sample of a file that has only samples);
* guide and proxy purposes, and prim types it cannot use (cubes, spheres, ...), are skipped and listed in the detail
  attribute `usd_skipped`; a point instancer's children (its prototypes) are never imported as scene geometry; a prim with no points at the read time is noted in `usd_notes`;
* a file with several MiniHou outputs comes back as the outputs joined in order (each keeps its own points and primitives);
* the cache key includes the file's modification time and size, so saving the file in another tool recooks the node.

`usd_io.read_graph(path)` returns the graph stored in a MiniHou file, if any.

## Measured and checked

* 1M quads (1,002,001 points): write 2.0 s, read 1.2 s, 20 MB as `.usdc` (Linux sandbox; bulk copies through numpy, no per-element loops).
* Structure is checked in the tests with `UsdGeom` validation (mesh topology, primvar sizes against their interpolation, subset
  families, extents). The `usd-core` wheel ships no `usdchecker`, so a full compliance check and opening the files in Houdini, Blender
  or Omniverse were **not** done here.

## Not done

* Instancer prototypes are empty Xforms: the objects' geometry lives in Blender, so another tool sees the placements and has to
  point the prototypes at its own assets. Imported instancers become `instance` / `rot` / `scale` points (their prototypes' geometry
  is not read); an instancer under a sheared transform loses the shear on the instance rotations.
* One frame per file (no time samples written); `time` on import reads them.
* Edge groups are kept as primvars only (USD has edge subsets in newer versions, but few tools read them).
* Cameras, lights and skeletons are not read or written. Materials are carried by name and binding, and shading networks by
  reference, not authored: see [MATERIALS.md](MATERIALS.md).
* A disk cache of cooked results as USD layers (reopen a huge scene instantly) is a possible next step, not built.
