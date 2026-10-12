# Output groups on the modelling nodes

Houdini's modelling nodes end with an **Output Groups** section: a checkbox and a name for each group the node can make from what it
just built (PolyBevel: Point Fillet Polygons, Offset Edges...; PolyExtrude: Front, Back, Side). They are what lets one node select what
the previous one made: bevel, then extrude the new strips, then crease the offset edges.

Here every one is a pair of parameters, `outgrp_<key>` (the checkbox) and `outgrp_<key>_name` (the name), shown in the editor as an
**Output Groups** section with the checkbox and the name side by side. A group that is off is not written, a group that is on exists
even when it is empty (so a downstream group field never fails because nothing happened to be bevelled), and a name is any valid group
name. Domains: **prim** and **point** groups are the usual `group_<name>` attributes; **edge** groups are the `edge_group_<name>`
attributes of `minihou/edges.py`, readable by every node that takes an edge selection (`crease`, `polybridge`, `dissolve_edges`,
`edge_group`, and `polybevel` with `group_type = edges`).

## What is there

The fracture tools (`voronoi_*`, `boolean_fracture`, `convex_decomposition`, `assemble`, `glue_cluster`, `rbd_material_fracture`) and the pie menu's new nodes follow Houdini's menu entries but their group sets are my own design, since I have no panel screenshots for them: tell me what Houdini offers and I will match it.
PolyBevel, Revolve, Subdivide, PolyExpand2D and Group Create are taken from the Houdini parameter panels you shared (labels, names and
defaults). The other nodes follow what Houdini's
nodes of the same name offer **as far as I remember them**, adapted to what each node here actually builds; where I was not sure, the group
is named after what it holds. Check them against your Houdini and tell me which differ: renaming or adding one is a line each.

