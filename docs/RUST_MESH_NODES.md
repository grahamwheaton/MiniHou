# Rust mesh nodes from the Blender audit

Four modelling operations from `docs/AUDIT_BLENDER_MODELLING_TOOLS.md`: `grid_fill`, `recalc_normals`, `tris_to_quads` and
the extended `inset`. All are non-destructive and deterministic. Group conventions follow the other group nodes: `group`
selects the elements (empty = all), `output_group` / `output_group_name` write the new or changed elements to a primitive
group. Faces without a source (the grid-fill faces) use the neutral material slot and NaN corner UVs
(`docs/RUST_MATERIALS_SCHEMA.md`); all other attributes of retained faces are kept. Code: `nodes/mesh_common.rs` (shared
helpers), `grid_fill.rs`, `recalc_normals.rs`, `tris_to_quads.rs`, `inset_ext.rs` (+ `pushpull.rs` for the node spec).

## grid_fill (Blender Grid Fill, `mesh.fill_grid`)

Fills closed boundary loops with a quad grid.

| Param | Meaning |
| --- | --- |
| `group` | Boundary edges (ids, `pA-B`, edge groups). Empty: every boundary loop. |
| `span` | Grid columns = edges of the first side (>= 1). Used when `auto_span` is off. |
| `auto_span` | Choose span and start corner from the loop's corner angles and side lengths (what Blender does when Span is untouched). |
| `offset` | Rotates the start vertex (added to the automatic start). |
| `use_interp_simple` | Interpolate by edge index instead of arc length. |
| `output_group` / `_name` | New faces (default on, `grid_fill`). |

A loop of `n` edges becomes a `span x (n/2 - span)` grid. Interior points come from a Coons patch (transfinite bilinear blend of
the four sides); point attributes are blended with the bilinear weights of the four grid corners. New faces wind opposite to the
adjacent boundary faces so the surface stays consistently oriented. Errors, naming the point: odd number of edges, fewer than
4 edges, open chain, a boundary that branches or touches itself, an edge that is not a boundary edge, span too large.
Several disjoint loops are each filled. Differences from Blender: loops with holes (an island loop inside another) are not
detected as such, each selected loop is filled on its own; selecting by face group is not supported (select the edges); the
auto layout is a deterministic scoring, not Blender's exact heuristic. Corner colours / other corner attributes of new
corners are averaged from the boundary corners; corner normals `N` are recomputed.

## recalc_normals (Blender Recalculate Normals, `mesh.normals_make_consistent`)

Per connected component (across edges shared by exactly two selected faces) the winding is made consistent by flood fill, then the
component is oriented. `inside` reverses the result. Closed components use the sign of the signed volume. Open components use
Blender's farthest-vertex rule: the faces at the vertex farthest from the component centre must face away from it. If that is
ambiguous (flat surface, degenerate) the existing majority orientation (area weighted) is kept. The output group holds the faces
that were flipped. Corner and face `N` attributes of flipped faces are negated, point `N` at touched points is recomputed.
Non-orientable or non-manifold components are made consistent as far as the flood fill reaches (first assignment wins).

## tris_to_quads (Blender Tris to Quads, `mesh.tris_convert_to_quads`)

Candidate pairs are selected triangles sharing a manifold edge with consistent winding. A pair is rejected when the normals differ
by more than `face_threshold`, when the quad is concave or degenerate, or when a corner deviates from 90 degrees by more than
`shape_threshold`. Compare flags reject pairs across UV differences (`uvs`, any `uv*` corner attribute), `Cd` differences
(`vcols`), edges in the edge groups `seam` / `sharp`, and different `material_index` (`materials`). Pairs are merged greedily, best score
first (score = normal angle + worst corner deviation, ties by edge point numbers), which is deterministic. The merged quad
replaces the lower-numbered triangle, keeps its primitive attributes and takes corner attributes from the original corners of
each edge. Output group: the new quads. Not supported: Blender's separate "Compare Sharp / Seam" on non-triangle faces.

## inset (Blender Inset Faces, `mesh.inset`)

Existing params (`group`, `amount` = thickness, `mode`) and the Python-compatible behaviour are unchanged when all new options
are at their defaults. New: `depth` (inner faces along the normal), `use_outset` (negates thickness), `use_individual`
(= `mode: individual`), `use_boundary` (off: mesh border edges and their points stay), `use_even_offset` (off: no corner scaling),
`use_relative_offset` (thickness times local boundary edge length), `use_edge_rail` (boundary points slide along an existing
edge inside the region; falls back to the miter, ignored when individual), `use_interpolate` (off zeroes new point attributes),
output groups `output_group` (ring faces) and `output_inner_group` (inner faces), plus the old `extrudeFront` / `extrudeSide`.
Any of `use_boundary` off, `use_even_offset` off, relative, rail or `use_interpolate` off selects the extended implementation
(`inset_ext.rs`), which does not run the planar-region validation or collapse detection of the original: very large thickness can
produce overlaps.
