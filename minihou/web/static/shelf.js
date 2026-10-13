/* The tool shelf: Houdini-style tabs of one-click tools under the top bar (Create, Modify, Model, Polygon, Deform...).
 *
 * A tool does what the same tool in the C pie menu does (pie.js): sources are added unwired, other tools are inserted after the
 * node you are looking at and filled in from your 3-D selection. Drag a tool onto the graph to drop it where you want.
 * Only tools this engine has are shown; the Library tab lists your own nodes and saved systems. */
(function () {
  "use strict";
  const T = (label, type, icon) => ({ label, type, icon: icon || type });

  // [tab, colour, tools]. Every built-in node is on at least one tab (tests/test_shelf.py checks).
  const TABS = [
    ["Create", "#6fbf73", [
      T("Box", "box"), T("Sphere", "sphere"), T("Tube", "tube"), T("Torus", "torus"), T("Grid", "grid"), T("Null", "null"),
      T("Line", "line"), T("Circle", "circle"), T("Bezier", "bezier", "curve"), T("Polyline", "curve"), T("Helix", "helix"), T("L-System", "lsystem"),
      T("Lattice Grid", "lattice_grid", "lattice"), T("Metaball", "points_to_surface"), T("Blender Object", "blender_object", "file"), T("Blender Image", "blender_image", "file"), T("Image File", "image_file", "file"), T("USD Import", "usd_import", "file"),
      T("Host Mesh", "modifier_input", "host"), T("Object Input", "object_input", "host")]],
    ["Modify", "#5ea0e0", [
      T("Transform", "transform"), T("Merge", "merge"), T("Duplicate", "copy_transform", "duplicate"), T("Copy to Points", "copy_to_points"),
      T("Instance", "instance"), T("Switch", "switch"), T("Match Size", "match_size"), T("Bound", "bound"), T("Blast", "blast"),
      T("Delete", "delete"), T("Fuse", "fuse"), T("Clean", "clean"), T("Sort", "sort"), T("Null", "null"), T("Output", "output"), T("Scene Objects", "scene_objects", "output"), T("Image Out", "image_out", "output"), T("USD Export", "usd_export", "output"),
      T("Input", "input", "host")]],
    ["Model", "#d99a4e", [
      T("Boolean", "boolean"), T("Clip", "clip"), T("Mirror", "mirror"), T("Subdivide", "subdivide"), T("Skin", "skin"),
      T("Skin Profiles", "skin_profiles", "skin"), T("Revolve", "revolve"), T("Sweep", "sweep"), T("Fillet", "fillet"),
      T("PolyReduce", "polyreduce"), T("Convex Hull", "convex_hull"), T("Remesh", "remesh")]],
    ["Polygon", "#c97bd0", [
      T("PolyExtrude", "polyextrude"), T("Push / Pull", "pushpull", "polyextrude"), T("Inset", "inset", "polyexpand2d"), T("Outset", "outset", "polyexpand2d"), T("PolyBevel", "polybevel"), T("PolySplit", "polysplit"), T("PolyFill", "polyfill"),
      T("PolyBridge", "polybridge"), T("PolyExpand2D", "polyexpand2d"), T("Dissolve", "dissolve"), T("Divide", "divide"),
      T("Reverse", "reverse"), T("Facet", "facet"), T("Bridge", "bridge", "polybridge"), T("Normal", "normal")]],
    ["Edges", "#e0c05a", [
      T("Dissolve Edges", "dissolve_edges", "dissolve"), T("Edge Collapse", "edge_collapse"), T("Edge Flip", "edge_flip"),
      T("Edge Cusp", "edge_cusp"), T("Crease", "crease"), T("Edge Group", "edge_group", "group"),
      T("Edge Group Promote", "edge_group_promote", "group")]],
    ["Deform", "#e0705a", [
      T("Bend", "bend"), T("Twist", "twist"), T("Linear Taper", "linear_taper", "taper"), T("Squash and Stretch", "squash_stretch", "squash"),
      T("Soft Transform", "soft_transform", "transform"), T("Soft Peak", "soft_peak", "peak"), T("Peak", "peak"), T("Smooth", "smooth"),
      T("Mountain", "noise"), T("Lattice", "lattice"), T("Point Deform", "point_deform", "lattice"), T("Path Deform", "path_deform", "curve"),
      T("Point Jitter", "point_jitter", "scatter"), T("Point Relax", "point_relax", "scatter"), T("Blend", "blend"),
      T("Exploded View", "exploded_view", "fracture")]],
    ["Texture", "#8f8fe6", [T("UV Project", "uv_project", "uv"), T("UV Flatten", "uv_flatten", "uv"), T("Color", "color")]],
    ["Groups", "#4fc3c3", [
      T("Group Create", "group_create", "group"), T("Group Expand", "group_expand", "group"), T("Group Promote", "group_promote", "group"),
      T("Group by Range", "group_range", "group"), T("Blast", "blast"), T("Delete", "delete")]],
    ["Attributes", "#8bb9c9", [
      T("Wrangle", "wrangle", "code"), T("Attribute Create", "attribute_create", "attr"), T("Attribute Transfer", "attribute_transfer", "attr"),
      T("Attribute Promote", "attribute_promote", "attr"), T("Attribute Randomize", "attribute_randomize", "attr"),
      T("Attribute Blur", "attribute_blur", "attr"), T("Attribute Rename", "attribute_rename", "attr"), T("Attribute Paint", "attrib_paint", "attr"),
      T("Attribute Delete", "attribute_delete", "attr"), T("Measure", "measure"), T("Connectivity", "connectivity"), T("Ray", "ray"),
      T("Set Material", "set_material", "attr"),
      T("Python", "python", "code")]],
    ["Curves", "#7fd0a0", [
      T("Bezier", "bezier", "curve"), T("Polyline", "curve"), T("Line", "line"), T("Helix", "helix"), T("Resample", "resample"), T("Carve", "carve"),
      T("Convert Line", "convert_line", "line"), T("PolyPath", "polypath", "curve"), T("Sweep", "sweep"), T("Fillet", "fillet")]],
    ["Points", "#d6d46f", [
      T("Scatter", "scatter"), T("Fuse", "fuse"), T("Delaunay", "delaunay", "hull"), T("Point Jitter", "point_jitter", "scatter"),
      T("Point Relax", "point_relax", "scatter"), T("Points to Surface", "points_to_surface")]],
    ["Rigid Bodies", "#e88a7a", [
      T("RBD Material Fracture", "rbd_material_fracture", "fracture"), T("Voronoi Fracture", "voronoi_fracture", "fracture"),
      T("Voronoi Fracture Points", "voronoi_fracture_points", "scatter"), T("Voronoi Split", "voronoi_split", "fracture"),
      T("Voronoi Adjacency", "voronoi_adjacency", "constraint"), T("Boolean Fracture", "boolean_fracture", "fracture"),
      T("Convex Decomposition", "convex_decomposition", "hull"), T("Assemble", "assemble", "duplicate"), T("RBD Configure", "rbd_configure", "solver"),
      T("RBD Solver", "rbd_solver", "solver"), T("Exploded View", "exploded_view", "fracture")]],
    ["Constraints", "#b0b0e0", [
      T("RBD Constraint", "rbd_constraint", "constraint"), T("Glue Cluster", "glue_cluster", "constraint"), T("RBD Constraint Properties", "rbd_constraint_properties", "constraint"),
      T("Voronoi Adjacency", "voronoi_adjacency", "constraint")]],
    ["Terrain FX", "#9c8b6a", [T("Grid", "grid"), T("Mountain", "noise"), T("Heightfield Erode", "heightfield_erode", "terrain"), T("Scatter", "scatter")]],
    ["Volume", "#6ab0e0", [T("Remesh", "remesh"), T("SDF Boolean", "sdf_boolean", "boolean"), T("Points to Surface", "points_to_surface")]],
    ["PCG", "#79c27a", null],                                                   // every pcg_* node (filled in from the engine's types)
    ["Procedural", "#79c2ad", [
      T("Building Rules", "building_rules", "box"), T("Region Identify", "region_identify", "connectivity"),
      T("Region Transform", "region_transform", "transform"), T("Goal Length", "goal_length", "constraint"),
      T("Goal Collision", "goal_collision", "constraint"), T("Goal Surface", "goal_surface", "constraint"),
      T("Goal Gravity", "goal_gravity", "constraint"), T("Goal Pin", "goal_pin", "constraint"),
      T("Goal Solve", "goal_solve", "solver")]],
    ["Flow", "#c4a35a", [
      T("Repeat", "loop_block", "loop"), T("Loop Begin", "loop_begin", "loop"), T("Loop End", "loop_end", "loop"), T("For Each", "for_each", "loop"),
      T("Solver", "solver", "loop"), T("Switch", "switch")]],
    ["Library", "#e8a33d", null],                                              // your own nodes and saved systems
  ];
  const ALWAYS = ["Create", "Modify", "Model", "Polygon", "Deform", "Texture", "Rigid Bodies", "Constraints", "Library"];
  const KEY = "minihou.shelf";

  // ---- icons: small drawings in a 32 x 32 box; tools without one get a tinted chip with their initials
  const NEUTRAL = "#cfd6dd", DARK = "#7d8791", MID = "#a8b1ba";
  const ICONS = {
    box: (c) => `<polygon points="16,4 28,10 16,16 4,10" fill="${NEUTRAL}"/><polygon points="4,10 16,16 16,29 4,23" fill="${MID}"/><polygon points="28,10 16,16 16,29 28,23" fill="${DARK}"/>`,
    sphere: (c) => `<defs><radialGradient id="g-sph" cx="35%" cy="30%" r="75%"><stop offset="0" stop-color="#fff"/><stop offset="1" stop-color="${DARK}"/></radialGradient></defs><circle cx="16" cy="16" r="12" fill="url(#g-sph)"/>`,
    tube: (c) => `<rect x="8" y="8" width="16" height="16" fill="${MID}"/><ellipse cx="16" cy="24" rx="8" ry="3.5" fill="${DARK}"/><ellipse cx="16" cy="8" rx="8" ry="3.5" fill="${NEUTRAL}"/>`,
    torus: (c) => `<ellipse cx="16" cy="17" rx="12" ry="9" fill="none" stroke="${MID}" stroke-width="6"/><ellipse cx="16" cy="15" rx="12" ry="8" fill="none" stroke="${NEUTRAL}" stroke-width="2"/>`,
    grid: (c) => `<polygon points="4,22 10,8 28,8 22,22" fill="${DARK}"/><g stroke="${NEUTRAL}" stroke-width="1"><path d="M7,15h18M13,8l-6,14M19,8l-6,14M25,8l-6,14"/></g>`,
    null: (c) => `<path d="M16,16 L16,4" stroke="#6fe06f" stroke-width="2.5"/><path d="M16,16 L27,16" stroke="#e06f6f" stroke-width="2.5"/><path d="M16,16 L7,24" stroke="#6f9fe0" stroke-width="2.5"/><circle cx="16" cy="16" r="2.5" fill="${NEUTRAL}"/>`,
    line: (c) => `<path d="M6,26 L26,6" stroke="${NEUTRAL}" stroke-width="2.2"/><circle cx="6" cy="26" r="3" fill="${c}"/><circle cx="26" cy="6" r="3" fill="${c}"/>`,
    circle: (c) => `<circle cx="16" cy="16" r="11" fill="none" stroke="${NEUTRAL}" stroke-width="2.2"/><circle cx="27" cy="16" r="2.4" fill="${c}"/>`,
    curve: (c) => `<path d="M5,24 C10,2 22,30 27,8" fill="none" stroke="${NEUTRAL}" stroke-width="2.2"/><circle cx="5" cy="24" r="2.6" fill="${c}"/><circle cx="27" cy="8" r="2.6" fill="${c}"/>`,
    helix: (c) => `<path d="M8,26 C24,26 24,21 8,21 C24,21 24,16 8,16 C24,16 24,11 8,11 C24,11 24,6 8,6" fill="none" stroke="${c}" stroke-width="2.2"/>`,
    lsystem: (c) => `<path d="M16,28 V14 M16,22 L9,15 M16,18 L23,11 M9,15 L7,8 M23,11 L25,5 M16,14 L16,6" stroke="${c}" stroke-width="2.2" fill="none"/>`,
    lattice: (c) => `<g stroke="${DARK}" stroke-width="1.2"><path d="M6,6h20v20H6z M6,16h20 M16,6v20"/></g><g fill="${c}"><circle cx="6" cy="6" r="2.2"/><circle cx="16" cy="6" r="2.2"/><circle cx="26" cy="6" r="2.2"/><circle cx="6" cy="16" r="2.2"/><circle cx="16" cy="16" r="2.2"/><circle cx="26" cy="16" r="2.2"/><circle cx="6" cy="26" r="2.2"/><circle cx="16" cy="26" r="2.2"/><circle cx="26" cy="26" r="2.2"/></g>`,
    points_to_surface: (c) => `<circle cx="12" cy="17" r="8" fill="${c}"/><circle cx="21" cy="13" r="6" fill="${MID}"/><path d="M14,14 Q17,12 18,15" stroke="${c}" stroke-width="5" fill="none"/>`,
    file: (c) => `<path d="M4,9 h8 l3,3 h13 v14 H4z" fill="${c}"/><path d="M4,13 h24 v13 H4z" fill="${NEUTRAL}" opacity=".85"/>`,
    host: (c) => `<rect x="12" y="8" width="16" height="16" fill="${MID}"/><path d="M3,16 H14 M10,11 l5,5 l-5,5" stroke="${c}" stroke-width="2.4" fill="none"/>`,
    transform: (c) => `<path d="M16,3 v26 M3,16 h26" stroke="${NEUTRAL}" stroke-width="2"/><g fill="${c}"><path d="M16,2 l4,5 h-8z"/><path d="M16,30 l4,-5 h-8z"/><path d="M2,16 l5,-4 v8z"/><path d="M30,16 l-5,-4 v8z"/></g>`,
    merge: (c) => `<path d="M4,7 H14 L22,16 L14,25 H4" fill="none" stroke="${NEUTRAL}" stroke-width="2.2"/><path d="M22,16 H29 M25,12 l4,4 l-4,4" stroke="${c}" stroke-width="2.4" fill="none"/>`,
    duplicate: (c) => `<rect x="4" y="4" width="16" height="16" fill="${DARK}"/><rect x="12" y="12" width="16" height="16" fill="${NEUTRAL}" stroke="${c}" stroke-width="1.5"/>`,
    copy_to_points: (c) => `<g fill="${c}"><circle cx="6" cy="8" r="2.4"/><circle cx="16" cy="6" r="2.4"/><circle cx="26" cy="9" r="2.4"/></g><g fill="${NEUTRAL}"><rect x="3" y="15" width="8" height="8"/><rect x="13" y="13" width="8" height="8"/><rect x="23" y="16" width="7" height="7"/></g>`,
    instance: (c) => `<g fill="${NEUTRAL}"><rect x="3" y="3" width="9" height="9"/><rect x="20" y="3" width="9" height="9"/><rect x="11" y="20" width="9" height="9"/></g><path d="M12,8 H20 M7,12 L14,20 M25,12 L18,20" stroke="${c}" stroke-width="1.6" fill="none"/>`,
    switch: (c) => `<rect x="4" y="11" width="24" height="10" rx="5" fill="${DARK}"/><circle cx="23" cy="16" r="4" fill="${c}"/>`,
    match_size: (c) => `<rect x="3" y="3" width="26" height="26" fill="none" stroke="${c}" stroke-width="1.6" stroke-dasharray="3 2"/><rect x="10" y="10" width="12" height="12" fill="${NEUTRAL}"/>`,
    bound: (c) => `<polygon points="16,4 28,10 16,16 4,10" fill="none" stroke="${c}" stroke-width="1.6" stroke-dasharray="3 2"/><polygon points="4,10 16,16 16,29 4,23" fill="none" stroke="${c}" stroke-width="1.6" stroke-dasharray="3 2"/><polygon points="28,10 16,16 16,29 28,23" fill="none" stroke="${c}" stroke-width="1.6" stroke-dasharray="3 2"/>`,
    blast: (c) => `<rect x="5" y="9" width="20" height="18" fill="${DARK}"/><path d="M11,6 L29,24 M29,6 L11,24" stroke="${c}" stroke-width="3"/>`,
    delete: (c) => `<path d="M7,7 L25,25 M25,7 L7,25" stroke="${c}" stroke-width="3.4"/>`,
    fuse: (c) => `<g fill="${NEUTRAL}"><circle cx="6" cy="8" r="2.6"/><circle cx="26" cy="8" r="2.6"/><circle cx="6" cy="24" r="2.6"/><circle cx="26" cy="24" r="2.6"/></g><path d="M8,9 L14,15 M24,9 L18,15 M8,23 L14,17 M24,23 L18,17" stroke="${c}" stroke-width="1.6"/><circle cx="16" cy="16" r="4" fill="${c}"/>`,
    sort: (c) => `<g fill="${c}"><rect x="5" y="6" width="22" height="4"/><rect x="5" y="14" width="16" height="4"/><rect x="5" y="22" width="9" height="4"/></g>`,
    output: (c) => `<path d="M4,18 v8 h24 v-8" fill="none" stroke="${NEUTRAL}" stroke-width="2.2"/><path d="M16,4 v16 M10,14 l6,6 l6,-6" stroke="${c}" stroke-width="2.6" fill="none"/>`,
    boolean: (c) => `<circle cx="12" cy="16" r="9" fill="${c}" opacity=".75"/><circle cx="20" cy="16" r="9" fill="${NEUTRAL}" opacity=".75"/>`,
    mirror: (c) => `<path d="M16,3 v26" stroke="${c}" stroke-width="1.6" stroke-dasharray="3 2"/><polygon points="14,8 4,24 14,24" fill="${NEUTRAL}"/><polygon points="18,8 28,24 18,24" fill="${DARK}"/>`,
    subdivide: (c) => `<rect x="5" y="5" width="22" height="22" fill="${DARK}"/><path d="M16,5 v22 M5,16 h22" stroke="${NEUTRAL}" stroke-width="1.6"/><circle cx="16" cy="16" r="9" fill="none" stroke="${c}" stroke-width="2"/>`,
    skin: (c) => `<ellipse cx="16" cy="6" rx="9" ry="3" fill="none" stroke="${NEUTRAL}" stroke-width="1.8"/><ellipse cx="16" cy="26" rx="12" ry="3.5" fill="none" stroke="${NEUTRAL}" stroke-width="1.8"/><path d="M7,6 L4,26 M25,6 L28,26" stroke="${c}" stroke-width="1.8"/>`,
    revolve: (c) => `<path d="M16,4 v24" stroke="${DARK}" stroke-width="1.5" stroke-dasharray="3 2"/><path d="M22,6 C30,10 30,22 22,26 M5,8 h8" stroke="${c}" stroke-width="2.4" fill="none"/><path d="M22,26 l-5,0 l3,-4" fill="${c}"/>`,
    sweep: (c) => `<path d="M4,26 C10,10 20,26 28,8" fill="none" stroke="${MID}" stroke-width="7" stroke-linecap="round"/><path d="M4,26 C10,10 20,26 28,8" fill="none" stroke="${c}" stroke-width="2"/>`,
    fillet: (c) => `<path d="M5,27 V12 Q5,5 12,5 H27" fill="none" stroke="${NEUTRAL}" stroke-width="2.4"/><circle cx="12" cy="12" r="1.6" fill="${c}"/>`,
    polyreduce: (c) => `<polygon points="16,4 28,12 24,26 8,26 4,12" fill="${DARK}" stroke="${NEUTRAL}" stroke-width="1.4"/><path d="M16,4 L16,26 M4,12 L28,12" stroke="${c}" stroke-width="1.4"/>`,
    convex_hull: (c) => `<polygon points="16,3 29,13 24,28 8,28 3,13" fill="none" stroke="${c}" stroke-width="1.8"/><g fill="${NEUTRAL}"><circle cx="16" cy="14" r="2.2"/><circle cx="11" cy="20" r="2.2"/><circle cx="21" cy="19" r="2.2"/></g>`,
    hull: (c) => `<polygon points="16,3 29,13 24,28 8,28 3,13" fill="none" stroke="${c}" stroke-width="1.8"/><g fill="${NEUTRAL}"><circle cx="16" cy="14" r="2.2"/><circle cx="11" cy="20" r="2.2"/><circle cx="21" cy="19" r="2.2"/></g>`,
    remesh: (c) => `<polygon points="4,24 16,4 28,24" fill="${DARK}"/><path d="M10,14 H22 M7,19 H25 M16,4 V24 M10,14 L13,24 M22,14 L19,24" stroke="${NEUTRAL}" stroke-width="1.2"/>`,
    polyextrude: (c) => `<polygon points="4,18 16,24 28,18 16,12" fill="${DARK}"/><polygon points="4,18 4,26 16,31 16,24" fill="${MID}"/><polygon points="16,24 16,31 28,26 28,18" fill="${DARK}"/><path d="M16,10 V2 M11,6 l5,-4 l5,4" stroke="${c}" stroke-width="2.4" fill="none"/>`,
    polybevel: (c) => `<path d="M5,27 V12 L12,5 H27 V27z" fill="${DARK}" stroke="${NEUTRAL}" stroke-width="1.6"/><path d="M5,12 L12,5" stroke="${c}" stroke-width="3.4"/>`,
    polysplit: (c) => `<rect x="5" y="5" width="22" height="22" fill="${DARK}"/><path d="M5,16 H27" stroke="${c}" stroke-width="2.6"/><circle cx="5" cy="16" r="2.2" fill="${NEUTRAL}"/><circle cx="27" cy="16" r="2.2" fill="${NEUTRAL}"/>`,
    polyfill: (c) => `<polygon points="6,8 22,4 28,16 20,28 6,22" fill="${c}" opacity=".85"/><polygon points="6,8 22,4 28,16 20,28 6,22" fill="none" stroke="${NEUTRAL}" stroke-width="1.6"/>`,
    polybridge: (c) => `<rect x="3" y="6" width="7" height="20" fill="${NEUTRAL}"/><rect x="22" y="6" width="7" height="20" fill="${NEUTRAL}"/><path d="M10,9 H22 V23 H10z" fill="${c}" opacity=".7"/>`,
    polyexpand2d: (c) => `<rect x="9" y="9" width="14" height="14" fill="none" stroke="${NEUTRAL}" stroke-width="2"/><rect x="3" y="3" width="26" height="26" fill="none" stroke="${c}" stroke-width="1.6" stroke-dasharray="3 2"/>`,
    dissolve: (c) => `<rect x="4" y="4" width="24" height="24" fill="${DARK}"/><path d="M16,4 V28" stroke="${c}" stroke-width="2" stroke-dasharray="2 2"/>`,
    divide: (c) => `<polygon points="4,26 28,26 16,5" fill="${DARK}"/><path d="M16,5 L16,26 M4,26 L22,16" stroke="${c}" stroke-width="1.8"/>`,
    reverse: (c) => `<path d="M5,11 H25 M20,6 l6,5 l-6,5 M27,21 H7 M12,16 l-6,5 l6,5" stroke="${c}" stroke-width="2.4" fill="none"/>`,
    facet: (c) => `<polygon points="16,3 29,12 24,27 8,27 3,12" fill="${DARK}"/><polygon points="16,3 29,12 16,16" fill="${NEUTRAL}"/><polygon points="3,12 16,16 8,27" fill="${MID}"/>`,
    normal: (c) => `<rect x="4" y="18" width="24" height="9" fill="${DARK}"/><g stroke="${c}" stroke-width="2"><path d="M9,18 V6 M16,18 V4 M23,18 V6"/></g>`,
    edge_collapse: (c) => `<path d="M4,16 H11 M28,16 H21" stroke="${NEUTRAL}" stroke-width="2.4"/><path d="M11,10 l5,6 l-5,6 M21,10 l-5,6 l5,6" stroke="${c}" stroke-width="2.2" fill="none"/>`,
    edge_flip: (c) => `<polygon points="5,5 27,5 27,27 5,27" fill="${DARK}"/><path d="M5,5 L27,27" stroke="${NEUTRAL}" stroke-width="2"/><path d="M27,5 L5,27" stroke="${c}" stroke-width="2" stroke-dasharray="3 2"/>`,
    edge_cusp: (c) => `<path d="M4,22 L16,8 L28,22" fill="none" stroke="${NEUTRAL}" stroke-width="3"/><circle cx="16" cy="8" r="3" fill="${c}"/>`,
    crease: (c) => `<rect x="4" y="8" width="24" height="16" fill="${DARK}"/><path d="M4,16 H28" stroke="${c}" stroke-width="3"/>`,
    group: (c) => `<rect x="4" y="4" width="24" height="24" fill="none" stroke="${c}" stroke-width="2" stroke-dasharray="4 3"/><g fill="${NEUTRAL}"><circle cx="11" cy="12" r="2.4"/><circle cx="20" cy="11" r="2.4"/><circle cx="15" cy="21" r="2.4"/></g>`,
    code: (c) => `<path d="M11,9 l-6,7 l6,7 M21,9 l6,7 l-6,7" stroke="${c}" stroke-width="2.6" fill="none"/>`,
    attr: (c) => `<g fill="${NEUTRAL}"><rect x="4" y="5" width="6" height="4"/><rect x="4" y="14" width="6" height="4"/><rect x="4" y="23" width="6" height="4"/></g><g fill="${c}"><rect x="13" y="5" width="15" height="4"/><rect x="13" y="14" width="15" height="4"/><rect x="13" y="23" width="15" height="4"/></g>`,
    measure: (c) => `<path d="M4,22 L22,4 L28,10 L10,28z" fill="${DARK}" stroke="${NEUTRAL}" stroke-width="1.4"/><path d="M10,18 l3,3 M14,14 l3,3 M18,10 l3,3" stroke="${c}" stroke-width="1.6"/>`,
    connectivity: (c) => `<g fill="${c}"><circle cx="8" cy="9" r="4"/><circle cx="24" cy="9" r="4" fill="${NEUTRAL}"/><circle cx="16" cy="24" r="4" fill="${MID}"/></g>`,
    ray: (c) => `<path d="M4,6 L22,24" stroke="${c}" stroke-width="2.2"/><path d="M12,27 H28" stroke="${NEUTRAL}" stroke-width="3"/><circle cx="22" cy="24" r="3" fill="${NEUTRAL}"/>`,
    resample: (c) => `<path d="M4,22 C10,6 22,6 28,22" fill="none" stroke="${DARK}" stroke-width="2"/><g fill="${c}"><circle cx="4" cy="22" r="2.4"/><circle cx="10" cy="11" r="2.4"/><circle cx="16" cy="8" r="2.4"/><circle cx="22" cy="11" r="2.4"/><circle cx="28" cy="22" r="2.4"/></g>`,
    carve: (c) => `<path d="M4,24 C10,6 22,6 28,24" fill="none" stroke="${DARK}" stroke-width="2"/><path d="M10,12 C13,8 19,8 22,12" fill="none" stroke="${c}" stroke-width="3.4"/>`,
    scatter: (c) => `<g fill="${c}"><circle cx="6" cy="8" r="2.4"/><circle cx="14" cy="5" r="2"/><circle cx="25" cy="9" r="2.6"/><circle cx="10" cy="17" r="2.6"/><circle cx="21" cy="18" r="2.2"/><circle cx="5" cy="26" r="2"/><circle cx="16" cy="26" r="2.6"/><circle cx="27" cy="26" r="2"/></g>`,
    noise: (c) => `<path d="M3,22 L8,12 L12,18 L18,6 L23,17 L29,10" fill="none" stroke="${c}" stroke-width="2.6"/><path d="M3,27 H29" stroke="${DARK}" stroke-width="2"/>`,
    bend: (c) => `<path d="M5,26 C5,10 12,6 27,6" fill="none" stroke="${NEUTRAL}" stroke-width="5"/><path d="M5,26 C5,10 12,6 27,6" fill="none" stroke="${c}" stroke-width="1.6" stroke-dasharray="3 2"/>`,
    twist: (c) => `<path d="M10,4 C24,10 8,16 22,22 M10,10 C24,16 8,22 22,28" fill="none" stroke="${NEUTRAL}" stroke-width="2.4"/><path d="M8,4 H24 M8,28 H24" stroke="${c}" stroke-width="2"/>`,
    taper: (c) => `<polygon points="5,26 27,26 21,6 11,6" fill="${DARK}" stroke="${NEUTRAL}" stroke-width="1.6"/><path d="M11,6 L21,6" stroke="${c}" stroke-width="3"/>`,
    squash: (c) => `<rect x="6" y="12" width="20" height="10" fill="${NEUTRAL}"/><path d="M16,3 V9 M12,6 l4,3 l4,-3 M16,29 V23 M12,26 l4,-3 l4,3" stroke="${c}" stroke-width="2" fill="none"/>`,
    peak: (c) => `<path d="M3,24 H11 Q16,2 21,24 H29" fill="none" stroke="${NEUTRAL}" stroke-width="2.4"/><circle cx="16" cy="7" r="2.6" fill="${c}"/>`,
    smooth: (c) => `<path d="M3,22 L9,8 L15,22 L21,8 L29,22" fill="none" stroke="${DARK}" stroke-width="1.6"/><path d="M3,20 C9,2 15,30 21,6 C25,0 27,16 29,20" fill="none" stroke="${c}" stroke-width="2.6"/>`,
    blend: (c) => `<circle cx="11" cy="16" r="8" fill="${c}" opacity=".7"/><rect x="14" y="8" width="15" height="15" fill="${NEUTRAL}" opacity=".75"/>`,
    fracture: (c) => `<circle cx="16" cy="16" r="12" fill="${DARK}"/><path d="M16,4 L13,12 L19,16 L12,21 L16,28 M13,12 L5,14 M19,16 L28,13 M12,21 L6,25" fill="none" stroke="${c}" stroke-width="1.8"/>`,
    solver: (c) => `<circle cx="16" cy="16" r="12" fill="none" stroke="${NEUTRAL}" stroke-width="2"/><path d="M13,10 L23,16 L13,22z" fill="${c}"/>`,
    loop: (c) => `<path d="M8,10 H22 Q28,10 28,16 Q28,22 22,22 H10 M13,6 l-5,4 l5,4 M19,18 l5,4 l-5,4" stroke="${c}" stroke-width="2.4" fill="none"/>`,
    constraint: (c) => `<rect x="2" y="12" width="12" height="8" rx="4" fill="none" stroke="${NEUTRAL}" stroke-width="2.4"/><rect x="18" y="12" width="12" height="8" rx="4" fill="none" stroke="${c}" stroke-width="2.4"/><path d="M12,16 H20" stroke="${NEUTRAL}" stroke-width="2.4"/>`,
    uv: (c) => `<g><rect x="4" y="4" width="12" height="12" fill="${NEUTRAL}"/><rect x="16" y="4" width="12" height="12" fill="${DARK}"/><rect x="4" y="16" width="12" height="12" fill="${DARK}"/><rect x="16" y="16" width="12" height="12" fill="${c}"/></g>`,
    color: (c) => `<g><rect x="4" y="6" width="10" height="20" fill="#e06f6f"/><rect x="11" y="6" width="10" height="20" fill="#6fe06f" opacity=".9"/><rect x="18" y="6" width="10" height="20" fill="#6f9fe0" opacity=".9"/></g>`,
    terrain: (c) => `<path d="M2,26 L10,10 L15,18 L21,7 L30,26z" fill="${DARK}" stroke="${NEUTRAL}" stroke-width="1.4"/><path d="M18,12 l3,-5 l3,5" fill="${c}"/>`,
  };

  function initials(label) {
    const w = label.replace(/[()]/g, "").split(/[\s-]+/).filter(Boolean);
    return (w.length > 1 ? w[0][0] + w[1][0] : label.slice(0, 2)).toUpperCase();
  }
  function iconSvg(tool, color) {
    const draw = ICONS[tool.icon] || ICONS[tool.type];
    const body = draw ? draw(color) : `<rect x="3" y="3" width="26" height="26" rx="7" fill="${color}" opacity=".9"/><text x="16" y="21" text-anchor="middle" font-size="12" font-weight="700" fill="#101418" font-family="system-ui,sans-serif">${initials(tool.label)}</text>`;
    return `<svg viewBox="0 0 32 32" width="34" height="34" aria-hidden="true">${body}</svg>`;
  }

  // ---- the shelf
  let hooks = { types: () => ({}), onPick() {}, onDrop() {}, typeList: () => [] };
  let root = null, prefs = { tab: "Create", hidden: [], collapsed: false };
  const el = (tag, attrs, ...kids) => {
    const e = document.createElement(tag);
    for (const [k, v] of Object.entries(attrs || {})) {
      if (k === "class") e.className = v;
      else if (k === "dataset") Object.assign(e.dataset, v);
      else if (k.startsWith("on")) e[k] = v;
      else if (v !== false && v != null) e.setAttribute(k, v);
    }
    for (const kid of kids.flat()) if (kid != null && kid !== false) e.append(kid.nodeType ? kid : document.createTextNode(kid));
    return e;
  };
  function load() { try { Object.assign(prefs, JSON.parse(localStorage.getItem(KEY) || "{}")); } catch (e) { /* storage unavailable */ } }
  function save() { try { localStorage.setItem(KEY, JSON.stringify(prefs)); } catch (e) { /* storage unavailable */ } }

  /* The tools of a tab that this engine has: PCG lists every pcg_* node, Library every library node. */
  function toolsOf(name) {
    const types = hooks.types();
    if (name === "PCG") return Object.keys(types).filter((n) => n.startsWith("pcg_")).sort().map((n) => T((types[n].label || n).replace(/^PCG /, ""), n, /filter|prune|density/.test(n)?"blast":/transform|rotate|scale/.test(n)?"transform":/spline|curve/.test(n)?"curve":/grid|surface/.test(n)?"grid":/attribute/.test(n)?"attrib":/merge|union/.test(n)?"merge":/sample|scatter/.test(n)?"scatter":"instance"));
    if (name === "Library") return hooks.typeList().filter((t) => t.menu === "Library").map((t) => T(t.label || t.name, t.name, "file"));
    const tab = TABS.find((x) => x[0] === name);
    return tab && tab[2] ? tab[2].filter((t) => types[t.type]) : [];
  }
  function visibleTabs() {
    const names = TABS.map((t) => t[0]);
    return names.filter((n) => !prefs.hidden.includes(n) && (toolsOf(n).length || n === "Library"));
  }
  function render() {
    if (!root || !Object.keys(hooks.types()).length) return;           // wait for the engine's node types
    load();
    const tabs = visibleTabs();
    if (!tabs.includes(prefs.tab)) prefs.tab = tabs[0] || "Create";
    const color = (TABS.find((t) => t[0] === prefs.tab) || [0, "#6fbf73"])[1];
    const row = el("div", { class: "sh-tabs" },
      ...tabs.map((n) => el("button", { class: "sh-tab" + (n === prefs.tab ? " on" : ""), dataset: { tab: n }, onclick: () => { prefs.tab = n; prefs.collapsed = false; save(); render(); } }, n)),
      el("button", { class: "sh-tab plus", title: "Choose which tabs to show", onclick: (e) => { e.stopPropagation(); tabMenu(e.currentTarget); } }, "+"),
      el("span", { class: "sh-spacer" }),
      el("button", { class: "sh-fold", title: prefs.collapsed ? "Show the tools" : "Hide the tools", onclick: () => { prefs.collapsed = !prefs.collapsed; save(); render(); } }, prefs.collapsed ? "▾" : "▴"));
    const tools = toolsOf(prefs.tab);
    const body = el("div", { class: "sh-tools" }, ...(tools.length ? tools.map((t) => {
      const type = hooks.types()[t.type] || {};
      const b = el("button", { class: "sh-tool", draggable: "true", dataset: { type: t.type, label: t.label },
        title: `${t.label} (${t.type}): ${(type.doc || "").split("\n")[0]}\nClick: add it (after the selected node, filled in from a 3-D selection). Drag: drop it on the graph.`,
        onclick: () => hooks.onPick(t, MiniPie.find(t.type, t.label)),
        ondragstart: (e) => { e.dataTransfer.setData("application/x-minihou-tool", JSON.stringify({ type: t.type, label: t.label })); e.dataTransfer.effectAllowed = "copy"; } });
      b.innerHTML = iconSvg(t, color);
      b.append(el("span", { class: "sh-label" }, t.label));
      return b;
    }) : [el("div", { class: "sh-empty" }, prefs.tab === "Library" ? "No library nodes yet: select nodes and press Ctrl+G to make one." : "Nothing here.")]));
    root.classList.toggle("collapsed", prefs.collapsed);
    root.replaceChildren(row, body);
    document.documentElement.style.setProperty("--shelf-h", (prefs.collapsed ? 27 : 107) + "px");
    root.dataset.tab = prefs.tab;
    row.querySelector(".sh-tab.on") && row.querySelector(".sh-tab.on").scrollIntoView({ block: "nearest", inline: "nearest" });
  }
  function tabMenu(anchor) {
    const old = document.getElementById("sh-tabmenu");
    if (old) { old.remove(); return; }
    const r = anchor.getBoundingClientRect();
    const m = el("div", { id: "sh-tabmenu", class: "menu", style: `position:fixed;left:${Math.min(r.left, innerWidth - 200)}px;top:${r.bottom + 2}px;min-width:180px` },
      ...TABS.map(([n, c]) => {
        const shown = !prefs.hidden.includes(n);
        return el("div", { class: "item", onmousedown: (e) => { e.preventDefault(); prefs.hidden = shown ? [...prefs.hidden, n] : prefs.hidden.filter((x) => x !== n); save(); m.remove(); render(); } },
          (shown ? "☑ " : "☐ ") + n);
      }));
    document.body.append(m);
    const away = (e) => { if (!m.contains(e.target)) { m.remove(); document.removeEventListener("mousedown", away, true); } };
    setTimeout(() => document.addEventListener("mousedown", away, true), 0);
  }

  function init(h) {
    hooks = Object.assign(hooks, h);
    root = document.getElementById("shelf");
    if (!root) return;
    load();
    render();
  }
  window.MiniShelf = { init, refresh: render, TABS, ICONS, toolsOf, state: () => ({ tab: prefs.tab, tabs: visibleTabs() }) };
})();
