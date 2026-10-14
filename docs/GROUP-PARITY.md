# Group tools: supported parity and review guide

Source base: `f33753aa57fbf1e84076c663610f04494e9bf9e1` (`origin/main`, 2026-10-14). This is a working mesh/curve implementation of the following group tools, with the precise limits below. It does not claim complete Houdini SOP parity.

## Domain and behavior matrix

P = points; F = primitives; E = derived undirected edges; V = primitive vertices/corners. A listed domain has a working implementation; the behavior column qualifies its parity.

| Editor tool / node key | Domains | Implemented behavior | Differences and unsupported behavior |
| --- | --- | --- | --- |
| Group / Group Create `group_create` | P F E V | Pattern/expression selection; Initial Merge replace/union/intersect/subtract; enabled Base Group; closed-polygon/open-curve type filtering for F; box/ellipsoid bounds and optional bounding-object AABB; normal direction, both facing, backface position; random percentage and attribute seeds; edge length/dihedral/open-border filters; boundary output. | Legacy filter sections intersect, including edges, preserving old graphs; Houdini can add edge matches independently. Simple bounding-box mode tests primitive centroids; filter mode tests constituent points (any/all via Partial). Bounding objects supply their AABB, not solid inside/volume/VDB/hull tests. No nonplanar-poly additive mode, edge-between-edge angle/depth controls, packed/volume primitive types, or ordered membership. `ordered=True` errors explicitly. Non-edge angle/length controls error instead of being ignored. Vertex normals are a MiniHou extension. |
| Group Promote `group_promote` | P F E V to P F E V | Enabled sequential promotion rows; Auto source/name wildcards; wildcard new names; Keep Original; any/fully contained; sharing-edge primitive filter; region boundaries; polygon unshared edges and curve endpoints/all segments; numeric connectivity attributes, tolerance and point-sharing primitive expansion; integer attribute output on P/F/V. | Empty Rules retains old name/to behavior: opposite P/F source, contained promotion and source removal. New rows default to any-connected. Auto processes matching domains in P/F/E/V order. Output collisions replace in that order. Degenerate-bridge removal errors explicitly. Edge integer output is unavailable. |
| Group Delete `group_delete` | P F E V / Any | Enabled wildcard rows; Delete Unused removes empty group definitions across domains. | Deletes group metadata, never geometry. No group order metadata exists. |
| Group Combine `group_combine` | P F E V / Auto | Enabled result rows; infer unique source type; base assignment/inversion; nested enabled boolean operations with inverse operands; union/intersection/subtract/XOR; later rows read earlier outputs. | Ambiguous Auto requires explicit type. Every result replaces that domain's existing group. No general Houdini expression language. |
| Group Rename `group_rename` | P F E V / Any | Sequential enabled wildcard rules; captured `*`/`?` text substituted into `*` in new name; overwrite/skip/error collisions. | General Houdini string-edit pattern language is not implemented. |
| Group Copy `group_copy` | P F E V | Second source input; per-domain name patterns/prefixes; element-index mapping; scalar integer matching attributes for P/F/V; overwrite/skip/suffix conflicts; empty groups. | No native string attributes. Edge copy maps derived edge numbers, so use equivalent topology. No stored ordered membership. |
| Group Transfer `group_transfer` | P F E | Second source input; per-domain patterns/prefixes; nearest source membership; inclusive distance threshold; conflicts; empty groups. | P uses positions, F centroids and E midpoints. It does not reproduce Houdini closest-surface primitive/edge correspondence. A closer unselected source blocks a farther selected source. No V mode, consistent with the documented transfer domains. |
| Group Invert `group_invert` | P F E V / Auto | Wildcard groups; complement membership; optional captured new names. | Missing wildcard matches do nothing; Auto may affect more than one domain. |
| Group Expression `group_expression` | P F V | Enabled rows with group name, expression and merge operation; named group reuse; frame-aware caching. | MiniHou expression subset, not arbitrary VEX. Rows run sequentially, so later rows see earlier groups; Houdini compiles its expressions into one pass. No bindings/evaluation-node-path/input ports. No E expression mode. |
| Group by Range `group_range` | P F | Base group; inclusive absolute Start/End, relative offsets, Start+Length, equal partitions; Select-of-Every/offset; invert/merge; disconnected regions. | One range per node; chain nodes for more ranges. No connectivity-attribute/collision/region-ID partition constraints. |
| Group Expand `group_expand` | P F E V | Grow/contract; signed ring count; flood fill; primitive point/edge adjacency. E adjacency shares endpoints. V adjacency shares a point or neighboring corner along its primitive. | Explicit domain rather than Auto; no normal spread, connectivity attribute or collision constraints. Empty result name replaces a plain base group (legacy behavior), rather than Houdini's no-output convention. |
| Group Find Path `group_find_path` | P E | Ordered element tokens or pairs; deterministic shortest geometric-distance paths through topology; same-domain collision group. | Named groups use element-number order because ordered groups are unavailable. No UV-constrained, loop/ring, turn preference or cross-domain collision mode. Disconnected paths error. |
| Group from Attribute Boundary `group_from_attribute_boundary` | P F E V output | Numeric primitive/point/vertex discontinuities and tolerance; base-region perimeter; polygon unshared and optional curve edges; all pairs on nonmanifold edges. | No degenerate-bridge removal. V output selects both corners on each incident primitive side. |
| Groups from Name `groups_from_name` | P F V | Integer name codes plus detail string table; per-name groups; prefix, replace/union; ignore/force-valid name handling. | Represent names as integer `attribute` indices into `detail[attribute + "_names"]`. No native string arrays or Houdini `xn__` encoding. Invalid/out-of-range/empty name codes are omitted. |
| Group Paint `group_paint` | P F E V | Persistent selection parameter; viewport brush edits that node directly; ordinary group output; one stroke per undo step; erase, redo and graph reload. | Component IDs refer to input topology; topology-changing upstream edits can change their meaning. V brush affects coincident corners at their point position. Brush mask/falloff is the existing MiniHou painter. |
| Group by Lasso `group_lasso` | P F E V | Existing lasso selection tool plus **Use viewport selection**, stored in the node/graph. | This records membership after lassoing; it does not save a camera-dependent lasso polygon. V membership comes from selected primitive corners or all corners at selected points. |
| Group by Cd Colour `group_by_color` | P F E V | Configurable Cd attribute, target RGB, Euclidean tolerance, invert, base filter and merge; explicit missing/malformed attribute errors. | Native domain preferred, with documented averaging/fallback rules. No perceptual colour-space conversion. |
| Group by Normal `group_by_normal` | P F E V | Direction, angle in degrees, both facing, invert/base/merge; normalized supplied N or computed normals. | Open curves and degenerate faces have no computed facing normal; zero normals do not match. |
| Group by Proximity `group_by_proximity` | P F E V | Optional target input or position; point and actual polygon/curve surface searches; distance, invert/base/merge; safe empty source; no mandatory SciPy. | P/V use point positions; F centroid; E midpoint. A large element can intersect the target while its representative is outside the threshold. Auto falls back to target points when no usable surface exists. Float32 threshold tolerance is 1e-6 model units. |

