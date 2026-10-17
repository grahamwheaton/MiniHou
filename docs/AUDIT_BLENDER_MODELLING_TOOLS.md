# Blender 5.2 3D Modelling Tools Audit vs MiniHou

This document audits **ALL** of Blender's 3D modelling tools and operators against MiniHou's Rust core engine (`rust/crates/minihou-core/src/nodes/*`), node registry (`nodes/mod.rs`), parameter metadata (`rust/crates/minihou-app/src/param_meta.json`), and native desktop viewport tools (`rust/crates/minihou-app/src/ui/*`, `docs/NATIVE_EDITOR.md`).

The audit is grounded in real headless introspection of **Blender 5.2.1 LTS** (`docs/data/blender_ops_5_2.json`), which introspected 167 mesh operators, 51 curve operators, 27 transform operators, 55 UV operators, 39 sculpt operators, 56 object-modelling operators, 83 modifier subclasses, and 266 geometry node types.

## (a) Summary Table Per Category

| Category | HAVE | PARTIAL | MISSING | N/A | Total Operations |
| :--- | :---: | :---: | :---: | :---: | :---: |
| **Mesh Edit Tools** | 26 | 25 | 19 | 0 | 70 |
| **Object Tools** | 10 | 1 | 0 | 1 | 12 |
| **Modifiers: Generate** | 11 | 7 | 0 | 1 | 19 |
| **Modifiers: Deform** | 9 | 4 | 5 | 0 | 18 |
| **Modifiers: Normals/Data** | 1 | 2 | 0 | 0 | 3 |
| **Curve Tools** | 11 | 3 | 0 | 0 | 14 |
| **UV Tools** | 8 | 1 | 3 | 0 | 12 |
| **Sculpt Tools** | 0 | 0 | 0 | 5 | 5 |
| **TOTAL** | **76** | **43** | **27** | **7** | **153** |

> [!NOTE]
> Across 153 evaluated modelling operations and modifiers: **76 (49%)** have full functional parity in MiniHou, **43 (28%)** are partially implemented (fewer options or missing viewport modals), **27 (17%)** are currently missing, and **7** are non-applicable sculpt/animation runtime operations.

## (b) Full Audit Table

