"""How nodes are grouped in the editor's Add Node menu, what people call them, and what else a search should find.

One table drives the menu: ``GROUPS`` lists the groups in the order they appear, each with its nodes in the order they are
shown (the common tools first). ``NodeType.describe`` adds the result to every node's schema (``menu``, ``label``,
``keywords``), so the editor and AI tools see the same grouping. Nodes that are not listed here (a node you wrote, a system
saved from a selection) land in the group of their ``category``, or in **Library**.
"""
from __future__ import annotations

from typing import Dict, List, Tuple

LIBRARY = "Library"

# (group, what it is for, nodes)
GROUPS: List[Tuple[str, str, List[str]]] = [
    ("Create", "Shapes and curves from nothing, and the doors into a graph",
     ["box", "sphere", "grid", "tube", "torus", "circle", "line", "curve", "helix", "lsystem", "lattice_grid",
      "blender_object", "usd_import", "modifier_input"]),
    ("Polygon Modelling", "Build and edit polygon surfaces",
     ["polyextrude", "polybevel", "polysplit", "polyfill", "polybridge", "polyexpand2d", "subdivide", "mirror", "boolean",
      "clip", "dissolve", "divide", "reverse", "facet", "bridge", "skin", "skin_profiles", "revolve", "polyreduce", "clean",
      "convex_hull"]),
    ("Edges", "Work on edges and creases",
     ["dissolve_edges", "edge_collapse", "edge_flip", "edge_cusp", "crease", "edge_group", "edge_group_promote"]),
    ("Transform & Combine", "Move things, copy them, join them",
     ["transform", "copy_transform", "copy_to_points", "instance", "merge", "switch", "match_size", "bound"]),
    ("Deform", "Bend, twist, noise and smooth",
     ["bend", "twist", "linear_taper", "squash_stretch", "soft_transform", "soft_peak", "peak", "smooth", "noise", "lattice",
      "point_deform", "path_deform", "point_jitter", "point_relax", "blend", "exploded_view"]),
    ("Groups & Selection", "Pick parts of the geometry, delete or keep them",
     ["group_create", "group_expand", "group_promote", "group_range", "blast", "delete"]),
    ("Attributes", "Create, change and measure data on points, faces and corners",
     ["wrangle", "attribute_create", "attribute_transfer", "attribute_promote", "attribute_randomize", "attribute_blur",
      "attribute_rename", "attribute_delete", "measure", "normal", "color", "connectivity", "sort", "ray", "python"]),
    ("UV", "Texture coordinates", ["uv_project", "uv_flatten"]),
    ("Curves", "Curves and what to build from them",
     ["resample", "carve", "convert_line", "fillet", "polypath", "sweep"]),
    ("Points", "Point clouds and what to do with them",
     ["scatter", "fuse", "delaunay", "points_to_surface"]),
    ("Volumes & Terrain", "Remeshing, signed distance fields and terrain", ["remesh", "sdf_boolean", "heightfield_erode"]),
    ("Fracture & RBD", "Break things and simulate the pieces",
     ["rbd_material_fracture", "voronoi_fracture", "voronoi_fracture_points", "voronoi_split", "voronoi_adjacency",
      "boolean_fracture", "convex_decomposition", "assemble", "glue_cluster", "rbd_constraint_properties",
      "rbd_configure", "rbd_solver"]),
    ("Procedural (PCG)", "Rule-based point generation, filtering and placement",
     ["pcg_points", "pcg_surface_sampler", "pcg_volume_sampler", "pcg_spline_sampler", "pcg_spawner", "pcg_density_filter",
      "pcg_density_noise", "pcg_self_prune", "pcg_distance", "pcg_projection", "pcg_transform", "pcg_bounds",
      "pcg_copy_points", "pcg_union", "pcg_intersection", "pcg_difference", "pcg_debug"]),
    ("Flow & Time", "Loops, per-piece work and simulation over time",
     ["loop_block", "loop_begin", "loop_end", "for_each", "solver"]),
    ("Utility & I/O", "Pass-through, system inputs and the output", ["null", "input", "output"]),
    (LIBRARY, "Your own nodes and saved systems", []),
]