The helper fallback/normal/position rules are specified in [group-helper-semantics.md](group-helper-semantics.md). Extra engine notes are in [group-parity-engine-notes.md](group-parity-engine-notes.md).

## Editor workflow

All tools appear in the Groups shelf, Group pie pages and Add Node search. Expression creates its own node; Paint and Lasso are active tools. Add/Remove, Move Up/Down and Enabled controls edit structured rule rows. Combine has the same controls for each boolean operation. Changes are graph-owned, undoable and saved/reloaded; no JSON entry is required. Legacy custom node parameter types keep their existing format; `Rules` is an optional SDK declaration.

Group fields provide domain-aware input-group dropdowns and **Select**. Select previews the input, so it also works before Blast removes the output geometry. Selection resolution is read-only and retains the existing 100,000-component limit. Selecting V membership highlights its owning points and retains the exact corner IDs for Select/capture and add/subtract round trips until the viewport selection changes; it is not a separate corner-picking mode. Use numeric corner IDs, expressions, the spreadsheet or promotions for independently specified coincident corners. Lasso records the current point/face selection as corners. V paint colouring averages coincident corner colours for display.

For Paint, create/select Group Paint, press **Paint membership**, then brush the surface. A narrow viewport enlarges automatically so the paint controls leave drawing space. Escape exits paint; the viewport maximize button restores the network/inspector. For Lasso, press **Lasso membership**, draw the selection, then **Use viewport selection**. The membership field remains editable and is also a readable group/range pattern.

Length fields are explicit: Create box center/size and edge lengths, Transfer threshold, and Proximity position/distance use the frontend's length units. RGB components, directions, angles and percentages do not rescale.

## Expression subset

An expression is one scalar/boolean expression, with no assignment, semicolon, newline, loop or arbitrary Python call. Supported inputs include `@P.x`, `@Cd.r`, `@N.z`, `@group_top`, numeric attributes, `@ptnum`, `@primnum`, `@vtxnum`, counts, `@closed`, `@Frame`, `@Time` and `@TimeInc` where meaningful on the chosen domain. Other-domain attributes follow MiniHou wrangle promotion rules. Unknown attributes/functions and unsupported syntax produce errors.

Use arithmetic `+ - * / // % **`, comparisons, `and`, `or`, `not`, and Python-style `a if condition else b`. Available whitelisted functions include `sin`, `cos`, `abs`, `min`, `max`, `clamp`, `fit`, `length`, `normalize`, `dot`, `cross`, `set`, `noise` and `rand`; the canonical list is `minihou.wrangle.FUNCTIONS`. VEX constructs such as `&&`, `||`, `ch()`, arbitrary VEX functions and geometry-writing statements are not supported.