| Blender Operation | Blender ID | MiniHou Status | MiniHou Node / Location | Missing Options / Differences | Suggested MiniHou Implementation | Size | Priority |
| :--- | :--- | :---: | :--- | :--- | :--- | :---: | :---: |
| **--- MESH EDIT TOOLS ---** | | | | | | | |
| Extrude Region | `bpy.ops.mesh.extrude_region_move` | **HAVE** | polyextrude (model.rs:384), ui/modal_model.rs:188 (key E) | None - full parity for faces, edges, points along normal | Existing | - | High |
| Extrude Individual Faces | `bpy.ops.mesh.extrude_faces_move` | **PARTIAL** | polyextrude (model.rs:384) | Node supports mode='individual', but viewport modal (E) lacks toggle for individual vs connected faces | Add modal toggle key (Alt+I/Alt+E) in ui/modal_model.rs to switch polyextrude mode to 'individual' | S | High |
| Extrude Along Normals | `bpy.ops.mesh.extrude_normals_move` | **HAVE** | polyextrude (model.rs:384), ui/modal_model.rs:188 | None - default polyextrude direction follows averaged/per-element surface normals | Existing | - | Medium |
| Extrude Manifold | `bpy.ops.mesh.extrude_manifold` | **MISSING** | None (pushpull.rs:10 has pocketing, but not manifold boundary dissolve) | Auto-dissolves overlapping coplanar faces and splits interior walls when extruding into adjacent volume | New boolean-assisted manifold extrusion mode or post-process step in polyextrude / pushpull | M | Medium |
| Extrude to Cursor | `bpy.ops.mesh.dupli_extrude_cursor` | **MISSING** | None | Ctrl+RMB click to extrude element directly towards 3D cursor position in viewport | Viewport click handler in ui/modal_model.rs when 3D cursor is active | S | Low |
| Extrude Repeat | `bpy.ops.mesh.extrude_repeat` | **PARTIAL** | ui/modal_model.rs:108 (Shift+R repeat last) | Viewport supports Shift+R repeating last extrusion, but no procedural multi-step repeat extrude node | Add 'steps' and 'step_scale' parameter to polyextrude node | S | Medium |
| Inset Faces | `bpy.ops.mesh.inset` | **PARTIAL** | inset (pushpull.rs:780), ui/modal_model.rs:188 (key I) | Missing simultaneous 'depth' (extrude height while insetting), 'use_boundary', 'use_edge_rail', 'use_even_offset' | Add 'depth' and 'rail' parameters to pushpull::INSET node and modal handler | S | High |
| Outset Faces | `bpy.ops.mesh.inset (use_outset=True)` | **HAVE** | outset (pushpull.rs:786) | None - dedicated outset node offsets outward | Existing | - | Medium |
| Bevel (Edges/Vertices) | `bpy.ops.mesh.bevel` | **PARTIAL** | polybevel (bevel.rs:18), ui/modal_model.rs:188 (Ctrl+B) | Missing outer/inner miter modes (patch, arc), 'spread', 'harden_normals', vertex vs edge mode toggle in viewport modal | Add miter outer/inner algorithms to bevel.rs; add 'V' modal toggle in ui/modal_model.rs | M | High |
| Loop Cut & Slide | `bpy.ops.mesh.loopcut_slide` | **PARTIAL** | loop_cut / polysplit (modeling_tools.rs:45), ui/mesh_cut.rs:160 (Ctrl+R) | Multi-cut sliding (only single cut slides; >1 cuts stay evenly spaced), even edge rail slide, flipped slide, smoothness | Enhance loop_cut_fractions in modeling_tools.rs to support multi-cut offset and edge rail distance interpolation | M | High |
| Offset Edge Loop / Slide | `bpy.ops.mesh.offset_edge_loops_slide` | **MISSING** | None (polybevel with shape=flat approximates edge double-bevel) | Duplicates edge loop and slides copies outward along adjacent face geometry | New node 'offset_edge_loop' or parameter mode on edge_tools / bevel | M | Medium |
| Knife Tool | `bpy.ops.mesh.knife_tool` | **PARTIAL** | knife (knife.rs:17), ui/mesh_cut.rs:100 (key K) | Cut-through toggle ('Z' occlude geometry), angle snapping ('C' constraint), vertex/edge midpoint snapping | Add 'cut_through' parameter to knife node; wire 'Z' and 'C' keys in ui/mesh_cut.rs session | M | High |
| Knife Project | `bpy.ops.mesh.knife_project` | **MISSING** | None (ray and boolean exist separately) | Projects 2D/3D curve or silhouette onto mesh surface to cut intersecting edges and faces | New node 'knife_project' combining ray projection and polygon planar splitting | M | High |
| Bisect | `bpy.ops.mesh.bisect` | **PARTIAL** | clip (model.rs:378), slice (contour.rs:30) | Interactive 2-point line cut plane in viewport; 'use_fill' hole capping on clip | Add 'fill' cap option to clip node; add interactive Bisect modal tool to viewport UI | M | High |
| Bridge Edge Loops | `bpy.ops.mesh.bridge_edge_loops` | **PARTIAL** | polybridge (edge_tools.rs:257), bridge (surface.rs:26) | Missing curved interpolation (path, surface cubic/hermite), profile shape, smoothness factor, use_merge | Add 'interpolation' (linear/smooth/profile) and curvature profile curve input to polybridge node | M | High |
| Fill (Face/Edge from Selection) | `bpy.ops.mesh.fill` | **PARTIAL** | polyfill (model.rs:366) | Node fills boundary loops with n-gons; missing interactive 'F' key to create edge between 2 vertices or face from selected edges/points | Add interactive 'F' modal / operator in ui/modal_model.rs to insert polyfill or edge bridge node | S | High |
| Grid Fill | `bpy.ops.mesh.fill_grid` | **MISSING** | None | Fills closed boundary loop of 2 opposing spans of equal edge count with pure quad grid topology | New node 'grid_fill' using harmonic / Transfinite Interpolation (TFI) grid patching | M | High |
| Triangulate Faces | `bpy.ops.mesh.quads_convert_to_tris` | **PARTIAL** | divide (model.rs:360) | Missing quad split methods (shortest diagonal, longest diagonal, beauty, fixed alternate) | Add 'method' parameter to divide node (beauty, shortest_diagonal, alternate) | S | Medium |
| Tris to Quads | `bpy.ops.mesh.tris_convert_to_quads` | **MISSING** | None | Merges adjacent coplanar triangle pairs into quads respecting max face angle and UV/sharp seams | New node 'tris_to_quads' using maximum-weight matching on adjacent triangle edge graph | M | High |
| Poly Build | `bpy.ops.mesh.polybuild_face_at_cursor` | **MISSING** | None (sketch.rs is for 2D/3D curves and tape measure) | Interactive quad placement tool: drag boundary edges to extrude quads, click vertices, retopology workflow | New viewport interactive tool 'poly_build' in minihou-app/src/ui | L | Medium |
| Spin | `bpy.ops.mesh.spin` | **PARTIAL** | revolve (surface.rs:39) | Node revolves curves into solid mesh; missing 'use_duplicate' (spin copies of geometry without connecting walls) and interactive viewport widget | Add 'keep_individual' parameter to revolve; add interactive Spin viewport gizmo | S | Medium |
| Screw | `bpy.ops.mesh.screw` | **PARTIAL** | helix (curves.rs:16) + sweep (curves.rs:338) | Combinable with helix curve + sweep, but no single-step procedural screw node with pitch/turns/axis | New node 'screw' revolving profile while advancing along axis by pitch distance per turn | S | Medium |
| Smooth Vertices | `bpy.ops.mesh.vertices_smooth` | **HAVE** | smooth (attrib_tools.rs:113), point_relax (deform.rs:66) | Missing per-axis constraint (lock X, Y, or Z) | Add 'axis_lock' parameter to smooth node | S | Medium |
| Laplacian Smooth | `bpy.ops.mesh.vertices_smooth_laplacian` | **PARTIAL** | smooth (attrib_tools.rs:113) | Method 'laplacian' exists; missing 'preserve_volume' and border lambda factor | Add 'preserve_volume' volume-preserving scale correction to smooth node | S | Low |
| Edge Slide | `bpy.ops.transform.edge_slide` | **MISSING** | None (loop_cut slides new cut, but cannot slide existing mesh edges) | Slides selected edge loops along adjacent faces with proportional distance clamp and rail projection | New node 'edge_slide' and viewport modal 'GG' in minihou-app | M | High |
| Vertex Slide | `bpy.ops.transform.vert_slide` | **MISSING** | None | Slides selected vertices along incident edges | New node 'vertex_slide' and viewport modal in minihou-app | S | Medium |
| Shrink / Fatten | `bpy.ops.transform.shrink_fatten` | **PARTIAL** | peak (transform.rs:10), soft_peak (modeling_tools.rs:14) | Procedural node 'peak' exists; missing interactive viewport modal tool (Alt+S) with distance readout | Bind Alt+S in ui/keymap.rs to a modal Peak transform tool in ui/modal_transform.rs | S | High |
| To Sphere | `bpy.ops.transform.tosphere` | **MISSING** | None | Warps selected vertices radially toward a best-fit sphere about selection pivot | New node 'to_sphere' or deform mode in deform.rs; viewport modal Shift+Alt+S | S | Medium |
| Shear | `bpy.ops.transform.shear` | **MISSING** | None | Shears geometry parallel to an axis based on perpendicular distance from pivot | Add 'shear' parameter vector to transform node (modify.rs:159) | S | Low |
| Push / Pull | `bpy.ops.transform.push_pull` | **HAVE** | pushpull (pushpull.rs:10), ui/modal_model.rs:188 (Alt+E) | None - MiniHou has both SketchUp-style solid pocket push/pull and proportional scale from pivot | Existing | - | High |
| Rip Region | `bpy.ops.mesh.rip` | **MISSING** | None | Splits vertex/edges along view direction and starts interactive move modal (key V) | New viewport modal tool 'rip' disconnecting element and initiating move | M | Medium |
| Rip Edge & Fill | `bpy.ops.mesh.rip_edge` | **MISSING** | None | Rips an edge and creates a connecting quad face between separated sides (Alt+V) | Add 'rip_edge' operation to edge_tools.rs | M | Low |
| Merge by Distance (Remove Doubles) | `bpy.ops.mesh.remove_doubles` | **HAVE** | fuse (primitives.rs:11) | None - merges vertices within distance with full attribute remapping and outgroups | Existing | - | High |
| Merge (Center / Cursor / Collapse) | `bpy.ops.mesh.merge` | **PARTIAL** | edge_collapse (modeling_tools.rs:26) | Merge at selection center, at 3D cursor, or merge first/last point; viewport modal 'M' | Add 'merge_points' node with type='center'/'first'/'last'/'cursor' and viewport 'M' modal | S | High |
| Split Faces / Edges | `bpy.ops.mesh.split` | **PARTIAL** | edge_cusp (modeling_tools.rs:38), polysplit (modeling_tools.rs:45) | Disconnects selected faces/edges into isolated vertices without moving them ('Y' in Blender) | New node 'split_elements' disconnecting picked element topology | S | Medium |
| Separate | `bpy.ops.mesh.separate` | **PARTIAL** | blast (group.rs:169), connectivity (attrib_tools.rs:107) | Interactive 'P' key in viewport to fork selected elements into a separate displayed node/branch | Add viewport operator 'Separate' (P) in ui/modal_model.rs creating parallel branch | S | High |
| Dissolve Vertices | `bpy.ops.mesh.dissolve_verts` | **MISSING** | None (fuse welds, but dissolve vertices removes 2-valent or collinear vertices while keeping edge) | Removes vertices while merging connected edges | New node 'dissolve_points' in dissolve.rs | S | Medium |
| Dissolve Edges | `bpy.ops.mesh.dissolve_edges` | **HAVE** | dissolve_edges (edge_tools.rs:251) | None - dissolves edges, supports angle threshold and seam preservation | Existing | - | High |
| Dissolve Faces | `bpy.ops.mesh.dissolve_faces` | **HAVE** | dissolve (dissolve.rs:12) | None - merges coplanar faces into n-gons within angle threshold | Existing | - | High |
| Limited Dissolve | `bpy.ops.mesh.dissolve_limited` | **HAVE** | dissolve (dissolve.rs:12) | None - coplanar face reduction by dihedral angle tolerance | Existing | - | High |
| Dissolve Degenerate / Clean | `bpy.ops.mesh.dissolve_degenerate` | **HAVE** | clean (deform.rs:296) | None - removes degenerate polygons, zero-area faces, and unused points | Existing | - | High |
| Edge Collapse | `bpy.ops.mesh.edge_collapse` | **HAVE** | edge_collapse (modeling_tools.rs:26) | None - collapses edges in group to midpoint | Existing | - | High |
| Delete Vertices | `bpy.ops.mesh.delete (type='VERT')` | **HAVE** | blast (group.rs:169, domain='points'), delete (modify.rs:405) | None - deletes points and incident faces | Existing | - | High |
| Delete Faces | `bpy.ops.mesh.delete (type='FACE')` | **HAVE** | blast (group.rs:169, domain='primitives'), delete (modify.rs:405), key X/Delete in viewport | None - deletes selected polygons | Existing | - | High |
| Delete Edges | `bpy.ops.mesh.delete (type='EDGE')` | **PARTIAL** | blast (group.rs:169) | blast line 173 explicitly rejects domain='edges' with error 'blast edge selections require edge-group parser' | Implement edge domain deletion in blast / delete nodes | S | High |
| Delete Only Faces | `bpy.ops.mesh.delete (type='ONLY_FACE')` | **MISSING** | None | Deletes face polygons while preserving boundary edge wireframe | Add 'keep_boundary_edges' flag to blast / delete node | S | Medium |
| Delete Edge Loop | `bpy.ops.mesh.delete_edgeloop` | **PARTIAL** | dissolve_edges (edge_tools.rs:251) | Dissolving edge loop removes edges, but Blender delete edge loop also merges adjacent rings | Add loop delete mode in edge_tools.rs | S | Low |
| Subdivide | `bpy.ops.mesh.subdivide` | **HAVE** | subdivide (subdivide.rs:10) | Missing fractal randomness and quadcorner mode; Catmull-Clark and linear subdivision fully implemented | Existing | - | High |
| Unsubdivide | `bpy.ops.mesh.unsubdivide` | **MISSING** | None (polyreduce does decimation, but not inverse quad grid unsubdivision) | Inverse Catmull-Clark grid un-subdivision for rebuilding low-poly topology | New node 'unsubdivide' reversing regular quad subdivision | M | Medium |
| Symmetrize | `bpy.ops.mesh.symmetrize` | **HAVE** | mirror (mirror.rs:10 with cut=true) | None - mirror with cut=true drops negative/positive side and mirrors across seam with welding | Existing | - | High |
| Mirror Transform | `bpy.ops.transform.mirror` | **HAVE** | mirror (mirror.rs:10), ui/mirror_tool.rs:1 (Alt+M) | None - interactive 6-axis gizmo and procedural reflection plane | Existing | - | High |
| Flip Normals | `bpy.ops.mesh.flip_normals` | **HAVE** | reverse (model.rs:348), ui/keymap.rs:52 (Alt+N) | None - flips polygon vertex winding and normals | Existing | - | High |
| Recalculate Normals Outside | `bpy.ops.mesh.normals_make_consistent` | **MISSING** | None (pushpull checks solid orientation, but no general non-manifold/manifold outward propagator) | Traverses connected manifold faces and ensures all face normals point consistently outward | New node 'orient_normals' / 'recalculate_normals' with outside/inside mode | M | High |
| Edge Crease | `bpy.ops.transform.edge_crease` | **PARTIAL** | crease (edge_tools.rs:222) | Node writes creaseweight attribute; missing interactive viewport modal (Shift+E) to drag crease weight | Bind Shift+E in viewport to modal crease slider modifying edge creaseweight | S | High |
| Edge Bevel Weight | `bpy.ops.transform.edge_bevelweight` | **MISSING** | None (polybevel uses min_angle or group pattern, but no per-edge bevel_weight float attribute) | Weights edges with 0..1 attribute for variable-width beveling | Add 'bevel_weight' attribute support to polybevel node | S | Medium |
| Mark / Clear Seam | `bpy.ops.mesh.mark_seam` | **PARTIAL** | edge_group (edge_tools.rs:228), uv_flatten (uv.rs:420) | Missing direct edge attribute 'seam' and viewport toggle key (Ctrl+E -> Mark Seam) | Add 'seam' integer/boolean edge attribute recognized by uv_flatten and viewport edge overlay | S | High |
| Mark / Clear Sharp | `bpy.ops.mesh.mark_sharp` | **PARTIAL** | edge_group (edge_tools.rs:228), edge_cusp (modeling_tools.rs:38) | Missing direct 'sharp' edge attribute and viewport toggle key | Add 'sharp' edge attribute and viewport rendering style | S | Medium |
| Select Loops | `bpy.ops.mesh.loop_select` | **HAVE** | select_loop (select_nodes.rs:143), ui/selection_region.rs (Alt+click) | None - Alt+click selects edge/face loops; select_loop node records procedural selection | Existing | - | High |
| Select Rings | `bpy.ops.mesh.select_edge_ring` | **HAVE** | select_ring (select_nodes.rs:145), ui/selection_region.rs (Ctrl+Alt+click) | None - Ctrl+Alt+click selects edge rings; select_ring node records procedural ring | Existing | - | High |
| Select Shortest Path | `bpy.ops.mesh.shortest_path_select` | **HAVE** | select_path (select_nodes.rs:94), ui/selection_region.rs (Ctrl+click) | None - Ctrl+click picks geodesic shortest path between elements | Existing | - | High |
| Select More / Less (Grow/Shrink) | `bpy.ops.mesh.select_more / select_less` | **HAVE** | grow (select_nodes.rs:66), ui/keymap.rs:121 (Ctrl+Alt+Plus / Minus) | None - grows/shrinks element selection by topological adjacency | Existing | - | High |
| Select Linked | `bpy.ops.mesh.select_linked` | **HAVE** | connectivity (attrib_tools.rs:107), ui/keymap.rs:123 (key L) | None - selects topologically connected components | Existing | - | High |
| Select Random | `bpy.ops.mesh.select_random` | **HAVE** | group_create (group.rs:83, random_enable=true) | None - selects random percentage with seed across points or faces | Existing | - | Medium |
| Checker Deselect | `bpy.ops.mesh.select_nth` | **HAVE** | group_range (modeling_tools.rs:20) | None - selects N every M elements with offset and invert | Existing | - | Medium |
| Select Similar | `bpy.ops.mesh.select_similar` | **PARTIAL** | group_create (group.rs:83, normals_enable=true), ui/viewport_ui.rs (Select Coplanar) | Supports normal angle and coplanar expansion; missing face area, edge length, perimeter, vertex group similarity | Add 'similar' metrics (area, perimeter, length) to group_create node | S | Medium |
| Select Sharp Edges | `bpy.ops.mesh.edges_select_sharp` | **HAVE** | group_create (group.rs:83, edges_enable=true, min_edge_angle) | None - selects edges with dihedral angle above threshold | Existing | - | High |
| Select Non-Manifold | `bpy.ops.mesh.select_non_manifold` | **HAVE** | group_create (group.rs:83, unshared_edges=true) | None - selects boundary edges and unshared edges | Existing | - | High |
| Select Boundary Loop | `bpy.ops.mesh.region_to_loop` | **PARTIAL** | group_create (group.rs:83, boundary_groups=true) | Converts selected face region into its outer boundary edge loop | Add 'region_boundary' domain conversion in group_promote | S | Medium |
| Select Interior Faces | `bpy.ops.mesh.select_interior_faces` | **MISSING** | None | Selects faces where all edges have more than 2 connected faces | Add 'interior_faces' option to group_create node | S | Low |
| Select Face by Sides | `bpy.ops.mesh.select_face_by_sides` | **PARTIAL** | wrangle (wrangle.rs:6, '@numv == 4') | Selects triangles, quads, or n-gons; achievable via wrangle expression but no dedicated UI/node preset | Add 'sides' filter (==, !=, <, >) to group_create node | S | Medium |
| **--- OBJECT TOOLS ---** | | | | | | | |
| Join Objects | `bpy.ops.object.join` | **HAVE** | merge (merge.rs:10) | None - merges multiple geometry streams into a single composite mesh | Existing | - | High |
| Duplicate Objects | `bpy.ops.object.duplicate` | **HAVE** | copy_to_points (copy_to_points.rs:20), ui/keymap.rs:101 (Ctrl+D / Ctrl+C / Ctrl+V) | None - duplicates nodes and geometries in DAG | Existing | - | High |
| Duplicate Linked | `bpy.ops.object.duplicate_move_linked` | **HAVE** | instance (instance.rs:10), DAG wire branching | None - multiple consumer wires share read-only Arc<Geometry> pointers natively | Existing | - | High |
| Make Instances Real | `bpy.ops.object.duplicates_make_real` | **HAVE** | copy_to_points (copy_to_points.rs:20) | None - bakes instanced geometries into concrete mesh topology | Existing | - | High |
| Shade Smooth / Flat | `bpy.ops.object.shade_smooth / shade_flat` | **HAVE** | normal (modify.rs:189), facet (model.rs:354), ui/viewport_ui.rs | None - computes smooth point normals or faceted polygon normals; viewport shading pie | Existing | - | High |
| Shade Auto Smooth by Angle | `bpy.ops.object.shade_smooth_by_angle` | **HAVE** | normal (modify.rs:189, cusp_angle), edge_cusp (modeling_tools.rs:38) | None - cusps normals beyond angular threshold | Existing | - | High |
| Set Origin | `bpy.ops.object.origin_set` | **PARTIAL** | match_size (modify.rs:270), transform (modify.rs:159) | Supports bounds center, min, max; missing origin to center of mass, origin to 3D cursor | Add 'center_of_mass' mode to match_size node | S | Medium |
| Clear Origin | `bpy.ops.object.origin_clear` | **HAVE** | transform (modify.rs:159, pivot=[0,0,0]) | None - resets pivot to world origin | Existing | - | Low |
| Apply Transforms | `bpy.ops.object.transform_apply` | **HAVE** | transform (modify.rs:159) | None - procedural nodes bake translate/rotate/scale directly into point positions (P) | Existing | - | High |
| Convert To (Mesh / Curve) | `bpy.ops.object.convert` | **HAVE** | convert_line (curves.rs:97), sweep (curves.rs:338), contour (contour.rs:11) | None - converts mesh edges to curves, curves to mesh tubes/surfaces, meshes to contours | Existing | - | High |
| Randomize Transform | `bpy.ops.object.randomize_transform` | **HAVE** | attribute_randomize (attrib_tools.rs:294), noise (modify.rs:161) | None - randomizes positions, rotations, or scale procedurally | Existing | - | Medium |
| Parent Set / Clear | `bpy.ops.object.parent_set / parent_clear` | **N/A** | DAG graph connections / subgraphs | Scene graph parent-child hierarchy replaced by dataflow node DAG | N/A | - | - |
| **--- MODIFIERS: GENERATE ---** | | | | | | | |
| Array Modifier | `bpy.types.ArrayModifier` | **PARTIAL** | copy_to_points (copy_to_points.rs:20), line (generate.rs:275) | Requires wiring line + copy_to_points; no single dedicated 'array' node with count, offset, fit length, curve fit | New dedicated node 'array' with count, offset xyz, rotation step, scale step, fit length/curve | S | High |
| Bevel Modifier | `bpy.types.BevelModifier` | **PARTIAL** | polybevel (bevel.rs:18) | Missing miter outer/inner patch/arc modes, vertex group weight map, harden normals | Add miter and weight map parameters to polybevel | M | High |
| Boolean Modifier | `bpy.types.BooleanModifier` | **HAVE** | boolean (boolean.rs:15), boolean_fracture (boolean_fracture.rs:12) | None - union, difference, intersect, subtract; outputs seam and side groups | Existing | - | High |
| Build Modifier | `bpy.types.BuildModifier` | **PARTIAL** | carve (curves.rs:161), delete (modify.rs:405 with @Frame) | No dedicated procedural build node animating progressive polygon appearance over frames | New node 'build' with start/length frame range | S | Low |
| Decimate Modifier | `bpy.types.DecimateModifier` | **HAVE** | polyreduce (reduce.rs:18), dissolve (dissolve.rs:12) | None - quadric edge collapse reduction (target triangles/ratio) and planar angle dissolve | Existing | - | High |
| Edge Split Modifier | `bpy.types.EdgeSplitModifier` | **HAVE** | edge_cusp (modeling_tools.rs:38), facet (model.rs:354) | None - splits sharp edges by angle or marked edges | Existing | - | High |
| Geometry Nodes | `bpy.types.NodesModifier` | **HAVE** | MiniHou Native DAG Engine (core/src/nodes/*) | None - MiniHou is an entire standalone procedural Geometry Nodes environment | Existing | - | High |
| Mask Modifier | `bpy.types.MaskModifier` | **HAVE** | blast (group.rs:169), delete (modify.rs:405) | None - filters geometry dynamically based on vertex/prim group or attribute | Existing | - | High |
| Mirror Modifier | `bpy.types.MirrorModifier` | **HAVE** | mirror (mirror.rs:10) | None - mirrors across axis, bisects cut plane, welds seams within distance | Existing | - | High |
| Multiresolution | `bpy.types.MultiresModifier` | **N/A** | subdivide (subdivide.rs:10) | Sculpt-layer multiresolution displacement mesh not part of procedural CAD engine | N/A | - | - |
| Remesh Modifier | `bpy.types.RemeshModifier` | **PARTIAL** | remesh (volume.rs:23) | Full Voxel remeshing with SDF and attribute transfer; missing Quad (Quadriflow) and Sharp (dual contouring) modes | Integrate Quadriflow or Instant Meshes library for quad remeshing | L | High |
| Screw Modifier | `bpy.types.ScrewModifier` | **PARTIAL** | helix (curves.rs:16) + sweep (curves.rs:338) | Missing single-node screw operator with axis, pitch, angle, iterations | New node 'screw' | S | Medium |
| Skin Modifier | `bpy.types.SkinModifier` | **PARTIAL** | skin (surface.rs:55), sweep (curves.rs:338) | Skin lofts profiles; sweep tubes curves; missing branching skeletal graph quad envelope skinning | New node 'skeleton_skin' generating quad hull from branching edge network | L | Medium |
| Solidify Modifier | `bpy.types.SolidifyModifier` | **HAVE** | solidify (solidify.rs:10) | None - thickness, offset, even thickness, rim generation, top/bottom/side outgroups | Existing | - | High |
| Subdivision Surface | `bpy.types.SubsurfModifier` | **HAVE** | subdivide (subdivide.rs:10) | None - Catmull-Clark and linear subdivision with semi-sharp edge creaseweight | Existing | - | High |
| Triangulate Modifier | `bpy.types.TriangulateModifier` | **HAVE** | divide (model.rs:360) | None - converts n-gons and quads to triangles | Existing | - | High |
| Volume to Mesh | `bpy.types.VolumeToMeshModifier` | **HAVE** | remesh (volume.rs:23), points_to_surface (volume.rs:28) | None - extracts isosurface mesh from SDF volume or point cloud | Existing | - | High |
| Weld Modifier | `bpy.types.WeldModifier` | **HAVE** | fuse (primitives.rs:11) | None - merges vertices within distance tolerance | Existing | - | High |
| Wireframe Modifier | `bpy.types.WireframeModifier` | **PARTIAL** | convert_line (curves.rs:97) + sweep (curves.rs:338) | Achievable via 2 nodes (convert_line -> sweep); missing single 1-step procedural wireframe node | New node 'wireframe' converting edges directly to thickened quad/tube geometry | S | High |
| **--- MODIFIERS: DEFORM ---** | | | | | | | |
| Armature Modifier | `bpy.types.ArmatureModifier` | **MISSING** | None (capture_deform.rs has point_deform and lattice, but no bone skeleton solver) | Bone skeleton hierarchy, bone weight vertex painting, dual quaternion skinning | Add skeletal rigging and linear blend skinning nodes | L | Low |
| Cast Modifier | `bpy.types.CastModifier` | **MISSING** | None | Warps geometry toward sphere, cylinder, or cuboid primitive | New node 'cast' in deform.rs | S | Low |
| Curve Modifier | `bpy.types.CurveModifier` | **HAVE** | path_deform (curves.rs:705) | None - deforms geometry along curve with minimal rotation frame, stretch, and twist | Existing | - | High |
| Displace Modifier | `bpy.types.DisplaceModifier` | **PARTIAL** | noise (modify.rs:161), image_file (image_nodes.rs:7), peak (transform.rs:10) | Displaces along normals via procedural noise or heightfield image grid; missing arbitrary UV texture sampling on 3D mesh | Add texture coordinate displacement sampling to modify.rs | M | Medium |
| Hook Modifier | `bpy.types.HookModifier` | **PARTIAL** | soft_transform (capture_deform.rs:566) | Deforms group around pivot with radius falloff; missing external target tracking | Existing node handles deformation | S | Low |
| Laplacian Deform | `bpy.types.LaplacianDeformModifier` | **MISSING** | None | Laplacian differential surface deformation with fixed anchor vertices | Add sparse linear system solver for Laplacian surface deformation | L | Low |
| Lattice Modifier | `bpy.types.LatticeModifier` | **HAVE** | lattice (capture_deform.rs:87), lattice_grid (capture_deform.rs:10) | None - full FFD trilinear lattice cage deformation with custom grid divisions | Existing | - | High |
| Mesh Deform Modifier | `bpy.types.MeshDeformModifier` | **HAVE** | point_deform (capture_deform.rs:94) | None - cage-based surface deformation with max points and radius falloff | Existing | - | High |
| Shrinkwrap Modifier | `bpy.types.ShrinkwrapModifier` | **HAVE** | ray (ray.rs:15) | None - projects points to nearest surface or along direction/normal with lift distance | Existing | - | High |
| Simple Deform: Twist | `bpy.types.SimpleDeformModifier (TWIST)` | **HAVE** | twist (modeling_tools.rs:390) | None - axial twist with direction, origin, group, strength | Existing | - | High |
| Simple Deform: Bend | `bpy.types.SimpleDeformModifier (BEND)` | **HAVE** | bend (deform.rs:5) | None - cylindrical bending along origin and axis | Existing | - | High |
| Simple Deform: Taper | `bpy.types.SimpleDeformModifier (TAPER)` | **HAVE** | linear_taper (modeling_tools.rs:396) | None - linear taper with symmetric, dropoff, up vector | Existing | - | High |
| Simple Deform: Stretch | `bpy.types.SimpleDeformModifier (STRETCH)` | **HAVE** | squash_stretch (modeling_tools.rs:402) | None - squash and stretch with volume preservation toggle | Existing | - | High |
| Smooth Modifier | `bpy.types.SmoothModifier` | **HAVE** | smooth (attrib_tools.rs:113) | None - Laplacian and uniform surface smoothing with iterations and pin boundary | Existing | - | High |
| Smooth Corrective | `bpy.types.CorrectiveSmoothModifier` | **MISSING** | None | Smooths deformation delta while preserving original rest volume and details | New node 'smooth_corrective' storing rest state and smoothing difference | M | Low |
| Surface Deform Modifier | `bpy.types.SurfaceDeformModifier` | **PARTIAL** | point_deform (capture_deform.rs:94), ray (ray.rs:15) | Binds mesh to follow deformation of another surface | Enhance point_deform with barycentric surface binding | M | Low |
| Warp Modifier | `bpy.types.WarpModifier` | **MISSING** | None | Two-object spatial warp transformation | New node 'warp' in deform.rs | S | Low |
| Wave Modifier | `bpy.types.WaveModifier` | **PARTIAL** | falloff (falloff.rs:6, shape='wave'), noise (modify.rs:161) | No dedicated animated wave displacement node; achieved via falloff wave + peak/transform | New node 'wave' in deform.rs | S | Low |
| **--- MODIFIERS: NORMALS/DATA ---** | | | | | | | |
| Data Transfer Modifier | `bpy.types.DataTransferModifier` | **HAVE** | attribute_transfer (attrib_tools.rs:454), ray (ray.rs:15) | None - transfers point/prim attributes, normals, UVs from source mesh to target | Existing | - | High |
| Normal Edit Modifier | `bpy.types.NormalEditModifier` | **PARTIAL** | normal (modify.rs:189), wrangle (wrangle.rs:6) | Missing radial normal re-pointing towards arbitrary center in normal node UI | Add 'target_center' radial normal mode to normal node | S | Medium |
| Weighted Normal Modifier | `bpy.types.WeightedNormalModifier` | **PARTIAL** | modify.rs:202 (point_normals internally weights by angle) | Missing exposed user dropdown for weighting method (face area vs corner angle) | Add 'weighting_mode' (angle, area, uniform) to normal node | S | Medium |
| **--- CURVE TOOLS ---** | | | | | | | |
| Extrude Curve Point | `bpy.ops.curve.extrude` | **PARTIAL** | sketch.rs:184 (drawing lines), curve_tool.rs | Sketch draws new curve segments; missing interactive 'E' key to extrude selected Bezier endpoint in viewport | Add 'E' handler in ui/curve_tool.rs when Bezier endpoint is selected | S | High |
| Subdivide Curve | `bpy.ops.curve.subdivide` | **HAVE** | resample (curves.rs:149) | None - resamples curve by uniform length or segment count | Existing | - | High |
| Handle Types (Auto/Vector/Aligned/Free) | `bpy.ops.curve.handle_type_set` | **HAVE** | curve_tool.rs:44 (AUTO, VECTOR, ALIGNED, MIRRORED, FREE) | None - full handle type resolution and interactive dragging in native viewport | Existing | - | High |
| Smooth Curve | `bpy.ops.curve.smooth` | **HAVE** | curve_simplify (curves.rs:209, smooth=true), resample (curves.rs:149) | None - smooths curve control points and tangents | Existing | - | Medium |
| Toggle Cyclic (Close/Open) | `bpy.ops.curve.cyclic_toggle` | **HAVE** | bezier (curves.rs:30, closed=true), sketch.rs | None - closes curve loop and welds start/end tangents | Existing | - | High |
| Fill 2D Curve | `bpy.ops.curve.fill` | **HAVE** | polyfill (model.rs:366) | None - creates planar boundary face from closed curve | Existing | - | High |
| Convert Curve to Mesh | `bpy.ops.curve.convert` | **HAVE** | sweep (curves.rs:338), skin (surface.rs:55), polyfill (model.rs:366) | None - converts curve to cross-section tube, lofted surface, or planar cap | Existing | - | High |
| Curve Tilt | `bpy.ops.transform.tilt` | **PARTIAL** | sweep (curves.rs:338, twist parameter) | Global twist supported in sweep; missing per-point 'tilt' float attribute along curve | Add 'tilt' attribute support in curves.rs and curve_tool.rs | S | High |
| Curve Fillet / Chamfer | `bpy.ops.curve.fillet` | **HAVE** | fillet (curves.rs:175) | None - rounds corners by radius, divisions, and min angle | Existing | - | High |
| Curve Simplify / Decimate | `bpy.ops.curve.decimate` | **HAVE** | curve_simplify (curves.rs:209) | None - Douglas-Peucker simplification by tolerance distance and angle | Existing | - | High |
| Spline Type (Bezier/Poly/NURBS) | `bpy.ops.curve.spline_type_set` | **PARTIAL** | bezier (curves.rs:30), convert_line (curves.rs:97), polypath (curves.rs:106) | Bezier and Polyline supported; NURBS splines not implemented | Add NURBS curve evaluation in curves.rs | M | Low |
| Make Segment | `bpy.ops.curve.make_segment` | **HAVE** | polypath (curves.rs:106), convert_line (curves.rs:97) | None - connects two open endpoints into a single spline | Existing | - | Medium |
| Split / Separate Curve | `bpy.ops.curve.split` | **HAVE** | carve (curves.rs:161), blast (group.rs:169) | None - cuts curve at specified normalized interval or element | Existing | - | Medium |
| Sweep / Profile Bevel | `bpy.ops.curve.sweep` | **HAVE** | sweep (curves.rs:338) | None - sweeps circle or profile along backbone curve with radius, section_scale, twist, end caps | Existing | - | High |
| **--- UV TOOLS ---** | | | | | | | |
| Unwrap (Conformal / LSCM) | `bpy.ops.uv.unwrap` | **HAVE** | uv_flatten (uv.rs:420) | None - conformal least-squares conformal map (LSCM) unwrap with padding and island packing | Existing | - | High |
| Smart UV Project | `bpy.ops.uv.smart_project` | **HAVE** | uv_flatten (uv.rs:420, seam_angle parameter) | None - automatic seam generation by face dihedral angle threshold and island packing | Existing | - | High |
| Cube Projection | `bpy.ops.uv.cube_project` | **HAVE** | uv_project (uv.rs:5, method='box') | None - triplanar box projection with orientation, center, scale, rotation | Existing | - | High |
| Cylinder Projection | `bpy.ops.uv.cylinder_project` | **HAVE** | uv_project (uv.rs:5, method='cylindrical') | None - cylindrical coordinate projection with seam wrap correction | Existing | - | High |
| Sphere Projection | `bpy.ops.uv.sphere_project` | **HAVE** | uv_project (uv.rs:5, method='spherical') | None - spherical latitude/longitude projection with polar seam correction | Existing | - | High |
| Planar Projection | `bpy.ops.uv.project_from_view` | **HAVE** | uv_project (uv.rs:5, method='planar') | None - orthographic/planar projection with rotation matrix | Existing | - | High |
| Pack Islands | `bpy.ops.uv.pack_islands` | **HAVE** | uv_flatten (uv.rs:420, padding parameter) | None - 2D bounding box bin packing with margin padding | Existing | - | High |
| UV Transform / Align / Randomize | `bpy.ops.uv.align` | **HAVE** | uv_transform (uv.rs:43) | None - translate, rotate, scale, flip_u, flip_v, random variation, axis alignment | Existing | - | High |
| Follow Active Quads | `bpy.ops.uv.follow_active_quads` | **MISSING** | None | Unwraps continuous quad strips into a straight orthogonal rectangular grid in UV space | New node 'uv_follow_active_quads' or mode in uv.rs walking quad strips | M | High |
| Lightmap Pack | `bpy.ops.uv.lightmap_pack` | **PARTIAL** | uv_flatten (uv.rs:420) | Missing separate lightmap face unwrap without distortion; uv_flatten packs islands with padding | Add dedicated lightmap unwrap mode | S | Low |
| Minimize Stretch | `bpy.ops.uv.minimize_stretch` | **MISSING** | None | Iterative relaxation of UV coordinates to reduce angle/area distortion | Add relaxation iterations to uv_flatten | S | Low |
| Interactive 2D UV Editor | `Blender UV Editor Workspace` | **MISSING** | None (MiniHou only operates procedurally in 3D viewport and node graph) | Dedicated 2D image/UV canvas for interactive vertex/island manipulation | Add 2D UV viewport dockable panel in minihou-app | L | Medium |
| **--- SCULPT TOOLS ---** | | | | | | | |
| Sculpt Brush Stroke | `bpy.ops.sculpt.brush_stroke` | **N/A** | attrib_paint (attrib_paint.rs / key 5) | N/A - MiniHou is a procedural CAD/mesh node graph; attribute painting exists, dense brush sculpting is out of scope | N/A (sculpt engine) | - | - |
| Dynamic Topology (Dyntopo) | `bpy.ops.sculpt.dynamic_topology_toggle` | **N/A** | None | N/A - Real-time continuous mesh retriangulation during brush stroke | N/A (sculpt engine) | - | - |
| Cloth Filter | `bpy.ops.sculpt.cloth_filter` | **N/A** | goals / solver (solver.rs) | N/A - Real-time interactive cloth sculpt filter; MiniHou has node-based physics solver instead | N/A (sculpt engine) | - | - |
| Color Filter | `bpy.ops.sculpt.color_filter` | **N/A** | color (attrib_tools.rs:296), attrib_paint | N/A - Global sculpt color attribute filtering | N/A (sculpt engine) | - | - |
| Face Sets | `bpy.ops.sculpt.face_set_edit` | **N/A** | group_create (group.rs:83), blast | N/A - Sculpt mode face set masking | N/A (sculpt engine) | - | - |

## (c) Top 15 Recommended Additions (in Priority Order)

### 1. Dedicated Array Node
- **Category**: Modifiers: Generate / Object
- **Implementation Size**: **S**
- **Why High Value for Procedural Modelling**: Crucial procedural primitive. Currently requires manually wiring `line` -> `copy_to_points` every time.
- **Technical Design & Implementation**: Implement `array` node in `rust/crates/minihou-core/src/nodes/generate.rs` (or `transform.rs`) supporting `count`, `offset` (XYZ translation step), `rotate` (Euler rotation step), `scale` (scale multiplier step), `fit_length`, and `merge` toggle.

### 2. Grid Fill (Quad Patching)
- **Category**: Mesh Edit Tools
- **Implementation Size**: **M**
- **Why High Value for Procedural Modelling**: Fills closed boundary loops between 2 opposing edge spans of equal count with clean quad grid topology (essential for CAD hole closing and organic patching).
- **Technical Design & Implementation**: New node `grid_fill` in `rust/crates/minihou-core/src/nodes/surface.rs` or `model.rs` using Transfinite Interpolation (TFI) / Coons patch across boundary loop spans.

### 3. Edge Slide & Vertex Slide (Node + Modal GG)
- **Category**: Mesh Edit Tools
- **Implementation Size**: **M**
- **Why High Value for Procedural Modelling**: Core topology manipulation tool. Allows tweaking edge positions without distorting the underlying curvature/plane.
- **Technical Design & Implementation**: Add `edge_slide` and `vert_slide` nodes in `edge_tools.rs` projecting points along adjacent quad rails with factor [-1..1] and clamp; wire `GG` in `ui/keymap.rs` and `ui/modal_transform.rs`.

### 4. Recalculate Normals Outside (Consistent Manifold Orientation)
- **Category**: Mesh Edit Tools
- **Implementation Size**: **M**
- **Why High Value for Procedural Modelling**: Fixes inverted or inconsistent face windings resulting from booleans, cuts, or imported geometry before downstream operators (pushpull, subdivide) choke.
- **Technical Design & Implementation**: New node `recalculate_normals` in `modify.rs` / `model.rs` traversing half-edge connectivity across manifold components, matching face winding orientation, and checking volume sign to orient outward.

### 5. Inset Face Depth & Boundary Edge Rail
- **Category**: Mesh Edit Tools
- **Implementation Size**: **S**
- **Why High Value for Procedural Modelling**: Adds simultaneous push/pull extrusion height (`depth`) directly inside `inset`, plus boundary sliding so open mesh boundaries don't pinch.
- **Technical Design & Implementation**: Add `depth` (float) and `edge_rail` (bool) parameters to `pushpull::INSET` in `pushpull.rs:780`, and forward them to `ui/modal_model.rs:188` (key `I`).

### 6. Tris to Quads Conversion
- **Category**: Mesh Edit Tools
- **Implementation Size**: **M**
- **Why High Value for Procedural Modelling**: Cleans up triangulated boolean cuts, CAD tessellations, and photogrammetry imports into quad-dominant topology for subdivision.
- **Technical Design & Implementation**: New node `tris_to_quads` in `model.rs` / `modeling_tools.rs` using maximum-weight matching on adjacent coplanar triangle pairs within max angle threshold.

### 7. Bevel Mitering (Patch / Arc) & Harden Normals
- **Category**: Mesh Edit Tools / Modifiers
- **Implementation Size**: **M**
- **Why High Value for Procedural Modelling**: Eliminates overlapping corner artifacts at multi-edge junctions on hard-surface bevels; harden normals aligns vertex normals to adjacent large faces for clean shading.
- **Technical Design & Implementation**: Implement `miter_outer` ('sharp', 'patch', 'arc') and `harden_normals` (bool) in `polybevel` (`bevel.rs:18`), transferring face normals to bevel fillet points.

### 8. Loop Cut Multi-Cut Offset & Edge-Rail Sliding
- **Category**: Mesh Edit Tools
- **Implementation Size**: **M**
- **Why High Value for Procedural Modelling**: Fixes current limitation where multi-cuts (>1) cannot be slid and are forced to stay evenly spaced. Adds even/flip edge rail distance sliding.
- **Technical Design & Implementation**: Upgrade `loop_cut_fractions` in `modeling_tools.rs:653` to apply slide offsets to all cuts, and add rail projection in `ui/mesh_cut.rs:160`.

### 9. Interactive Face / Edge Creation ('F' Key)
- **Category**: Mesh Edit Tools
- **Implementation Size**: **S**
- **Why High Value for Procedural Modelling**: Single most used interactive hotkey in Blender edit mode. Connects 2 vertices with an edge, 3-4 vertices with a face, or fills a closed edge loop.
- **Technical Design & Implementation**: Add 'F' shortcut in `ui/keymap.rs` and `ui/modal_model.rs` that detects current selection (2 points -> `convert_line`, 3+ points/edges -> `polyfill` or face builder node).

### 10. Knife Cut-Through & Angle/Midpoint Snapping
- **Category**: Mesh Edit Tools
- **Implementation Size**: **M**
- **Why High Value for Procedural Modelling**: Enables cutting straight through volumes/hollow parts (cut-through toggle 'Z') and snapping knife segments to 45/90 degree angles ('C') or edge midpoints.
- **Technical Design & Implementation**: Add `cut_through: bool` in `knife.rs:17` by continuing ray traces across all backfaces; bind 'Z' and 'C' keys in `ui/mesh_cut.rs:100` session.

### 11. Follow Active Quads UV Unwrapping
- **Category**: UV Tools
- **Implementation Size**: **M**
- **Why High Value for Procedural Modelling**: Unwraps continuous quad strips (pipes, roads, trimsheets, architectural mouldings) into straight rectangular UV strips.
- **Technical Design & Implementation**: New node `uv_follow_active_quads` in `uv.rs` that takes an active seed quad and unwraps connected quad strips into an orthogonal UV grid.

### 12. Dedicated Wireframe Node
- **Category**: Modifiers: Generate
- **Implementation Size**: **S**
- **Why High Value for Procedural Modelling**: Eliminates having to build an unwieldy 2-node graph (`convert_line` -> `sweep`) every time a wireframe / cage lattice is needed.
- **Technical Design & Implementation**: New node `wireframe` in `rust/crates/minihou-core/src/nodes/edge_tools.rs` with `thickness`, `offset`, and `replace_original` parameters.

### 13. Curved Bridge Edge Loops Interpolation & Curvature
- **Category**: Mesh Edit Tools
- **Implementation Size**: **M**
- **Why High Value for Procedural Modelling**: Allows bridging between edge loops with smooth cubic/Hermite curves or custom profile curves (creating organic necks, handles, arches).
- **Technical Design & Implementation**: Add `interpolation: cubic | linear` and `smoothness: f64` in `polybridge` (`edge_tools.rs:257`) using Hermite spline tangents from boundary face normals.

### 14. Interactive Separate ('P' Key) & Fork Branch
- **Category**: Mesh Edit Tools / Object
- **Implementation Size**: **S**
- **Why High Value for Procedural Modelling**: Quickly splits selected faces/elements into a separate procedural stream without manual copy-pasting of blast nodes in the network editor.
- **Technical Design & Implementation**: Add 'P' key in `ui/keymap.rs` and `ui/modal_model.rs` that inserts a `blast` node on the current branch and forks a complementary `blast` (invert=true) node.

### 15. Curve Per-Point Tilt & Normal Rotation
- **Category**: Curve Tools
- **Implementation Size**: **S**
- **Why High Value for Procedural Modelling**: Allows banking and twisting swept ribbons, tracks, and road profiles along 3D splines per control point instead of only a global sweep twist.
- **Technical Design & Implementation**: Add float attribute `tilt` on curve points, evaluated during Frenet-Serret / minimal-rotation frame calculation in `curves.rs:338` (`sweep`) and `curves.rs:705` (`path_deform`).

## (d) Blender-Style Interactive Tools That Exist as Operators but Not as MiniHou Viewport Tools

The table below details operators that exist in Blender's 3D Viewport but have **no direct interactive viewport modal/binding in MiniHou** (even if a procedural background node partially exists in MiniHou's core engine):

| Viewport Tool | Blender Operator | MiniHou Current Status | Description & Impact |
| :--- | :--- | :--- | :--- |
| **Edge Slide (`GG`)** | `bpy.ops.transform.edge_slide` | MISSING in core & viewport | Double-tapping `G` slides picked edge loops along adjacent faces. MiniHou only slides newly created loop cuts during `Ctrl+R` session (`mesh_cut.rs`), not existing edges. |
| **Shrink / Fatten Modal (`Alt+S`)** | `bpy.ops.transform.shrink_fatten` | HAVE in node (`transform.rs:10` `peak`), MISSING in viewport | Dragging vertices along their normals interactively with real-time numeric readout. MiniHou has `peak` and `soft_peak` nodes, but no `Alt+S` modal transform in `ui/modal_transform.rs`. |
| **Face / Edge Fill ('F')** | `bpy.ops.mesh.fill` | PARTIAL in node (`model.rs:366` `polyfill`), MISSING in viewport | Pressing `F` with 2 vertices selected creates an edge; with 3+ vertices/edges creates an n-gon or quad face. MiniHou requires adding a `polyfill` node manually in the network editor. |
| **Merge Modal ('M')** | `bpy.ops.mesh.merge` | PARTIAL in node (`modeling_tools.rs:26` `edge_collapse`), MISSING in viewport | Pressing `M` opens a popup to merge selected elements at Center, at Cursor, Collapse, or First/Last. MiniHou only collapses edges or fuses within distance via nodes. |
| **Separate Modal ('P')** | `bpy.ops.mesh.separate` | PARTIAL in node (`group.rs:169` `blast`), MISSING in viewport | Pressing `P` separates selected elements into a separate mesh object. MiniHou has `X`/`Delete` to delete, but no quick fork-to-new-object shortcut. |
| **Rip Element ('V')** | `bpy.ops.mesh.rip` | MISSING in core & viewport | Pressing `V` tears vertices/edges open along the pointer motion and immediately enters interactive move mode. |
| **To Sphere Warp (`Shift+Alt+S`)** | `bpy.ops.transform.tosphere` | MISSING in core & viewport | Dragging warps selected geometry into a sphere around the selection pivot. |
| **Knife Cut-Through Toggle ('Z') & Angle Constraint ('C')** | `bpy.ops.mesh.knife_tool` | PARTIAL in viewport (`mesh_cut.rs:100`) | MiniHou's `K` tool only cuts visible front faces. Pressing `Z` to cut all the way through the mesh or `C` to constrain cut angles is absent. |
| **Curve Point Extrude ('E' in Curve Edit Mode)** | `bpy.ops.curve.extrude` | PARTIAL in viewport (`curve_tool.rs`) | In Blender curve editing, selecting a curve endpoint and pressing `E` extrudes a new Bezier segment. MiniHou only allows dragging existing handles or drawing fresh curves in `sketch.rs`. |
| **Extrude Individual Faces Modal Toggle** | `bpy.ops.mesh.extrude_faces_move` | HAVE in node (`model.rs:384`), MISSING in viewport modal | MiniHou's `E` modal always runs connected region extrusion. It lacks a toggle key during modal drag to switch to individual face normals. |
| **Interactive Edge Crease Modal (`Shift+E`)** | `bpy.ops.transform.edge_crease` | HAVE in node (`edge_tools.rs:222`), MISSING in viewport | Interactive mouse drag to change semi-sharp crease weight between 0.0 and 1.0 on selected edges. |
| **Select Similar Popover (`Shift+G`)** | `bpy.ops.mesh.select_similar` | PARTIAL in core, MISSING in viewport | Shortcut `Shift+G` opens a menu to select elements by area, perimeter, coplanar, vertex count, or normal. MiniHou only has 'Select Coplanar' in the Face tab. |
