"""Write the parameter metadata (kind, range, options, doc, default) of the node types the Rust core has, from the Python node
registry, to ``rust/crates/minihou-app/src/param_meta.json``. The app reads it to draw sliders and drop-downs. The registry is
the source of truth; ``tests/test_rust_param_meta.py`` fails when the file is out of date, so run this script after changing a
parameter of one of these nodes::

    py tools/dump_rust_param_meta.py
"""
import json
import os
import sys
import tomllib
from pathlib import Path

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, ROOT)

TYPES = ["grid", "box", "curve", "circle", "sphere", "tube", "torus", "delete", "transform", "normal", "noise", "match_size", "merge", "null", "output", "blender_object", "modifier_input", "blender_image", "image_file", "image_out", "scene_objects"]
TARGET = os.path.join(ROOT, "rust", "crates", "minihou-app", "src", "param_meta.json")
PACKAGES = Path(ROOT) / "rust" / "crates" / "minihou-core" / "src" / "nodes" / "packages"


def package_meta() -> dict:
    """Discover the same package directories and IDs as minihou-core/build.rs."""
    result = {}
    if not PACKAGES.exists():
        return result
    for package in sorted(path for path in PACKAGES.iterdir() if path.is_dir()):
        with (package / "node.toml").open("rb") as f:
            node_id = tomllib.load(f)["id"]
        if not (package / "node.rs").is_file():
            raise ValueError(f"{package}: missing node.rs")
        with (package / "param_meta.json").open(encoding="utf-8") as f:
            params = json.load(f)
        for name, metadata in params.items():
            if not metadata.get("doc"):
                raise ValueError(f"{node_id}.{name}: missing parameter doc")
        if node_id in result:
            raise ValueError(f"duplicate package node id: {node_id}")
        result[node_id] = params
    return result

# Repeat regions and scalar expression/deformation ports (metadata generated separately).
TYPES += ["loop_begin", "loop_end", "input", "object_input", "wrangle", "blend", "exploded_view", "usd_export", "usd_import"]

# Procedural / instancing ports; retain the other workers' type lists above.
TYPES += ["bezier", "helix", "lsystem", "convert_line", "polypath", "resample", "carve",
          "copy_to_points", "instance", "for_each", "group_promote", "attribute_delete",
          "attribute_rename", "attribute_create", "attribute_promote", "attribute_blur",
          "connectivity", "smooth", "falloff", "bend", "uv_project", "uv_flatten", "uv_transform",
          "fillet", "curve_simplify", "group_create", "group_expand", "blast", "measure"]


# Modeling ports. loop_cut is the native keymap alias for Python's polysplit.
TYPES += ["divide", "mirror", "clip", "polyfill", "polyextrude",
          "inset", "outset", "solidify", "crease", "edge_group", "edge_group_promote",
          "polybridge", "dissolve_edges", "subdivide", "twist", "linear_taper",
          "squash_stretch", "soft_peak", "group_range", "edge_collapse", "edge_flip",
          "edge_cusp", "polysplit", "skin_profiles", "polyexpand2d", "polybevel", "knife"]


# Solid/volume ports; regenerate metadata once all parallel ports are integrated.
TYPES += ["boolean", "polyreduce", "pushpull", "boolean_fracture"]
TYPES += ["sdf_shape", "sdf_boolean", "voronoi_fracture", "voronoi_split",
          "voronoi_adjacency", "voronoi_fracture_points", "ray", "occlusion"]

# PCG, procedural systems, and assemble ports.
TYPES += ["pcg_points", "pcg_transform", "pcg_bounds", "pcg_union", "pcg_difference",
          "pcg_intersection", "pcg_density_filter", "pcg_density_noise", "pcg_distance",
          "pcg_copy_points", "pcg_self_prune", "pcg_projection", "pcg_spline_sampler",
          "pcg_spawner", "pcg_volume_sampler", "pcg_surface_sampler", "pcg_debug",
          "building_rules", "region_identify", "region_transform", "assemble"]

# Surface, curve, deform, and terrain ports.
TYPES += ["delaunay", "bridge", "revolve", "sweep", "skin", "lattice",
          "lattice_grid", "path_deform", "point_deform", "points_to_surface",
          "contour", "remesh", "heightfield_erode"]

# Ported nodes: scatter, deform, primitives, transform, attributes, materials, surface, contour.
TYPES += ["scatter", "point_jitter", "point_relax", "sort", "switch", "clean",
          "fuse", "soft_transform", "copy_transform", "attribute_randomize",
          "attribute_transfer", "convex_hull", "slice", "dissolve", "color", "attrib_paint"]


# Time / physics ports (docs/RUST_SOLVER_DESIGN.md).
TYPES += ["goal_length", "goal_collision", "goal_surface", "goal_gravity", "goal_pin", "goal_solve",
          "solver", "rbd_configure", "rbd_constraint", "rbd_solver", "rbd_constraint_properties", "glue_cluster"]

# Last node ports.
TYPES += ["rbd_material_fracture", "convex_decomposition", "python"]


