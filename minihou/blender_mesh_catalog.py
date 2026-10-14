"""Audited, nonmodal Blender mesh operations permitted in the isolated worker.

Never dispatch an arbitrary operator supplied by a graph. Interactive tools and
operators that write files or require another object are intentionally excluded.
"""
OPERATORS = (
    'merge', 'remove_doubles', 'split', 'edge_split', 'vert_connect', 'vert_connect_path',
    'vert_connect_concave', 'vert_connect_nonplanar', 'edge_face_add', 'fill', 'fill_grid',
    'fill_holes', 'beautify_fill', 'solidify', 'poke', 'tris_convert_to_quads',
    'quads_convert_to_tris', 'subdivide', 'unsubdivide', 'bevel', 'inset',
    'extrude_region_move', 'extrude_faces_move', 'extrude_edges_move', 'extrude_vertices_move',
    'extrude_manifold', 'extrude_repeat', 'bridge_edge_loops', 'spin', 'screw', 'symmetrize',
    'symmetry_snap', 'bisect', 'intersect', 'intersect_boolean', 'convex_hull', 'wireframe',
    'decimate', 'dissolve_edges', 'dissolve_faces', 'dissolve_verts', 'dissolve_limited',
    'dissolve_degenerate', 'delete', 'delete_loose', 'delete_edgeloop', 'edge_collapse',
    'edge_rotate', 'face_make_planar', 'face_split_by_edges', 'normals_make_consistent',
    'flip_normals', 'faces_shade_flat', 'faces_shade_smooth', 'mark_seam', 'mark_sharp',
    'vertices_smooth', 'vertices_smooth_laplacian', 'subdivide_edgering', 'offset_edge_loops',
    'primitive_circle_add', 'primitive_cone_add', 'primitive_cube_add', 'primitive_cylinder_add',
    'primitive_grid_add', 'primitive_ico_sphere_add', 'primitive_monkey_add', 'primitive_plane_add',
    'primitive_torus_add', 'primitive_uv_sphere_add',
)