# a node that is not listed above goes where its category says (library nodes with a stock category sit with the stock nodes)
CATEGORY_GROUP: Dict[str, str] = {
    "source": "Create", "generate": "Create", "model": "Polygon Modelling", "modify": "Polygon Modelling",
    "combine": "Transform & Combine", "deform": "Deform", "group": "Groups & Selection", "attribute": "Attributes",
    "curve": "Curves", "volume": "Volumes & Terrain", "terrain": "Volumes & Terrain", "physics": "Fracture & RBD",
    "pcg": "Procedural (PCG)", "flow": "Flow & Time", "time": "Flow & Time", "utility": "Utility & I/O",
    "output": "Utility & I/O",
}

# names a node is known by elsewhere (Houdini, Blender, everyday words): a search for any of them finds it
ALIASES: Dict[str, str] = {
    "polyextrude": "extrude inset shell thicken", "polybevel": "bevel chamfer round edges", "polysplit": "edge loop insert cut",
    "polyfill": "fill hole cap", "polybridge": "bridge connect loops", "polyexpand2d": "offset outline inset curve expand",
    "subdivide": "catmull clark smooth subsurf subdivision", "mirror": "symmetry reflect flip", "boolean": "union subtract intersect cut csg",
    "clip": "cut plane slice", "dissolve": "merge faces planar", "divide": "triangulate quads", "reverse": "flip normals winding",
    "facet": "flat shading unique points", "skin": "loft", "revolve": "lathe spin", "polyreduce": "decimate simplify reduce",
    "clean": "cleanup remove degenerate", "convex_hull": "hull wrap",
    "dissolve_edges": "remove edges", "edge_collapse": "merge edge", "edge_flip": "swap diagonal", "edge_cusp": "hard edge sharp normals",
    "crease": "sharp subdivision weight", "transform": "move rotate scale translate pivot", "copy_transform": "duplicate array repeat stairs",
    "copy_to_points": "instance scatter copy stamp", "instance": "blender objects forest", "merge": "combine append join",
    "switch": "select input choose", "match_size": "fit center align", "bound": "bounding box",
    "bend": "curve warp", "twist": "spiral rotate along axis", "linear_taper": "taper scale along", "squash_stretch": "scale volume",
    "soft_transform": "proportional falloff", "soft_peak": "inflate falloff", "peak": "inflate push normals offset",
    "smooth": "relax laplacian blur", "noise": "mountain perlin displace random", "lattice": "ffd free form",
    "point_deform": "capture cage", "path_deform": "curve warp bend along", "point_jitter": "random move", "point_relax": "spread even",
    "blend": "morph shape keys blendshape", "exploded_view": "separate pieces",
    "group_create": "select group make", "group_expand": "grow shrink selection", "group_promote": "convert group points primitives",
    "group_range": "every other nth", "blast": "delete extract keep", "delete": "remove",
    "wrangle": "vex expression code snippet attribwrangle", "attribute_create": "add attribute", "attribute_transfer": "copy nearest project",
    "attribute_promote": "convert move domain", "attribute_randomize": "random", "attribute_blur": "smooth average",
    "attribute_rename": "name", "attribute_delete": "remove", "measure": "area volume perimeter curvature",
    "normal": "normals compute", "color": "colour", "connectivity": "pieces islands", "sort": "reorder", "ray": "project raycast hit",
    "python": "script code custom", "uv_project": "texture coordinates map", "uv_flatten": "unwrap lscm texture",
    "resample": "points along curve spacing", "carve": "trim curve", "convert_line": "polyline", "fillet": "round corners curve",
    "polypath": "path curve", "sweep": "polywire tube pipe along curve", "scatter": "random points distribute",
    "fuse": "weld merge points snap", "delaunay": "triangulate points", "points_to_surface": "meta balls blobby",
    "remesh": "retopo voxel rebuild", "sdf_boolean": "volume union", "heightfield_erode": "terrain erosion",
    "rbd_material_fracture": "shatter destruction concrete glass wood", "voronoi_fracture": "shatter break cells pieces",
    "voronoi_fracture_points": "scatter seeds shatter", "voronoi_split": "cut cells", "voronoi_adjacency": "neighbours touching",
    "boolean_fracture": "cutters break", "convex_decomposition": "collision proxy hulls", "assemble": "pieces names pivot pack",
    "glue_cluster": "constraints bond weld", "rbd_constraint_properties": "glue strength", "rbd_configure": "bodies mass friction physics",
    "rbd_solver": "simulate physics dynamics bullet", "loop_block": "repeat iterate for", "loop_begin": "repeat iterate",
    "loop_end": "repeat iterate", "for_each": "per piece iterate", "solver": "feedback simulation previous frame",
    "null": "named output pass through", "input": "system port", "output": "result display", "box": "cube", "sphere": "ball",
    "grid": "plane floor", "tube": "cylinder cone", "torus": "donut ring", "circle": "disc", "line": "segment",
    "curve": "spline path", "helix": "spiral coil", "lsystem": "plant tree fractal", "blender_object": "read import mesh", "usd_import": "usd usda usdc usdz open load file scene pixar omniverse houdini",
    "modifier_input": "host geometry",
}