| Node | Output groups (default name, domain) | Default |
|---|---|---|
| `polybevel` | **Point Fillet Polygons** `pointfilletpolys` (prim), **Point Fillet Edges** `pointfilletedges` (edge), **Edge Fillet Polygons** `edgefilletpolys` (prim), **Offset Edges** `offsetedges` (edge), **Offset Points** `offsetpoints` (point), **Merged Points** `mergedpoints` (point) | all off. `mergedpoints` is always empty: this bevel limits an offset where Houdini merges points. The older `output_group` (`bevel`, every new face) stays |
| `polyextrude` | **Front** `extrudeFront`, **Back** `extrudeBack`, **Side** `extrudeSide` (prim) | front and side on (they were always made), back off (needs `output_back`) |
| `mirror` | **Mirrored Primitives** `mirrored`, **Mirrored Points** `mirroredpoints`, **Seam Points** `seampoints` | off |
| `boolean` | **Faces from A** `from_a`, **Faces from B** `from_b` (prim), **Intersection Edges** `intersection` (edge), **Intersection Points** `intersectionpoints` (point) | from_a / from_b on (as before), the rest off |
| `clip` | **Above** `above`, **Below** `below`, **Cut Polygons** `cutpolys` (prim), **Cut Points** `cutpoints` (point), **Cut Edges** `cutedges` (edge: the opening a cap would fill) | `above` on (as before), the rest off |
| `fuse` | **Fused Points** `fusedpoints`, **Unique Points** `uniquepoints` | off |
| `sweep` | **Sides** `sweepsides`, **Start Cap** `startcap`, **End Cap** `endcap` (prim) | off |
| `skin` | **Surface** `skinsurface`, **Cap Polygons** `skincap` | off |
| `bridge`, `polybridge` | **Bridge Polygons** `bridgepolys` | off |
| `revolve` | **End Caps Group** `endcaps` (prim; from your Revolve panel). New **End Caps** option closes both ends of a partial revolve; **Start Angle** added | off |
| `subdivide` | **Generate Resulting Creases** `creases` (edge; the panel's "New Group"). New **Creases** field (an edge selection to crease), **Override Crease Weight** and **Crease Weight** | off |
| `polyexpand2d` | **Inside Group** `inside`, **Outside Group** `outside` (prim), and the point attributes **Edge Distance** `edgedist` / **Edge Speed** `edgespeed` (each its own checkbox + name). New **mode** `inside_outside` makes the outside and the inside loop (**Output Inside / Outside**) and **Keep Input** keeps the original | off |
| `voronoi_fracture` | **Inside Faces** `inside` (prim: the caps; the older `inside_group` still names or removes it), **Outside Faces** `outside` (prim: the input's own surface), **Cut Points** `cutpoints` (point), **Cut Edges** `cutedges` (edge: where inside meets outside) | inside on (as before), the rest off |
| `voronoi_split` | **Cut Points** `cutpoints`, **Cut Edges** `cutedges` (the open border each cut left; nothing is capped) | off |
| `voronoi_fracture_points` | **Interior Points** `interior`, **Surface Points** `surface`, **Clustered Points** `clustered`, **Extra Input Points** `extrapoints` (all point) | off |
| `voronoi_adjacency` | **Connected Pieces** `connected`, **Isolated Pieces** `isolated` (point: the piece centres), **Strong Links** `stronglinks` (prim: links whose contact area is at least the "strong area" share of the biggest) | off |
| `boolean_fracture` | **Inside Faces** `inside`, **Outside Faces** `outside`, **Cutter Pieces** `cutterpieces`, **Remainder Piece** `remainder` (prim), **Cut Points** `cutpoints`, **Cut Edges** `cutedges` | inside on, the rest off |
| `convex_decomposition` | **Inside Faces** `inside`, **Outside Faces** `outside`, **Cut Points** `cutpoints`, **Cut Edges** `cutedges` | inside on, the rest off |
| `assemble` | **Split Points** `splitpoints` (every copy after the first of a point that pieces shared), **Shared Points** `sharedpoints`. The `piece_groups` option also makes a prim group per piece | off |
| `glue_cluster` | pieces output: **Glued Pieces** `glued`, **Loose Pieces** `loose`, **Interface Faces** `interface` (prim); constraints output: **Internal Links** `internal`, **External Links** `external`, **Weak Links** `weaklinks` (prim) | off |
| `rbd_material_fracture` | the fracture's four: `inside`, `outside`, `cutpoints`, `cutedges` | inside on |
| `polysplit` | **New Points** `splitpoints`, **New Edges** `splitedges` | off |
| `edge_collapse` | **Collapsed Points** `collapsedpoints` | off |
| `edge_flip` | **Flipped Edges** `flippededges` | off |

`group_create` now has Houdini's **filter sections** (method `filters`): the base group, **Keep in Bounding Regions** (box or sphere, include partial),
**Keep by Normals** (direction, spread angle, opposite, backface from a position), **Include by Edges** (min/max angle, min/max length, unshared edges) and
**Keep by Random Chance** (percent, seed, seed attribute) each switched on separately, and an element must pass every enabled one. It also makes
**edge groups** (`domain = edges`), keeps **Initial Merge** (`combine`), and **Create Boundary Groups** (`boundary_groups`) adds `<name>_boundary`, the edge
group around the selection. The older single methods (pattern, bounding box, normal, random, boundary) are unchanged.

Already had their own group parameter, unchanged: `polyfill` (`fill_group`), `polybevel`
(`output_group`), `group_create`, `edge_group`.

## PolyBevel's Group Type

`polybevel` now reads its `group` like Houdini's: **Group Type** `points` (the old behaviour: edges with both ends in the point group),
`edges` (an edge group, edge numbers or `pA-B` pairs: for example the `offsetedges` of an earlier bevel), or `primitives` (every interior
edge of those primitives). A group selects exactly those edges and ignores `min_angle`; an empty group still means "every sharp edge".

## Not done (no output groups here, or not applicable)

`divide`, `dissolve`, `dissolve_edges`, `facet`, `reverse`, `delete`, `blast`, `remesh`, `copy_to_points` and the sources have no
output groups in what I know of Houdini or build nothing a group could name (tell me if one of them does). PolyBevel's other
Houdini parameters (fillet shape, divisions, convexity, profile, collisions and limits) are not part of this change: the bevel is still a
single flat chamfer.

## Adding them to your own nodes

```python
from minihou.outgroups import OutGroup, OutGroups

OUT = OutGroups(OutGroup("rim", "Rim", "rim", "edge"), OutGroup("top", "Top Polygons", "toppolys", "prim", default=True))

@node("mynode", inputs=["geo"], params=[Float("amount", 1.0), *OUT.params()])
def mynode(geo, amount, **og):
    ...
    out = OUT.write(out, og, "top", top_mask)                 # a bool array (or a function returning one) over the group's domain
    return OUT.write(out, og, "rim", lambda: outgroups.edge_mask(out, point_pairs))
```
