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

PolyBevel is taken exactly from the Houdini parameter panel (labels, names and "all off by default"). The other nodes follow what Houdini's
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

Already had their own group parameter, unchanged: `polyfill` (`fill_group`), `voronoi_fracture` (`inside_group`), `polybevel`
(`output_group`), `group_create`, `edge_group`.

## PolyBevel's Group Type

`polybevel` now reads its `group` like Houdini's: **Group Type** `points` (the old behaviour: edges with both ends in the point group),
`edges` (an edge group, edge numbers or `pA-B` pairs: for example the `offsetedges` of an earlier bevel), or `primitives` (every interior
edge of those primitives). A group selects exactly those edges and ignores `min_angle`; an empty group still means "every sharp edge".

## Not done (no output groups here, or not applicable)

`divide`, `subdivide`, `dissolve`, `dissolve_edges`, `polyexpand2d`, `revolve`, `facet`, `reverse`, `delete`, `blast`, `remesh`,
`copy_to_points` and the sources have no output groups in what I know of Houdini or build nothing a group could name. PolyBevel's other
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