_TITLES = {"polyextrude": "PolyExtrude", "polybevel": "PolyBevel", "polysplit": "PolySplit", "polyfill": "PolyFill",
           "polybridge": "PolyBridge", "polyexpand2d": "PolyExpand2D", "polyreduce": "PolyReduce", "polypath": "PolyPath",
           "uv_project": "UV Project", "uv_flatten": "UV Flatten", "sdf_boolean": "SDF Boolean", "pcg_points": "PCG Points",
           "lsystem": "L-System", "rbd_configure": "RBD Configure", "rbd_solver": "RBD Solver",
           "rbd_material_fracture": "RBD Material Fracture", "rbd_constraint_properties": "RBD Constraint Properties",
           "loop_block": "Repeat (Loop Block)", "for_each": "For Each", "points_to_surface": "Points to Surface",
           "copy_to_points": "Copy to Points", "blender_object": "Blender Object", "usd_import": "USD Import", "modifier_input": "Modifier Input"}

_ORDER: Dict[str, Tuple[int, int]] = {}
for _gi, (_g, _doc, _names) in enumerate(GROUPS):
    for _ni, _n in enumerate(_names):
        if _n in _ORDER:
            raise ValueError(f"node {_n!r} is listed in two menu groups")
        _ORDER[_n] = (_gi, _ni)
_RANK = {g: i for i, (g, _d, _n) in enumerate(GROUPS)}


def label(name: str) -> str:
    """What the menu calls a node: ``polyextrude`` -> PolyExtrude, ``edge_collapse`` -> Edge Collapse."""
    if name in _TITLES:
        return _TITLES[name]
    if name not in _ORDER:                      # a node somebody wrote: show the name they chose
        return name
    return " ".join(w.upper() if w in ("pcg", "uv", "rbd", "sdf") else w.capitalize() for w in name.split("_"))


def group_of(name: str, category: str = "", source: str = "") -> str:
    """The menu group of a node type: its table entry, else its category's group, else Library."""
    if name in _ORDER:
        return GROUPS[_ORDER[name][0]][0]
    if category == "system" or category == "custom" or not category:
        return LIBRARY
    return CATEGORY_GROUP.get(category, LIBRARY)


def info(name: str, category: str = "", source: str = "") -> dict:
    """The menu fields of a node's schema: ``menu`` (group), ``menu_rank`` / ``menu_order`` (sorting), ``label``, ``keywords``."""
    group = group_of(name, category, source)
    return {"menu": group, "menu_rank": _RANK[group], "menu_order": _ORDER.get(name, (0, 10_000))[1], "label": label(name),
            "keywords": ALIASES.get(name, "")}


def groups() -> List[dict]:
    """The groups in menu order, for editors: ``[{"name", "doc"}]``."""
    return [{"name": g, "doc": d} for g, d, _n in GROUPS]


def pseudo() -> List[dict]:
    """Menu entries that are not node types but the editor offers anyway (a loop block adds a begin/end pair)."""
    return [dict({"name": "loop_block", "category": "flow", "inputs": [{"name": "geo"}], "pseudo": True,
                  "doc": "Insert paired Loop Begin / Transform body / Loop End (repeat N times)"}, **info("loop_block", "flow"))]
