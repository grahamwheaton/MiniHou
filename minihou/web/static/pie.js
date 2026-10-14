/* The "C" pie menu: Houdini's modelling tools in eight directions, with a page per category.
 *
 * A page has up to eight slots, clockwise from the top; a slot is a tool (node type + preset parameters), a sub-page, or a
 * "more" arrow to a second page of the same category. Hold C, flick towards a slot and let go; or tap C and click.
 * The editor supplies what a tool does (MiniPie.init({ onPick })). */
(function () {
  "use strict";
  const T = (label, type, extra) => Object.assign({ label, type }, extra || {});
  const P = (label, page) => ({ label, page });

  // sel: how a 3-D selection fills the tool.  A string names the parameter that receives the selection's pattern; an object
  // gives fixed parameters where "$" stands for the pattern.  from: faces | points | edges (default: what the mode selects)
  const POINTS = (name) => ({ [name]: "$", from: "points" });
  const PAGES = {
    main: { title: "Modelling", items: [P("Adjust", "adjust"), P("Deform", "deform"), P("Polygons", "polygons"), P("Group", "group"),
      P("Create", "create"), P("Build", "build"), P("Edges", "edges"), P("Modify", "modify")] },

    adjust: { title: "Adjust", items: [
      T("Transform", "transform", { sel: { face: POINTS("group"), point: "group", edge: POINTS("group") },
        tip: "Move, rotate and scale the selected points" }),
      T("Soft Transform", "soft_transform", { center: "center", tip: "Transform with a falloff around the selection" }),
      T("Soft Peak", "soft_peak", { sel: { face: POINTS("group"), point: "group", edge: POINTS("group") } }),
      T("Smooth", "smooth", { sel: { face: POINTS("group"), point: "group", edge: POINTS("group") } }),
      T("Sculpt", null, { disabled: "Sculpting is not available yet" }),
      T("Point Weld", "fuse", { tip: "Merge points that are close together (Fuse)" }),
      null,
      P("»", "adjust2")] },
    adjust2: { title: "Adjust", items: [
      T("Peak", "peak", { sel: { face: POINTS("group"), point: "group", edge: POINTS("group") } }),
      T("Point Jitter", "point_jitter"), T("Match Size", "match_size"), T("Facet", "facet"), T("Normal", "normal"),
      T("Attribute Blur", "attribute_blur"), null, null] },

    deform: { title: "Deform", items: [
      T("Lattice", "lattice", { tip: "Wire the rest and deformed lattices into the second and third inputs" }),
      T("Mountain", "noise", { params: { amplitude: [0, 0, 0.3], frequency: 1.5 }, tip: "Noise displacement (Mountain)" }),
      T("Bend", "bend"), T("Linear Taper", "linear_taper"), T("Squash and Stretch", "squash_stretch"), T("Twist", "twist"),
      null, P("»", "deform2")] },
    deform2: { title: "Deform", items: [
      T("Path Deform", "path_deform"), T("Point Deform", "point_deform"), T("Blend", "blend"),
      T("Exploded View", "exploded_view"), T("Soft Transform", "soft_transform", { center: "center" }),
      T("Point Relax", "point_relax"), null, null] },

    polygons: { title: "Polygons", items: [
      T("PolyBevel", "polybevel", { sel: { edge: "edges", face: { group: "$", group_type: "primitives" },
        point: { group: "$", group_type: "points" } } }),
      T("PolyFill", "polyfill"),
      T("PolyExtrude", "polyextrude", { sel: { face: "group" }, params: { distance: 0.3 } }),
      T("PolySplit", "polysplit", { sel: { edge: "split_edges" }, tip: "Insert an edge loop through the quads around the selected edge" }),
      T("PolyBridge", "polybridge"), T("Clip", "clip"), T("Boolean", "boolean"), P("»", "polygons2")] },
    polygons2: { title: "Polygons", items: [
      T("Divide", "divide", { sel: { face: "group" } }), T("Reverse", "reverse", { sel: { face: "group" } }),
      T("PolyReduce", "polyreduce"), T("PolyExpand2D", "polyexpand2d"), T("Subdivide", "subdivide"),
      T("Clean", "clean"), T("Skin", "skin"), P("Solid tools", "polygons3")] },
    polygons3: {title:"Solid tools",items:[T("Push / Pull","pushpull",{sel:{face:"group"}}),T("Inset","inset",{sel:{face:"group"}}),T("Outset","outset",{sel:{face:"group"}}),T("Convex Hull","convex_hull"),null,null,null,P("Blender tools","blender")]},

    blender: { title: "Blender modelling", items: [
      T("Solidify", "solidify", { sel: { face: "group" } }),
      T("Merge Vertices", "merge_vertices", { sel: { point: "group", edge: POINTS("group"), face: POINTS("group") } }),
      T("Split Edges", "split_edges", { sel: { edge: "group" } }),
      T("Connect Vertices", "connect_vertices", { sel: { point: "group" } }),
      T("Make Face", "make_faces", { sel: { point: "group", edge: POINTS("group") } }),
      T("Grid Fill", "grid_fill", { sel: { edge: "group" } }),
      T("Triangles to Quads", "triangles_to_quads", { sel: { face: "group" } }), P("More", "blender2")] },
    blender2: { title: "Native topology", items: [
      T("Native Bevel", "native_bevel", { sel: { edge: "group" } }),
      T("Native Extrude", "native_extrude", { sel: { face: "group" } }),
      T("Native Inset", "native_inset", { sel: { face: "group" } }),
      T("Bisect", "native_bisect", { sel: { face: "group" } }),
      T("Symmetrize", "symmetrize", { sel: { point: "group", face: POINTS("group") } }),
      T("Spin", "spin", { sel: { point: "group", edge: POINTS("group") } }),
      T("Wireframe", "native_wireframe", { sel: { face: "group" } }), P("More", "blender3")] },
    blender3: { title: "Mesh finishing", items: [
      T("Poke Faces", "poke_faces", { sel: { face: "group" } }),
      T("Merge by Distance", "merge_by_distance", { sel: { point: "group", face: POINTS("group") } }),
      T("Split Geometry", "split_geometry", { sel: { face: "group" } }),
      T("Limited Dissolve", "limited_dissolve", { sel: { edge: "group" } }),
      T("To Sphere", "to_sphere", { sel: { point: "group", edge: POINTS("group"), face: POINTS("group") } }),
      T("Shear", "shear", { sel: { point: "group", edge: POINTS("group"), face: POINTS("group") } }),
      T("Advanced Native Mesh", "blender_mesh"), null] },

    group: { title: "Group", items: [
      T("Group Promote", "group_promote"), T("Group by Range", "group_range"), T("Group Paint", null, { disabled: "Painting groups is not available yet" }),
      T("Group Create", "group_create", { sel: { face: { pattern: "$", domain: "primitives", method: "pattern", name: "faces" },
        point: { pattern: "$", domain: "points", method: "pattern", name: "points" },
        edge: { pattern: "$", domain: "edges", method: "pattern", name: "edges" } } }),
      T("Group Expression", "group_create", { params: { name: "group1", domain: "points", method: "pattern", pattern: "@P.y > 0" },
        tip: "A group made from an expression, e.g. @P.y > 0" }),
      T("Group Expand", "group_expand"), T("Edge Group", "edge_group"), P("»", "group2")] },
    group2: { title: "Group", items: [
      T("Edge Group Promote", "edge_group_promote"), T("Blast", "blast", { sel: { face: { group: "$", domain: "primitives" },
        point: { group: "$", domain: "points" } } }), T("Crease", "crease", { sel: { edge: "group" } }),
      null, null, null, null, null] },

    create: { title: "Create", items: [
      T("Null", "null", { tip: "A named pass-through" }), T("Circle", "circle", { source: true }), T("Tube", "tube", { source: true }),
      T("Sphere", "sphere", { source: true }), T("Curve", "curve", { source: true }), T("Grid", "grid", { source: true }),
      T("Box", "box", { source: true }), P("»", "create2")] },
    create2: { title: "Create", items: [
      T("Torus", "torus", { source: true }), T("Line", "line", { source: true }), T("Helix", "helix", { source: true }),
      T("L-System", "lsystem", { source: true }), T("Lattice Grid", "lattice_grid", { source: true }),
      T("Bound", "bound"), T("Revolve", "revolve"), T("Scatter", "scatter")] },

    build: { title: "Build", items: [
      T("Mirror", "mirror"), T("Duplicate", "copy_transform", { tip: "Copy and Transform: stairs, arrays, spirals" }),
      T("Combine", "merge", { tip: "Merge geometry" }),
      T("Extract", "blast", { params: { delete_non_selected: true }, sel: { face: { group: "$", domain: "primitives" },
        point: { group: "$", domain: "points" } }, tip: "Keep only the selection" }),
      null, null, P("Fracture", "fracture"), P("»", "build2")] },
    build2: { title: "Build", items: [
      T("Copy to Points", "copy_to_points"), T("Instance", "instance"), T("Sweep", "sweep"), T("Skin", "skin"),
      T("Revolve", "revolve"), T("Switch", "switch"), T("Polypath", "polypath"), null] },

    // Houdini's RBD fracture menu: Assemble, Boolean Fracture, Convex Decomposition, Glue Cluster, RBD Material Fracture and the four Voronoi tools
    fracture: { title: "Fracture", items: [
      T("Voronoi Fracture", "voronoi_fracture", { tip: "Wire the fracture points into the second input" }),
      T("Voronoi Fracture Points", "voronoi_fracture_points"), T("Voronoi Split", "voronoi_split"),
      T("Voronoi Adjacency", "voronoi_adjacency"), T("Boolean Fracture", "boolean_fracture", { tip: "Wire the cutters into the second input" }),
      T("Convex Decomposition", "convex_decomposition"), T("Glue Cluster", "glue_cluster"), P("»", "fracture2")] },
    fracture2: { title: "Fracture", items: [
      T("RBD Material Fracture", "rbd_material_fracture"), T("Assemble", "assemble"),
      T("RBD Configure", "rbd_configure"), T("RBD Solver", "rbd_solver", { tip: "Bodies in the first input, then colliders and constraint links" }),
      T("RBD Constraint Properties", "rbd_constraint_properties", { tip: "Set or scale constraint breaking strength" }),
      T("RBD Constraint", "rbd_constraint", { source: true, tip: "Create a glue, pin, hinge or spring link between body IDs" }), null, null] },

    edges: { title: "Edges", items: [
      T("Dissolve", "dissolve_edges", { sel: { edge: "group" } }), T("Edge Collapse", "edge_collapse", { sel: { edge: "group" } }),
      T("Edge Loop", "polysplit", { sel: { edge: "split_edges" }, tip: "Insert an edge loop (PolySplit)" }),
      T("Edge Cusp", "edge_cusp", { sel: { edge: "group" } }), T("Edge Flip", "edge_flip", { sel: { edge: "group" } }),
      null, null, P("»", "edges2")] },
    edges2: { title: "Edges", items: [
      T("Crease", "crease", { sel: { edge: "group" } }), T("Edge Group", "edge_group"), T("Polybridge", "polybridge"),
      T("Fillet", "fillet"), T("Resample", "resample"), null, null, null] },

    // the Modify page was not among the shared screenshots: the usual suspects
    modify: { title: "Modify", items: [
      T("Blast", "blast", { sel: { face: { group: "$", domain: "primitives" }, point: { group: "$", domain: "points" } } }),
      T("Fuse", "fuse"), T("Normal", "normal"), T("Reverse", "reverse", { sel: { face: "group" } }),
      T("Divide", "divide", { sel: { face: "group" } }), T("Subdivide", "subdivide"), T("Resample", "resample"), P("»", "modify2")] },
    modify2: { title: "Modify", items: [
      T("Attribute Wrangle", "wrangle"), T("Attribute Create", "attribute_create"), T("Attribute Randomize", "attribute_randomize"),
      T("Attribute Transfer", "attribute_transfer"), T("Sort", "sort"), T("UV Project", "uv_project"),
      T("UV Flatten", "uv_flatten"), T("Measure", "measure")] },
  };

  const NAMES = ["N", "NE", "E", "SE", "S", "SW", "W", "NW"];
  const R0 = 34, R1 = 176, RL = 108;                     // dead zone, outer radius, label radius
  const NS = "http://www.w3.org/2000/svg";
  let root = null, st = null, hooks = { onPick() {}, onOpen() {}, types: () => ({}), blocked: () => false };
  let mouse = [innerWidth / 2, innerHeight / 2];

  const el = (tag, attrs, kids) => {
    const e = document.createElementNS(NS, tag);
    for (const [k, v] of Object.entries(attrs || {})) e.setAttribute(k, v);
    for (const c of kids || []) e.append(c);
    return e;
  };
  const pol = (r, deg) => [r * Math.sin((deg * Math.PI) / 180), -r * Math.cos((deg * Math.PI) / 180)];
  function wedge(i) {
    const a0 = i * 45 - 22.5 + 1, a1 = i * 45 + 22.5 - 1;
    const [x0, y0] = pol(R1, a0), [x1, y1] = pol(R1, a1), [x2, y2] = pol(R0, a1), [x3, y3] = pol(R0, a0);
    return `M${x0},${y0} A${R1},${R1} 0 0 1 ${x1},${y1} L${x2},${y2} A${R0},${R0} 0 0 0 ${x3},${y3} Z`;
  }
  const usable = (it) => !!it && (it.page || (it.type && !it.disabled && hooks.types()[it.type]));
  const why = (it) => (!it ? "" : it.disabled || (it.type && !hooks.types()[it.type] ? `No ${it.type} node in this engine` : it.tip || ""));

  function ensureRoot() {
    if (root) return;
    root = document.createElement("div");
    root.id = "pie";
    root.className = "hidden";
    root.addEventListener("pointermove", (e) => { if (st) hover(e.clientX, e.clientY); });
    root.addEventListener("pointerdown", (e) => { e.preventDefault(); e.stopPropagation(); });
    root.addEventListener("pointerup", (e) => { if (!st) return; e.preventDefault(); e.stopPropagation(); choose(); });
    root.addEventListener("contextmenu", (e) => { e.preventDefault(); close(); });
    document.body.append(root);
  }

  function render() {
    const page = PAGES[st.page];
    const size = R1 * 2 + 4;
    const svg = el("svg", { width: size, height: size, viewBox: `${-size / 2} ${-size / 2} ${size} ${size}` });
    svg.append(el("circle", { r: R1 + 2, class: "pie-bg" }));
    st.nodes = [];
    for (let i = 0; i < 8; i++) {
      const it = page.items[i];
      if (!it) { svg.append(el("path", { d: wedge(i), class: "pie-wedge empty" })); st.nodes.push(null); continue; }
      const ok = usable(it);
      const w = el("path", { d: wedge(i), class: "pie-wedge" + (ok ? "" : " off") + (it.page ? " sub" : "") });
      const [lx, ly] = pol(RL, i * 45);
      const label = el("text", { x: lx, y: ly, class: "pie-label" + (ok ? "" : " off"), "text-anchor": "middle", "dominant-baseline": "middle" });
      label.textContent = it.label;
      const g = el("g", { "data-slot": i, "data-label": it.label }, [w, label]);
      const tip = why(it);
      if (tip) g.append(el("title", {}, [document.createTextNode(tip)]));
      svg.append(g);
      st.nodes.push(g);
    }
    const centre = el("g", { class: "pie-centre", "data-slot": "centre" }, [el("circle", { r: R0 - 3 })]);
    const c = el("text", { class: "pie-title", "text-anchor": "middle", "dominant-baseline": "middle" });
    c.textContent = st.page === "main" ? "C" : "◂";
    centre.append(c);
    svg.append(centre);
    st.centre = centre;
    root.replaceChildren(svg);
    const info = document.createElement("div");
    info.className = "pie-info";
    info.textContent = page.title + (st.page === "main" ? "" : " — release in the middle to go back");
    root.append(info);
    st.info = info;
    svg.style.left = st.x - size / 2 + "px";
    svg.style.top = st.y - size / 2 + "px";
    info.style.left = st.x + "px";
    info.style.top = st.y + R1 + 10 + "px";
    st.hover = null;
    hover(mouse[0], mouse[1]);
  }

  function slotAt(x, y) {
    const dx = x - st.x, dy = y - st.y, d = Math.hypot(dx, dy);
    if (d < R0) return "centre";
    if (d > R1 + 70) return null;
    let a = (Math.atan2(dx, -dy) * 180) / Math.PI;
    if (a < 0) a += 360;
    return Math.round(a / 45) % 8;
  }
  function hover(x, y) {
    const s = slotAt(x, y);
    st.hover = s;
    st.nodes.forEach((g, i) => g && g.classList.toggle("hot", i === s));
    st.centre.classList.toggle("hot", s === "centre");
    const it = typeof s === "number" ? PAGES[st.page].items[s] : null;
    st.info.textContent = it ? (it.page ? `${it.label}` : it.label) + (why(it) ? " — " + why(it) : "") : PAGES[st.page].title;
  }

  function choose() {
    if (!st) return;
    const s = st.hover;
    if (s === "centre") { if (st.page === "main") close(); else { st.page = "main"; render(); } return; }
    if (typeof s !== "number") { close(); return; }
    const it = PAGES[st.page].items[s];
    if (!it) { close(); return; }
    if (it.page) { st.page = it.page; render(); return; }
    if (!usable(it)) { hooks.onPick(null, why(it)); close(); return; }
    close();
    hooks.onPick(it);
  }

  function open(x, y, page) {
    ensureRoot();
    const size = R1 + 20;
    x = Math.max(size, Math.min(x, innerWidth - size));
    y = Math.max(size, Math.min(y, innerHeight - size - 20));
    hooks.onOpen();
    st = { x, y, page: page || "main", t0: performance.now(), held: true, hover: null };
    root.classList.remove("hidden");
    render();
  }
  function close() { if (st && root) root.classList.add("hidden"); st = null; }
  const isOpen = () => !!st;

  function init(h) {
    hooks = Object.assign(hooks, h);
    document.addEventListener("pointermove", (e) => { mouse = [e.clientX, e.clientY]; }, true);
    document.addEventListener("keydown", (e) => {
      const tag = (e.target.tagName || "").toLowerCase();
      if (st) {
        if (e.key === "Escape") close();
        else if (e.key.toLowerCase() === "c" && e.repeat) { /* held */ }
        else if (/^[1-8]$/.test(e.key)) { st.hover = Number(e.key) - 1; choose(); }
        else return;
        e.preventDefault(); e.stopImmediatePropagation();
        return;
      }
      if (e.key.toLowerCase() !== "c" || e.ctrlKey || e.metaKey || e.altKey || e.shiftKey || e.repeat) return;
      if (tag === "input" || tag === "textarea" || tag === "select" || e.target.isContentEditable || hooks.blocked()) return;
      open(mouse[0], mouse[1]);
      e.preventDefault(); e.stopImmediatePropagation();
    }, true);
    document.addEventListener("keyup", (e) => {
      if (!st || e.key.toLowerCase() !== "c") return;
      if (performance.now() - st.t0 > 260 && st.hover !== null && st.hover !== "centre") choose();   // hold, flick, release
      e.preventDefault(); e.stopImmediatePropagation();
    }, true);
    window.addEventListener("blur", close);
  }

  /* The pie's definition of a tool (its selection mapping and presets) by node type, preferring the one with this label. */
  function find(type, label) {
    let any = null;
    for (const page of Object.values(PAGES)) for (const it of page.items) {
      if (!it || it.page || it.type !== type) continue;
      if (it.label === label) return it;
      if (!any) any = it;
    }
    return any;
  }

  window.MiniPie = { init, open, close, isOpen, PAGES, find, state: () => st && { page: st.page, hover: st.hover, x: st.x, y: st.y } };
})();
