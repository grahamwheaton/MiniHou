# Native mesh operator inventory

Audited against Blender 4.3.2. Native-backend means allowlisted and present in this runtime, not universal behavioural verification. Related tools and selection workflows do not claim parity. Interactive and remaining gaps are intentionally explicit.

| Blender mesh operator | Coverage | MiniHou route / remaining work |
| --- | --- | --- |
| `attribute_set` | remaining-gap | Not yet exposed by the mesh backend |
| `average_normals` | remaining-gap | Not yet exposed by the mesh backend |
| `beautify_fill` | native-backend | blender_mesh: beautify_fill |
| `bevel` | native-backend | blender_mesh: bevel |
| `bisect` | native-backend | blender_mesh: bisect |
| `blend_from_shape` | remaining-gap | Not yet exposed by the mesh backend |
| `bridge_edge_loops` | native-backend | blender_mesh: bridge_edge_loops |
| `colors_reverse` | remaining-gap | Not yet exposed by the mesh backend |
| `colors_rotate` | remaining-gap | Not yet exposed by the mesh backend |
| `convex_hull` | native-backend | blender_mesh: convex_hull |
| `customdata_custom_splitnormals_add` | remaining-gap | Not yet exposed by the mesh backend |
| `customdata_custom_splitnormals_clear` | remaining-gap | Not yet exposed by the mesh backend |
| `customdata_mask_clear` | remaining-gap | Not yet exposed by the mesh backend |
| `customdata_skin_add` | remaining-gap | Not yet exposed by the mesh backend |
| `customdata_skin_clear` | remaining-gap | Not yet exposed by the mesh backend |
| `decimate` | native-backend | blender_mesh: decimate |
| `delete` | native-backend | blender_mesh: delete |
| `delete_edgeloop` | native-backend | blender_mesh: delete_edgeloop |
| `delete_loose` | native-backend | blender_mesh: delete_loose |
| `dissolve_degenerate` | native-backend | blender_mesh: dissolve_degenerate |
| `dissolve_edges` | native-backend | blender_mesh: dissolve_edges |
| `dissolve_faces` | native-backend | blender_mesh: dissolve_faces |
| `dissolve_limited` | native-backend | blender_mesh: dissolve_limited |
| `dissolve_mode` | related-tool | dissolve_edges / limited_dissolve |
| `dissolve_verts` | native-backend | blender_mesh: dissolve_verts |
| `dupli_extrude_cursor` | interactive-gap | Needs dedicated viewport interaction/context |
| `duplicate` | related-tool | copy_transform |
| `duplicate_move` | related-tool | copy_transform |
| `edge_collapse` | native-backend | blender_mesh: edge_collapse |
| `edge_face_add` | native-backend | blender_mesh: edge_face_add |
| `edge_rotate` | native-backend | blender_mesh: edge_rotate |
| `edge_split` | native-backend | blender_mesh: edge_split |
| `edgering_select` | selection-workflow | Viewport selection / groups; not full operator parity |
| `edges_select_sharp` | selection-workflow | Viewport selection / groups; not full operator parity |
| `extrude_context` | related-tool | polyextrude |
| `extrude_context_move` | related-tool | polyextrude |
| `extrude_edges_indiv` | related-tool | blender_mesh: extrude_edges_move |
| `extrude_edges_move` | native-backend | blender_mesh: extrude_edges_move |
| `extrude_faces_indiv` | related-tool | blender_mesh: extrude_faces_move |
| `extrude_faces_move` | native-backend | blender_mesh: extrude_faces_move |
| `extrude_manifold` | native-backend | blender_mesh: extrude_manifold |
| `extrude_region` | related-tool | native_extrude |
| `extrude_region_move` | native-backend | blender_mesh: extrude_region_move |
| `extrude_region_shrink_fatten` | related-tool | polyextrude |
| `extrude_repeat` | native-backend | blender_mesh: extrude_repeat |
| `extrude_vertices_move` | native-backend | blender_mesh: extrude_vertices_move |
| `extrude_verts_indiv` | related-tool | blender_mesh: extrude_vertices_move |
| `face_make_planar` | native-backend | blender_mesh: face_make_planar |
| `face_set_extract` | remaining-gap | Not yet exposed by the mesh backend |
| `face_split_by_edges` | native-backend | blender_mesh: face_split_by_edges |
| `faces_mirror_uv` | remaining-gap | Not yet exposed by the mesh backend |
| `faces_select_linked_flat` | selection-workflow | Viewport selection / groups; not full operator parity |
| `faces_shade_flat` | native-backend | blender_mesh: faces_shade_flat |
| `faces_shade_smooth` | native-backend | blender_mesh: faces_shade_smooth |
| `fill` | native-backend | blender_mesh: fill |
| `fill_grid` | native-backend | blender_mesh: fill_grid |
| `fill_holes` | native-backend | blender_mesh: fill_holes |
| `flip_normals` | native-backend | blender_mesh: flip_normals |
| `flip_quad_tessellation` | remaining-gap | Not yet exposed by the mesh backend |
| `hide` | selection-workflow | Viewport selection / groups; not full operator parity |
| `inset` | native-backend | blender_mesh: inset |
| `intersect` | native-backend | blender_mesh: intersect |
| `intersect_boolean` | native-backend | blender_mesh: intersect_boolean |
| `knife_project` | interactive-gap | Needs dedicated viewport interaction/context |
| `knife_tool` | interactive-gap | Needs dedicated viewport interaction/context |
| `loop_multi_select` | selection-workflow | Viewport selection / groups; not full operator parity |
| `loop_select` | selection-workflow | Viewport selection / groups; not full operator parity |
| `loop_to_region` | selection-workflow | Viewport selection / groups; not full operator parity |
| `loopcut` | interactive-gap | Needs dedicated viewport interaction/context |
| `loopcut_slide` | interactive-gap | Needs dedicated viewport interaction/context |
| `mark_freestyle_edge` | remaining-gap | Not yet exposed by the mesh backend |
| `mark_freestyle_face` | remaining-gap | Not yet exposed by the mesh backend |
| `mark_seam` | native-backend | blender_mesh: mark_seam |
| `mark_sharp` | native-backend | blender_mesh: mark_sharp |
| `merge` | native-backend | blender_mesh: merge |
| `merge_normals` | remaining-gap | Not yet exposed by the mesh backend |
| `mod_weighted_strength` | remaining-gap | Not yet exposed by the mesh backend |
| `normals_make_consistent` | native-backend | blender_mesh: normals_make_consistent |
| `normals_tools` | remaining-gap | Not yet exposed by the mesh backend |
| `offset_edge_loops` | native-backend | blender_mesh: offset_edge_loops |
| `offset_edge_loops_slide` | related-tool | blender_mesh: offset_edge_loops (slide is not equivalent) |
| `paint_mask_extract` | remaining-gap | Not yet exposed by the mesh backend |
| `paint_mask_slice` | remaining-gap | Not yet exposed by the mesh backend |
| `point_normals` | remaining-gap | Not yet exposed by the mesh backend |
| `poke` | native-backend | blender_mesh: poke |
| `polybuild_delete_at_cursor` | interactive-gap | Needs dedicated viewport interaction/context |
| `polybuild_dissolve_at_cursor` | interactive-gap | Needs dedicated viewport interaction/context |
| `polybuild_extrude_at_cursor_move` | interactive-gap | Needs dedicated viewport interaction/context |
| `polybuild_face_at_cursor` | interactive-gap | Needs dedicated viewport interaction/context |
| `polybuild_face_at_cursor_move` | interactive-gap | Needs dedicated viewport interaction/context |
| `polybuild_split_at_cursor` | interactive-gap | Needs dedicated viewport interaction/context |
| `polybuild_split_at_cursor_move` | interactive-gap | Needs dedicated viewport interaction/context |
| `polybuild_transform_at_cursor` | interactive-gap | Needs dedicated viewport interaction/context |
| `polybuild_transform_at_cursor_move` | interactive-gap | Needs dedicated viewport interaction/context |
| `primitive_circle_add` | native-backend | blender_mesh: primitive_circle_add |
| `primitive_cone_add` | native-backend | blender_mesh: primitive_cone_add |
| `primitive_cube_add` | native-backend | blender_mesh: primitive_cube_add |
| `primitive_cube_add_gizmo` | remaining-gap | Not yet exposed by the mesh backend |
| `primitive_cylinder_add` | native-backend | blender_mesh: primitive_cylinder_add |
| `primitive_grid_add` | native-backend | blender_mesh: primitive_grid_add |
| `primitive_ico_sphere_add` | native-backend | blender_mesh: primitive_ico_sphere_add |
| `primitive_monkey_add` | native-backend | blender_mesh: primitive_monkey_add |
| `primitive_plane_add` | native-backend | blender_mesh: primitive_plane_add |
| `primitive_torus_add` | native-backend | blender_mesh: primitive_torus_add |
| `primitive_uv_sphere_add` | native-backend | blender_mesh: primitive_uv_sphere_add |
| `quads_convert_to_tris` | native-backend | blender_mesh: quads_convert_to_tris |
| `region_to_loop` | selection-workflow | Viewport selection / groups; not full operator parity |
| `remove_doubles` | native-backend | blender_mesh: remove_doubles |
| `reveal` | selection-workflow | Viewport selection / groups; not full operator parity |
| `rip` | interactive-gap | Needs dedicated viewport interaction/context |
| `rip_edge` | interactive-gap | Needs dedicated viewport interaction/context |
| `rip_edge_move` | interactive-gap | Needs dedicated viewport interaction/context |
| `rip_move` | interactive-gap | Needs dedicated viewport interaction/context |
| `screw` | native-backend | blender_mesh: screw |
| `select_all` | selection-workflow | Viewport selection / groups; not full operator parity |
| `select_axis` | selection-workflow | Viewport selection / groups; not full operator parity |
| `select_by_attribute` | selection-workflow | Viewport selection / groups; not full operator parity |
| `select_face_by_sides` | selection-workflow | Viewport selection / groups; not full operator parity |
| `select_interior_faces` | selection-workflow | Viewport selection / groups; not full operator parity |
| `select_less` | selection-workflow | Viewport selection / groups; not full operator parity |
| `select_linked` | selection-workflow | Viewport selection / groups; not full operator parity |
| `select_linked_pick` | selection-workflow | Viewport selection / groups; not full operator parity |
| `select_loose` | selection-workflow | Viewport selection / groups; not full operator parity |
| `select_mirror` | selection-workflow | Viewport selection / groups; not full operator parity |
| `select_mode` | selection-workflow | Viewport selection / groups; not full operator parity |
| `select_more` | selection-workflow | Viewport selection / groups; not full operator parity |
| `select_next_item` | selection-workflow | Viewport selection / groups; not full operator parity |
| `select_non_manifold` | selection-workflow | Viewport selection / groups; not full operator parity |
| `select_nth` | selection-workflow | Viewport selection / groups; not full operator parity |
| `select_prev_item` | selection-workflow | Viewport selection / groups; not full operator parity |
| `select_random` | selection-workflow | Viewport selection / groups; not full operator parity |
| `select_similar` | selection-workflow | Viewport selection / groups; not full operator parity |
| `select_similar_region` | selection-workflow | Viewport selection / groups; not full operator parity |
| `select_ungrouped` | selection-workflow | Viewport selection / groups; not full operator parity |
| `separate` | related-tool | blast (split graph branches) |
| `set_normals_from_faces` | remaining-gap | Not yet exposed by the mesh backend |
| `set_sharpness_by_angle` | remaining-gap | Not yet exposed by the mesh backend |
| `shape_propagate_to_all` | remaining-gap | Not yet exposed by the mesh backend |
| `shortest_path_pick` | selection-workflow | Viewport selection / groups; not full operator parity |
| `shortest_path_select` | selection-workflow | Viewport selection / groups; not full operator parity |
| `smooth_normals` | remaining-gap | Not yet exposed by the mesh backend |
| `solidify` | native-backend | blender_mesh: solidify |
| `sort_elements` | remaining-gap | Not yet exposed by the mesh backend |
| `spin` | native-backend | blender_mesh: spin |
| `split` | native-backend | blender_mesh: split |
| `split_normals` | remaining-gap | Not yet exposed by the mesh backend |
| `subdivide` | native-backend | blender_mesh: subdivide |
| `subdivide_edgering` | native-backend | blender_mesh: subdivide_edgering |
| `symmetrize` | native-backend | blender_mesh: symmetrize |
| `symmetry_snap` | native-backend | blender_mesh: symmetry_snap |
| `tris_convert_to_quads` | native-backend | blender_mesh: tris_convert_to_quads |
| `unsubdivide` | native-backend | blender_mesh: unsubdivide |
| `uv_texture_add` | remaining-gap | Not yet exposed by the mesh backend |
| `uv_texture_remove` | remaining-gap | Not yet exposed by the mesh backend |
| `uvs_reverse` | remaining-gap | Not yet exposed by the mesh backend |
| `uvs_rotate` | remaining-gap | Not yet exposed by the mesh backend |
| `vert_connect` | native-backend | blender_mesh: vert_connect |
| `vert_connect_concave` | native-backend | blender_mesh: vert_connect_concave |
| `vert_connect_nonplanar` | native-backend | blender_mesh: vert_connect_nonplanar |
| `vert_connect_path` | native-backend | blender_mesh: vert_connect_path |
| `vertices_smooth` | native-backend | blender_mesh: vertices_smooth |
| `vertices_smooth_laplacian` | native-backend | blender_mesh: vertices_smooth_laplacian |
| `wireframe` | native-backend | blender_mesh: wireframe |