Examples: `@P.z > 0`, `@group_red and @P.x < 1`, `dot(normalize(@N), set(0,0,1)) > 0.9`, `rand(@ptnum, seed=3) < 0.25`. Disabled rows are skipped. Frame-dependent expressions in nested rows invalidate the cache correctly.

## Storage, compatibility and Blender

P/F/V membership is integer `group_<name>` on that domain. E membership is integer `edge_group_<name>` on outgoing corner rows, reduced by endpoint pair. The new group tools preserve separate same-name groups on different domains. Legacy P/F `groups.write` callers, including Create/Range's established paths, retain their existing other-domain removal convention. Ordinary Transform/Blast point/primitive fields can resolve new edge/vertex groups; P/F same-domain precedence and legacy point-to-primitive contained conversion remain intact.

Blender receives P/F/V groups as BOOLEAN on POINT/FACE/CORNER. E groups now export as BOOLEAN EDGE named `edge_group_<name>`, including open-curve segments and nonmanifold OR membership. Use that name in Geometry Nodes **Named Attribute**. Polygon corner attributes export exactly; open curves have no Blender face corners, so their V/F attributes cannot be represented as mesh corner/face values. Blender allows only one attribute per name, so same-name P/F/V groups on multiple domains require distinct names before export. The engine does not silently merge those memberships.

Legacy `group_promote(name,to)` remains opposite-source contained P/F conversion with source removal. Rule rows are opt-in; they use explicit Auto and any-connected semantics. Existing node names and old scalar parameter formats remain loadable. No EXE or final add-on package is produced by this task; the lead builds and integrates.

## Examples and validation

Open these graphs in the editor and inspect their intermediate groups:

- [Core Create ? Promote ? Combine ? Transform / Blast](../examples/graphs/group-parity-core.json)
- [Cd / Normal / Proximity helpers](../examples/graphs/group-parity-helpers.json)
- [Copy and threshold Transfer](../examples/graphs/group-parity-copy-transfer.json)
- [Find Path and Attribute Boundary](../examples/graphs/group-parity-path-boundary.json)
- [Range, name tables, Rename and Delete](../examples/graphs/group-parity-names.json)

Run from this worktree with the bundled Python:

```powershell
& ../runtime/python312/python.exe tools/group_parity_validate.py --suite engine
& ../runtime/python312/python.exe tools/group_parity_validate.py --suite browser
& 'C:/Program Files/Blender Foundation/Blender 5.2/blender.exe' --background --factory-startup --python tools/group_blender_selftest.py
```

Final validation: **378 engine/API/schema tests, 56 actual Chromium tests and 3 real Blender 5.2 export tests passed, with zero skips**. Logs are retained locally under `build/group-parity-validation/`.

Each launcher prepends this checkout and prints `minihou.__file__`, because the bundled `_pth` otherwise imports the original repository. Browser tests use `../runtime/browsers` and run serially. Validation covers all domains, empty/frozen inputs, attributes/topology, wildcard sequential rules, source retention, boolean chains, tolerances, transfer conflicts/thresholds, open/degenerate/nonmanifold geometry, real inspector/menu/brush/lasso/undo/reload workflows, legacy group/filter/expand/output/select/viewport/measurement behavior, custom SDK/library compatibility and real Blender attribute export. This is host validation in isolated background Blender, not a claim of interactive Blender UI testing.

## Official reference pages

Reviewed against the [SideFX SOP index](https://www.sidefx.com/docs/houdini/nodes/sop/index.html) and official [Create](https://www.sidefx.com/docs/houdini/nodes/sop/groupcreate.html), [Promote](https://www.sidefx.com/docs/houdini/nodes/sop/grouppromote.html), [Delete](https://www.sidefx.com/docs/houdini/nodes/sop/groupdelete.html), [Combine](https://www.sidefx.com/docs/houdini/nodes/sop/groupcombine.html), [Copy](https://www.sidefx.com/docs/houdini/nodes/sop/groupcopy.html), [Rename](https://www.sidefx.com/docs/houdini/nodes/sop/grouprename.html), [Transfer](https://www.sidefx.com/docs/houdini/nodes/sop/grouptransfer.html), [Invert](https://www.sidefx.com/docs/houdini/nodes/sop/groupinvert.html), [Expression](https://www.sidefx.com/docs/houdini/nodes/sop/groupexpression.html), [Range](https://www.sidefx.com/docs/houdini/nodes/sop/grouprange.html), [Expand](https://www.sidefx.com/docs/houdini/nodes/sop/groupexpand.html), [Find Path](https://www.sidefx.com/docs/houdini/nodes/sop/groupfindpath.html), [Attribute Boundary](https://www.sidefx.com/docs/houdini/nodes/sop/groupfromattribboundary.html), and [Groups from Name](https://www.sidefx.com/docs/houdini/nodes/sop/groupsfromname.html) pages. The compatibility choices and limits above are MiniHou behavior, not statements that SideFX provides those same restrictions.