def meta() -> dict:
    import minihou  # noqa: F401 - registers the node types
    from minihou.sdk import REGISTRY

    out = {}
    for t in TYPES:
        params = {}
        for p in REGISTRY.get(t).params:
            d = {"kind": p.kind}
            for a in ("min", "max", "soft_min", "soft_max", "step", "doc"):
                v = getattr(p, a, None)
                if v is not None:
                    d[a] = v
            d["default"] = p.to_json_value(p.default)
            opts = getattr(p, "options", None) or getattr(p, "choices", None)
            if opts:
                d["options"] = [o if isinstance(o, str) else getattr(o, "name", str(o)) for o in opts]
            params[p.name] = d
        out[t] = params
    out["polysplit"].update({
        "factor": {"kind":"float","default":0.0,"min":-1.0,"max":1.0,"doc":"Signed slide of the new cuts along their rails."},
        "smoothness": {"kind":"float","default":0.0,"min":-1.0,"max":1.0,"doc":"Loop Cut smoothing amount (reserved for future surface interpolation)."},
        "falloff": {"kind":"enum","default":"inverse_square","options":["inverse_square","linear","smooth"],"doc":"Smoothness falloff (reserved)."},
        "even": {"kind":"bool","default":False,"doc":"Keep a constant travel distance along uneven rails."},
        "flipped": {"kind":"bool","default":False,"doc":"Swap the side selected by the Factor sign."},
        "clamp": {"kind":"bool","default":True,"doc":"Stop at the neighbouring edge."},
        "correct_uvs": {"kind":"bool","default":True,"doc":"Slide corner UVs independently on each face."},
    })
    # Select arrow (viewport pick) element domain for fields that take edges.
    out["subdivide"]["creases"]["pick_domain"] = "edges"
    # Select arrow domains for group fields, declared once here so every group field follows one rule.
    for node, field, domain in [("polyextrude","group","faces"), ("transform","group","points"),
                                ("polybevel","group","edges")]:
        if field in out.get(node, {}):
            out[node][field]["pick_domain"] = domain
    # polybevel group follows its group_type parameter (edges or primitives) when set.
    out["polybevel"]["group"]["pick_domain_param"] = "group_type"
    out["polybevel"]["group"]["pick_domain_default"] = "edges"
    out["attribute_create"]["group"]["pick_domain_param"] = "domain"
    out["attribute_create"]["group"]["pick_domain_default"] = "points"
    # Remaining group fields: fixed element domain, or follow the node's own domain parameter.
    _pts = "bend lattice linear_taper mirror noise peak point_deform point_jitter pushpull smooth soft_peak soft_transform squash_stretch twist attribute_blur attribute_transfer attribute_randomize resample wrangle instance convex_hull blend goal_pin goal_surface occlusion measure_bounds measure_distance measure_length skin_profiles scatteralign voronoi_fracture rbd_constraint_properties".split()
    _faces = "dissolve divide inset outset tris_to_quads slice reverse recalc_normals solidify uv_project uv_transform unpack pack polyfill polyexpand2d measure_area measure_volume contour assemble".split()
    _edges = "dissolve_edges edge_collapse edge_cusp edge_flip edge_slide grid_fill fillet".split()
    for _nodes, _dom in ((_pts, "points"), (_faces, "faces"), (_edges, "edges")):
        for _n in _nodes:
            if "group" in out.get(_n, {}) and "pick_domain" not in out[_n]["group"]:
                out[_n]["group"]["pick_domain"] = _dom
    for _n, _f in [("blast","group"), ("group_create","pattern"), ("color","group"), ("crease","group"),
                   ("falloff","group"), ("group_expand","group"), ("grow","group"), ("set_material","group"),
                   ("select_loop","group_name"), ("select_path","group_name"), ("select_ring","group_name")]:
        if _f in out.get(_n, {}):
            out[_n][_f]["pick_domain_param"] = "domain"
            out[_n][_f]["pick_domain_default"] = "primitives" if _n in ("blast","set_material","group_expand","falloff") else ("edges" if _n in ("crease","select_loop","select_path","select_ring","grow") else "points")
    if "points" in out.get("polyextrude", {}):
        out["polyextrude"]["points"]["pick_domain"] = "points"
    if "edges" in out.get("polyextrude", {}):
        out["polyextrude"]["edges"]["pick_domain"] = "edges"
    for _f in ("edges", "exclude_edges"):
        if _f in out.get("polybevel", {}):
            out["polybevel"][_f]["pick_domain"] = "edges"
    for _n in ("polysplit", "loop_cut"):
        if "split_edges" in out.get(_n, {}):
            out[_n]["split_edges"]["pick_domain"] = "edges"
    out["loop_cut"] = out["polysplit"]
    out["edge_slide"] = {
        "group": {"kind":"str","default":"","doc":"Selected edge group or pA-B pairs; empty leaves the mesh unchanged."},
        "factor": {"kind":"float","default":0.0,"min":-1.0,"max":1.0,"doc":"Signed distance to slide the selected edges along their adjacent rails."},
        "even": {"kind":"bool","default":False,"doc":"Use equal travel distance along rails of different lengths."},
        "flipped": {"kind":"bool","default":False,"doc":"Reverse the side chosen by the Factor sign."},
        "clamp": {"kind":"bool","default":True,"doc":"Stop sliding at the neighbouring edge."},
        "mirror_editing": {"kind":"bool","default":True,"doc":"Mirror setting; active when a symmetry context is available."},
        "correct_uvs": {"kind":"bool","default":True,"doc":"Adjust corner UVs on each face while sliding."},
        "output_group": {"kind":"bool","default":False,"doc":"Write a group containing the slid edges."},
        "output_group_name": {"kind":"str","default":"slid_edges","doc":"Name of the generated slid edge group."},
    }
    # modifier_input owns an ordered interface schema rather than fixed node parameters.
    # Keep this description with the generated native parameter metadata for the editor.
    out["edge_slide"]["group"]["pick_domain"] = "edges"
    out["modifier_input"]["__dynamic_inputs__"] = {
        "kind": "interface", "doc": "Ordered Blender inputs with stable ids, names, defaults and optional ranges/subtypes.",
        "types": ["float", "int", "bool", "vector", "color", "rotation", "matrix", "string", "menu",
                  "object", "collection", "material", "image", "texture", "geometry", "bundle", "closure"],
    }
    out["polyextrude"].update({
        "edges": {"kind": "str", "default": "", "doc": "Optional edge selection for edge extrusion."},
        "points": {"kind": "str", "default": "", "doc": "Optional point selection for point extrusion."},
        "direction": {"kind": "vec3", "default": [0.0, 0.0, 0.0], "doc": "Explicit extrusion direction; zero follows face normals."},
    })
    # Rust Push/Pull extension (SketchUp-style punch through / coplanar merge); "off" is the Python behaviour.
    out["pushpull"]["through"] = {"kind": "enum", "default": "auto", "options": ["off", "auto", "force"],
                                  "doc": "Punch through: auto opens a hole when the push reaches or passes the opposite surface (excess ignored) and merges a pull that lands on a parallel face; force always punches; off is the original single-convex-face behaviour."}
    # Rust Mirror SOP extensions (Houdini parity); the defaults reproduce the Python node.
    out["mirror"]["distance"]["doc"] = "Consolidate tolerance: how close to the plane a point must be to weld (Houdini's seam tolerance)."
    out["mirror"].update({
        "group": {"kind": "str", "default": "", "doc": "Primitives to mirror when Operation is Mirror Selected."},
        "operation": {"kind": "enum", "default": "mirror_all", "options": ["mirror_all", "mirror_selected"], "doc": "Mirror every primitive, or only the Group."},
        "offset": {"kind": "float", "default": 0.0, "soft_min": -10.0, "soft_max": 10.0, "doc": "Houdini's Distance: moves the plane along the direction."},
        "reverse": {"kind": "bool", "default": False, "doc": "Flip the plane direction: keep (with Cut) and mirror the other side."},
        "cut": {"kind": "bool", "default": False, "doc": "Bisect first: remove geometry behind the plane so only the kept side is mirrored (Blender-style)."},
        "unshared_only": {"kind": "bool", "default": False, "enabled_if": {"consolidate_seam": True}, "doc": "Only consolidate points on unshared (boundary) edges lying in the plane."},
    })
    # Rust PolyFill SOP extensions (Houdini parity); the defaults reproduce the Python node.
    out["polyfill"].update({
        "boundary_group": {"kind": "str", "default": "", "doc": "Boundary edges or curves to fill (empty: fill all holes)."},
        "group": {"kind": "str", "default": "", "doc": "Alias for boundary_group."},
        "mode": {"kind": "enum", "default": "quadrilaterals", "options": ["quadrilaterals", "triangles", "triangle_fan", "single_polygon", "quadrilateral_fan", "quadrilateral_grid"], "doc": "Hole filling pattern (quadrilaterals, triangles, triangle_fan, single_polygon, quadrilateral_fan, quadrilateral_grid)."},
        "fill_mode": {"kind": "enum", "default": "quadrilaterals", "options": ["quadrilaterals", "triangles", "triangle_fan", "single_polygon", "quadrilateral_fan", "quadrilateral_grid"], "doc": "Alias for mode."},
        "auto_complete_boundaries": {"kind": "bool", "default": True, "doc": "Walk remaining boundary edges to close open boundary selections."},
        "autocomplete": {"kind": "bool", "default": True, "doc": "Alias for auto_complete_boundaries."},
        "reverse_patches": {"kind": "bool", "default": False, "doc": "Flip the normal orientation of newly generated patch faces."},
        "reverse": {"kind": "bool", "default": False, "doc": "Alias for reverse_patches."},
        "unique_points": {"kind": "bool", "default": False, "doc": "Duplicate boundary points so patch geometry is disconnected from the input mesh."},
        "uniquepoints": {"kind": "bool", "default": False, "doc": "Alias for unique_points."},
        "update_point_normals": {"kind": "bool", "default": True, "doc": "Recompute point normals along the boundary and new patch vertices."},
        "updatenormals": {"kind": "bool", "default": True, "doc": "Alias for update_point_normals."},
        "edge_loop": {"kind": "float", "default": 0.25, "min": 0.0, "max": 1.0, "doc": "Fractional width of transitional quad strip along open boundaries."},
        "edgeloop": {"kind": "float", "default": 0.25, "min": 0.0, "max": 1.0, "doc": "Alias for edge_loop."},
        "smooth": {"kind": "float", "default": 50.0, "min": 0.0, "soft_max": 100.0, "doc": "Laplacian smoothing iterations applied to interior patch points."},
        "corner_offset": {"kind": "int", "default": 0, "doc": "Shift the starting corner index around the boundary loop."},
        "custom_corners": {"kind": "str", "default": "", "doc": "Point group specifying custom corner locations for patch layout."},
        "deform_patch": {"kind": "bool", "default": True, "doc": "Project and deform patch interior according to boundary curvature and surface offset."},
        "deform": {"kind": "bool", "default": True, "doc": "Alias for deform_patch."},
        "surface_offset": {"kind": "float", "default": 0.0, "doc": "Offset distance along normal for patch interior vertices."},
        "tangent_strength": {"kind": "float", "default": 0.4, "min": 0.0, "soft_max": 2.0, "doc": "Influence of boundary tangent vectors on interior patch bulge."},
        "patch_group": {"kind": "str", "default": "", "doc": "Primitive group name for newly created patch faces."},
        "patchgroup": {"kind": "str", "default": "", "doc": "Alias for patch_group."},
        "edge_loop_group": {"kind": "str", "default": "loopgroup", "doc": "Primitive group name for transitional edge loop faces."},
        "edgeloopgroup": {"kind": "str", "default": "loopgroup", "doc": "Alias for edge_loop_group."},
        "append_to_groups": {"kind": "bool", "default": False, "doc": "Append new faces to existing groups rather than replacing them."},
        "append_groups": {"kind": "bool", "default": False, "doc": "Alias for append_to_groups."},
    })
    # Rust-only nodes (no Python counterpart to introspect)
    out["lod_bake"] = {
        "cluster_size": {"default": 128, "doc": "Maximum triangles per cluster (16 to 128)", "kind": "int", "min": 16, "max": 128},
        "error_px": {"default": 1.0, "doc": "Screen-space error budget in pixels", "kind": "float", "min": 0.01, "soft_max": 16.0},
        "enabled": {"default": True, "doc": "Build and use the clustered LOD payload", "kind": "bool"},
        "min_triangles": {"default": 50000, "doc": "Skip the build for smaller meshes", "kind": "int", "min": 0, "soft_max": 1000000},
    }
    # Selection nodes (docs/RUST_SELECT_NODES.md): Blender's Select More/Less, Shortest Path, Loop and Ring.
    _domain = {"default": "primitives", "kind": "enum", "options": ["points", "edges", "primitives"],
               "doc": "Element type of the selection (Blender's vertex / edge / face select mode); edge groups are edge_group_ attributes"}
    _combine = {"default": "replace", "kind": "enum", "options": ["replace", "union"],
                "doc": "replace: write only this selection; union: add it to an existing group of the same name"}
    _direction = {"default": "both", "kind": "enum", "options": ["both", "forward", "backward"],
                  "doc": "both: walk from both ends of each seed; forward / backward: start only towards the seed's higher / lower point (rings: second / first face)"}
    _seeds = {"default": "", "kind": "str",
              "doc": "Seed edges: edge ids, p<a>-<b> point pairs, edge group names or wildcards (empty: nothing selected)"}
    out["grow"] = {
        "group": {"default": "", "kind": "str", "doc": "Selection to grow or shrink: group name or pattern in the chosen domain (empty: everything)"},
        "domain": dict(_domain),
        "mode": {"default": "grow", "kind": "enum", "options": ["grow", "shrink"],
                 "doc": "grow: Select More (add neighbours); shrink: Select Less (keep elements whose neighbours are all selected)"},
        "steps": {"default": 1, "kind": "int", "min": 0, "max": 100000, "soft_max": 10, "doc": "How many grow / shrink steps (0: unchanged)"},
        "face_step": {"default": True, "kind": "bool",
                      "doc": "Step across polygons through shared points (Blender's Face Step); off: neighbours share an edge (faces) or a point (edges)"},
        "keep_selection": {"default": True, "kind": "bool", "doc": "Write the whole resulting selection to Result Group"},
        "result_group_name": {"default": "", "kind": "str", "enabled_if": {"keep_selection": True},
                              "doc": "Group for the resulting selection (empty: the input group's name, or 'selection' for patterns)"},
        "output_group": {"default": False, "kind": "bool", "doc": "Also write only the change: the elements added by growing or removed by shrinking"},
        "output_group_name": {"default": "grown", "kind": "str", "enabled_if": {"output_group": True}, "doc": "Group for the added / removed elements"},
    }
    out["select_path"] = {
        "domain": dict(_domain, default="points"),
        "start": {"default": 0, "kind": "int", "min": 0, "soft_max": 1000, "doc": "First element number (used when Waypoints is empty)"},
        "end": {"default": 1, "kind": "int", "min": 0, "soft_max": 1000, "doc": "Last element number (used when Waypoints is empty)"},
        "waypoints": {"default": "", "kind": "str",
                      "doc": "Ordered elements to route through: ids (edges also p<a>-<b>) keep their order; group names add members in element order"},
        "method": {"default": "shortest_distance", "kind": "enum", "options": ["fewest_steps", "shortest_distance"],
                   "doc": "fewest_steps: fewest connections (topological); shortest_distance: geometrically shortest by edge length"},
        "closed": {"default": False, "kind": "bool", "doc": "Route from the last waypoint back to the first"},
        "group_name": {"default": "path", "kind": "str", "doc": "Group that receives the path"},
        "combine": dict(_combine),
    }
    out["select_loop"] = {
        "seeds": dict(_seeds),
        "domain": dict(_domain, default="edges", doc="Output elements: the loop's edges, their points, or the face loop's primitives"),
        "mode": {"default": "edge_loop", "kind": "enum", "options": ["edge_loop", "boundary_loop"],
                 "doc": "edge_loop: through valence-4 points, stopping at poles and borders; boundary_loop: follow the open border of a boundary seed"},
        "direction": dict(_direction),
        "group_name": {"default": "loop", "kind": "str", "doc": "Group that receives the loop"},
        "combine": dict(_combine),
    }
    out["select_ring"] = {
        "seeds": dict(_seeds),
        "domain": dict(_domain, default="edges", doc="Output elements: the ring's edges, their points, or the quads between them"),
        "direction": dict(_direction),
        "group_name": {"default": "ring", "kind": "str", "doc": "Group that receives the ring"},
        "combine": dict(_combine),
    }
    out["convex_hull"] = {
        "group": {"default": "", "doc": "Points or primitives to compute the hull from (empty: all)", "kind": "str"},
        "type": {"default": "3d", "doc": "3D convex hull mesh or 2D convex hull polygon", "kind": "enum", "options": ["3d", "2d"]},
        "positions_2d": {"default": "fit_plane", "doc": "Projection plane for 2D hull (best-fit plane or world axis plane)", "kind": "enum", "options": ["fit_plane", "xy", "yz", "zx"]},
        "shrink_amount": {"default": 0.0, "doc": "Pull hull surface toward interior points (0: pure convex hull, 1: full shrinkwrap)", "kind": "float", "min": 0.0, "max": 1.0, "soft_max": 1.0},
        "preserve_attributes": {"default": True, "doc": "Copy point attributes and groups from the source points to the hull", "kind": "bool"},
        "remove_inline_points": {"default": True, "doc": "Merge coplanar triangles into polygon faces and drop inline edge/interior points", "kind": "bool"},
    }
    # Sketch-tool extension of the Curve SOP: a drawn open line also splits coplanar faces of the input geometry.
    out["blender_object"]["reset_transform"] = {"kind": "bool", "default": False, "doc": "Ignore the object's location: keep its rotation and scale but move it to the origin. Off matches Blender's own placement."}
    out["curve"]["split"] = {"default": False, "kind": "bool", "doc": "With an input: split coplanar faces of the input along the drawn open polyline (SketchUp-style line tool)"}
    out["curve"]["as_mesh"] = {"default": False, "kind": "bool", "doc": "Sketch insertion: create mesh lines or a face instead of a viewport curve"}
    out["bezier"]["as_mesh"] = {"default": False, "kind": "bool", "doc": "Sketch insertion: create mesh lines or a face instead of a viewport curve"}
    # Rust Ray SOP extensions. Keep the Python defaults and legacy methods intact.
    ray = out["ray"]
    ray["method"]["options"] += ["project_rays"]
    ray["method"]["doc"] = "Project Rays uses Direction From; direction and normal retain Python-compatible behavior; minimum_distance finds the closest surface."
    ray["scale"].pop("max", None)
    ray["scale"]["soft_max"] = 1.0
    ray["scale"]["doc"] = "Fraction of the intersection path (0: unchanged, 1: surface; values above 1 overshoot)."
    ray["lift"]["doc"] = "Project Rays: offset along the ray after scaling. Legacy methods: offset along the hit normal."
    ray["max_distance"]["enabled_if"] = {"max_distance_enabled": True}
    ray.update({
        "entity": {"kind": "enum", "default": "points", "options": ["points", "primitives"], "doc": "Interpret Group as points or primitives; selected primitives project their shared points once."},
        "collision_group": {"kind": "str", "default": "", "doc": "Primitive group on the second input (empty: all). Hit primitive numbers remain original input numbers."},
        "direction_from": {"kind": "enum", "default": "normal", "options": ["normal", "specified_vector", "attribute"], "enabled_if": {"method": "project_rays"}, "doc": "Normal computes missing point normals; Specified Vector uses Direction; Attribute reads a point vector."},
        "direction_attribute": {"kind": "str", "default": "N", "enabled_if": {"method": "project_rays", "direction_from": "attribute"}},
        "show_guide_geometry": {"kind": "bool", "default": False, "doc": "Show collision wireframe while this node is selected and displayed. Off by default to preserve legacy output metadata. Very large guides are thinned to 20000 edges."},
        "intersect_farthest_surface": {"kind": "bool", "default": False, "doc": "Choose the farthest surface along each ray, within Max Distance."},
        "point_intersection_normal": {"kind": "bool", "default": False, "doc": "Write hit surface normals to N. Importing target N overrides this. hitN is always available."},
        "point_intersection_distance": {"kind": "bool", "default": False, "doc": "Write intersection distance to dist; hitdist is always available (-1 on misses)."},
        "reverse_rays": {"kind": "bool", "default": False},
        "ray_tolerance": {"kind": "float", "default": 0.01, "min": 0.0, "soft_max": 0.1, "enabled_if": {"method": "project_rays"}, "doc": "World-space edge tolerance for Project Rays; near misses within this distance count as hits."},
        "bias": {"kind": "float", "default": 0.0, "soft_min": -1.0, "soft_max": 1.0, "doc": "Offset transformed points along the hit surface normal."},
        "max_distance_enabled": {"kind": "bool", "default": True, "doc": "Disable for unlimited reach. Enabled by default to preserve Python's 1e6 limit."},
        "sample": {"kind": "int", "default": 1, "min": 1, "max": 65536, "soft_max": 32, "doc": "Ray count per point, including the unperturbed first ray; Minimum Distance uses one closest-point query."},
        "jitter_scale": {"kind": "float", "default": 0.0, "min": 0.0, "soft_max": 1.0, "doc": "Radius of uniform-disk direction perturbations in the ray tangent plane."},
        "ray_combiner": {"kind": "enum", "default": "average", "options": ["average", "median", "shortest", "longest"], "doc": "Combine successful ray lengths along the base direction; average also averages normals and imported float attributes. Prim/UVW use the actual hit nearest the average length."},
        "seed": {"kind": "int", "default": 1, "doc": "Deterministic jitter seed, combined with the source point number."},
        "create_point_group": {"kind": "bool", "default": True, "doc": "Enabled by default for compatibility with rayHit."},
        "ray_hit_group": {"kind": "str", "default": "rayHit", "enabled_if": {"create_point_group": True}},
        "prim_num_attribute_enabled": {"kind": "bool", "default": True, "doc": "Enabled by default for compatibility with hitprim."},
        "prim_num_attribute": {"kind": "str", "default": "hitprim", "enabled_if": {"prim_num_attribute_enabled": True}},
        "prim_uvw_attribute_enabled": {"kind": "bool", "default": False},
        "prim_uvw_attribute": {"kind": "str", "default": "hitprimuv", "enabled_if": {"prim_uvw_attribute_enabled": True}, "doc": "Triangle-local (u,v,0). N-gons use the intersected triangulation triangle, not Houdini's polygon parameterization."},
        "import_attributes_from_hits": {"kind": "bool", "default": False},
        "point_attributes": {"kind": "str", "default": "*", "enabled_if": {"import_attributes_from_hits": True}, "doc": "Target point patterns; floats are barycentrically interpolated. P is excluded. Commas/spaces and ^ exclusions are accepted."},
        "vertex_attributes": {"kind": "str", "default": "", "enabled_if": {"import_attributes_from_hits": True}, "doc": "Target vertex patterns; interpolate at the actual triangle corners, preserving seams."},
        "primitive_attributes": {"kind": "str", "default": "", "enabled_if": {"import_attributes_from_hits": True}, "doc": "Target primitive patterns; copy hit primitive values to source points."},
        "detail_attributes": {"kind": "str", "default": "", "enabled_if": {"import_attributes_from_hits": True}, "doc": "Numeric details broadcast to hit points; structured/string values are copied as detail data when any ray hits."},
    })
    for _name, _doc in {
        "direction_attribute": "Point vector attribute used as the ray direction when Direction From is Attribute.",
        "import_attributes_from_hits": "Copy attributes from the hit primitive of the second input onto the projected points.",
        "prim_num_attribute": "Name of the integer attribute that receives the hit primitive number.",
        "prim_uvw_attribute_enabled": "Write the hit primitive's parametric UVW coordinates to an attribute.",
        "ray_hit_group": "Name of the point group created for points whose ray hit the surface.",
        "reverse_rays": "Cast rays in the opposite direction.",
    }.items():
        out["ray"][_name].setdefault("doc", _doc)
    out["group_create"]["method"]["options"].append("coplanar")
    out["group_create"]["plane_offset"] = {"kind": "float", "default": 0.0, "doc": "Coplanar plane distance along the normalized Direction. Used when Planes is empty."}
    out["group_create"]["direction"]["doc"] = "Reference normal for Normal/Coplanar. Explicit Planes overrides Direction and Plane Offset when populated."
    out["group_create"]["planes"] = {"kind": "string", "default": "", "doc": "Reference plane normals and offsets: nx ny nz offset; one plane per semicolon."}
    out["group_create"]["plane_tolerance"] = {"kind": "float", "default": 0.0001, "min": 0.0, "doc": "Maximum distance from each reference plane in internal length units."}
    # Blender-audit mesh nodes (docs/RUST_MESH_NODES.md): Inset options and three Rust-only nodes.
    out["inset"]["amount"]["doc"] = "Inset thickness (Blender Thickness); negative values outset."
    out["inset"]["mode"]["doc"] = "connected: inset the selected region as one; individual: inset every face on its own (same as Use Individual)."
    out["inset"].update({
        "depth": {"kind": "float", "default": 0.0, "soft_min": -2.0, "soft_max": 2.0, "doc": "Move the inner faces along the face normal (Blender Depth)."},
        "use_boundary": {"kind": "bool", "default": True, "doc": "Also inset the mesh border; off keeps border edges and their points in place (Blender Boundary)."},
        "use_even_offset": {"kind": "bool", "default": True, "doc": "Scale the offset at corners so the ring keeps an even thickness (Blender Offset Even)."},
        "use_relative_offset": {"kind": "bool", "default": False, "doc": "Scale the thickness by the local boundary edge length (Blender Offset Relative)."},
        "use_edge_rail": {"kind": "bool", "default": False, "doc": "Slide boundary points along the existing edges that run into the region (Blender Edge Rail); ignored in individual mode."},
        "use_outset": {"kind": "bool", "default": False, "doc": "Outset instead of inset."},
        "use_individual": {"kind": "bool", "default": False, "doc": "Inset each face separately instead of the region (same as Mode individual)."},
        "use_interpolate": {"kind": "bool", "default": True, "doc": "Copy point attributes to the new inner points; off zeroes them."},
        "output_group": {"kind": "bool", "default": False, "doc": "Write the ring faces to a group."},
        "output_group_name": {"kind": "str", "default": "inset_ring", "enabled_if": {"output_group": True}, "doc": "Name of the ring face group."},
        "output_inner_group": {"kind": "bool", "default": False, "doc": "Write the inner (inset) faces to a group."},
        "output_inner_group_name": {"kind": "str", "default": "inset_inner", "enabled_if": {"output_inner_group": True}, "doc": "Name of the inner face group."},
    })
    # Rust-only fence generator (nodes/fence.rs): posts, rails, slats and X battens along a curve.
    out["fence"] = {
        "path": {"kind": "geometry", "default": "", "doc": "Curve or polyline the fence follows"},
        "bay_width": {"kind": "float", "default": 2.0, "min": 0.1, "soft_max": 6.0, "doc": "Distance between posts along the path"},
        "height": {"kind": "float", "default": 1.8, "min": 0.1, "soft_max": 4.0, "doc": "Overall panel height"},
        "rail_height": {"kind": "float", "default": 0.85, "min": 0.0, "soft_max": 3.0, "doc": "Height of the lower rail above the ground"},
        "slat_width": {"kind": "float", "default": 0.10, "min": 0.01, "soft_max": 0.5, "doc": "Width of each vertical slat"},
        "slat_gap": {"kind": "float", "default": 0.05, "min": 0.0, "soft_max": 0.5, "doc": "Gap between slats"},
        "batten_thickness": {"kind": "float", "default": 0.07, "min": 0.01, "soft_max": 0.3, "doc": "Thickness of the X battens in the top band"},
        "post_size": {"kind": "float", "default": 0.12, "min": 0.01, "soft_max": 0.5, "doc": "Width and depth of each post"},
    }
    out["grid_fill"] = {
        "group": {"kind": "str", "default": "", "doc": "Boundary edges to fill: edge ids, pA-B point pairs or edge group names (empty: every boundary loop)."},
        "span": {"kind": "int", "default": 1, "min": 1, "soft_max": 32, "enabled_if": {"auto_span": False}, "doc": "Number of grid columns, the length of the first side in edges (Blender Span)."},
        "auto_span": {"kind": "bool", "default": True, "doc": "Pick span and start from the loop's corners, as Blender does when Span is untouched."},
        "offset": {"kind": "int", "default": 0, "soft_min": -32, "soft_max": 32, "doc": "Rotate the loop start vertex (Blender Offset)."},
        "use_interp_simple": {"kind": "bool", "default": False, "doc": "Simple blending: interpolate by edge count instead of edge length (Blender Simple Blending)."},
        "output_group": {"kind": "bool", "default": True, "doc": "Write the new faces to a primitive group."},
        "output_group_name": {"kind": "str", "default": "grid_fill", "enabled_if": {"output_group": True}, "doc": "Name of the new faces' group."},
    }
    out["recalc_normals"] = {
        "group": {"kind": "str", "default": "", "doc": "Faces to orient (empty: all)."},
        "inside": {"kind": "bool", "default": False, "doc": "Point normals inward instead of outward (Blender Inside)."},
        "output_group": {"kind": "bool", "default": False, "doc": "Write the faces that were flipped to a primitive group."},
        "output_group_name": {"kind": "str", "default": "flipped", "enabled_if": {"output_group": True}, "doc": "Name of the flipped faces' group."},
    }
    out["array"] = {
        "count": {"kind": "int", "default": 2, "min": 1, "max": 10000, "doc": "Number of copies, including the original (1 leaves the input unchanged)."},
        "relative_offset": {"kind": "vec3", "default": [1.0, 0.0, 0.0], "doc": "Offset between copies as a fraction of the input bounds size per axis (Blender Relative Offset)."},
        "constant_offset": {"kind": "vec3", "default": [0.0, 0.0, 0.0], "doc": "Offset between copies in scene units, added to the relative offset (Blender Constant Offset)."},
        "merge_distance": {"kind": "float", "default": 0.0, "min": 0.0, "doc": "Weld points of each copy onto the previous copy within this distance (0: no merge). Object offset is not supported."},
    }
    out["tris_to_quads"] = {
        "group": {"kind": "str", "default": "", "doc": "Faces to merge (empty: all triangles)."},
        "face_threshold": {"kind": "float", "default": 40.0, "min": 0.0, "max": 180.0, "doc": "Maximum angle between the two triangle normals, in degrees (Blender Max Face Angle)."},
        "shape_threshold": {"kind": "float", "default": 40.0, "min": 0.0, "max": 180.0, "doc": "Maximum deviation of any quad corner from 90 degrees (Blender Max Shape Angle)."},
        "uvs": {"kind": "bool", "default": False, "doc": "Do not merge across a UV boundary (Compare UVs)."},
        "vcols": {"kind": "bool", "default": False, "doc": "Do not merge across a Cd colour difference (Compare Color Attributes)."},
        "seam": {"kind": "bool", "default": False, "doc": "Do not merge across edges in the edge group seam."},
        "sharp": {"kind": "bool", "default": False, "doc": "Do not merge across edges in the edge group sharp."},
        "materials": {"kind": "bool", "default": False, "doc": "Do not merge faces with different material_index (Compare Materials)."},
        "output_group": {"kind": "bool", "default": True, "doc": "Write the new quads to a primitive group."},
        "output_group_name": {"kind": "str", "default": "quads", "enabled_if": {"output_group": True}, "doc": "Name of the new quads' group."},
    }
    # Native-only Houdini Scatter and Align node (no Python registry entry).
    sa = {}
    def add(name, kind, default, doc, **extra):
        sa[name] = {"kind": kind, "default": default, "doc": doc, **extra}
    add("mode", "enum", "surface", "Scatter on the surface or inside a closed volume.", options=["surface", "volume"])
    add("group", "str", "", "Primitive group to scatter over; empty selects all faces.")
    add("tag_name", "str", "$OS", "Integer tag attribute name; $OS resolves to scatteralign.")
    add("global_seed", "int", 0, "Seed for deterministic point placement, rotation and scale.")
    add("coverage", "float", 1.0, "Multiplier for the generated point count.", min=0.0)
    add("point_count_method", "enum", "by_size", "Derive count from radius and surface area, or specify a fixed count.", options=["by_size", "fixed"])
    add("point_count", "int", 100, "Count when Point Count Method is Fixed; base count in volume mode.", min=0)
    add("scaling_mode", "enum", "uniform", "Use the mean radius or random radii between Min and Max.", options=["uniform", "random"])
    add("min_radius", "float", 0.1, "Smallest radius in random scaling; contributes to By Size count.", min=0.0)
    add("max_radius", "float", 0.1, "Largest radius in random scaling; contributes to By Size count.", min=0.0)
    add("uniform_scale", "float", 1.0, "Multiplier for the output pscale.", min=0.0)
    add("alignment", "enum", "normal", "Align local Z to the surface normal or keep local Z world up.", options=["normal", "world_up"])
    add("rotation", "bool", True, "Enable random cone rotation of the local Z axis.")
    add("max_random_cone_angle", "float", 0.0, "Maximum cone angle in degrees.", min=0.0, max=180.0)
    add("rotation_around_normal", "bool", True, "Enable spin around the aligned local Z axis.")
    add("min_angle", "float", 0.0, "Minimum spin angle in degrees.")
    add("max_angle", "float", 360.0, "Maximum spin angle in degrees.")
    add("round_to_multiple", "float", 0.0, "Round spin to this many degrees; zero disables rounding.", min=0.0)
    add("offset_angle", "float", 0.0, "Angle added after spin rounding, in degrees.")
    add("remap_using_distribution_ramp", "bool", False, "Remap spin randomness with a linear two-point ramp.")
    add("distribution_ramp_start", "float", 0.0, "Ramp value at zero; linear two-point ramps only.", min=0.0, max=1.0)
    add("distribution_ramp_end", "float", 1.0, "Ramp value at one; linear two-point ramps only.", min=0.0, max=1.0)
    add("copy_point_attributes", "bool", True, "Copy source point attributes to generated points.")
    add("point_attributes", "str", "*", "Space-separated source point attributes to copy; * copies all. Float attributes interpolate, integers use the dominant vertex.")
    out["scatteralign"] = sa
    # Rust-only native instances for Copy to Points (nodes/packed.rs); off keeps the Python-identical copy.
    out["copy_to_points"]["pack_instances"] = {"kind": "bool", "default": True, "doc": "Output native instances (one point per copy referencing the shared source, with its transform) instead of copying the source geometry; Unpack converts them to real geometry."}
    for node_id, params in package_meta().items():
        if node_id in out:
            raise ValueError(f"package node id conflicts with existing metadata: {node_id}")
        out[node_id] = params
    return out


def main() -> int:
    data = meta()  # Import/validate before opening the destination, preserving it on missing dependencies.
    with open(TARGET, "w", encoding="utf-8", newline="\n") as f:
        json.dump(data, f, indent=1, sort_keys=True)
        f.write("\n")
    print("wrote", TARGET)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
