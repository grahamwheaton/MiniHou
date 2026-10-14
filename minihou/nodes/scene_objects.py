"""Scene objects: results that become real Blender objects, not part of the one MiniHou mesh.

``scene_objects`` is a sink (like a named ``output``): it passes its input through, and the live link (and a Blender modifier's
cook) turns what it receives into a **manifest** the add-on realises as ordinary scene objects:

* ``instances``: one linked duplicate of a Blender object per point, placed with the point's position, ``rot`` and ``scale``;
* ``pieces``: one mesh object per piece of the geometry (a ``piece`` primitive attribute, or connected pieces), each with its origin
  at the piece's centre;
* ``curves``: one Blender curves object holding every curve of the geometry (open primitives; polygons as cyclic outlines);
* ``points``: one Blender point-cloud object holding every point.

You can then give each object its own modifiers, materials, constraints; they are found again by a stable id, so the next cook moves
the same objects instead of making new ones. See :func:`build_manifest` for the wire form.
"""
import numpy as np

from .. import materials
from ..geometry import Geometry, rows
from ..sdk import Enum, Input, Int, Str, node

MAX_OBJECTS_LIMIT = 10_000


SCENE_SOURCE = "scene_source"            # detail attributes scene_objects leaves on its output so the link can build the manifest
SCENE_COLLECTION = "scene_collection"


@node("scene_objects", inputs=["geo", Input("source_object", optional=True, doc="Which Blender object(s) to duplicate: a blender_object / "
                                            "object_input (the object itself), a blender_collection (every object in it) or any geometry "
                                            "with a 'source' name in its detail attributes. Overrides the 'source' parameter"),
                           Input("target_collection", optional=True, doc="Where the objects go: a blender_object / blender_collection (its "
                                 "collection), or any geometry with a 'collection' name in its detail attributes. Overrides the "
                                 "'collection' parameter")], category="output", params=[
    Enum("mode", ["instances", "pieces", "curves", "points"],
         doc="instances: one Blender object per point (a linked duplicate of the object the point names), placed with the point's "
             "position, rot and scale; pieces: one mesh object per piece of the geometry (fracture pieces, say), origin at its centre; "
             "curves: a Blender curves object with every curve (poly curves, with radius and tilt); points: a Blender point cloud "
             "with every point (and its radius)"),
    Str("prefix", "MiniHou", doc="Object names start with this (Prefix_0001, ...)"),
    Str("collection", "", doc="Blender collection the objects go in (created if missing; empty: the prefix)"),
    Str("source", "", doc="Blender object to duplicate for points that do not say which (no 'instance' attribute); "
                          "points from the instance node name their objects themselves"),
    Str("id_attrib", "id", doc="instances: integer point attribute that gives each object a stable identity; without it the point "
                               "number is used (so changing how many points there are can hand objects to other points)"),
    Str("piece_attrib", "piece", doc="pieces: integer primitive attribute that numbers the pieces (it is the objects' identity); "
                                     "when the geometry has none, connected pieces are used, numbered in order"),
    Str("attributes", "uv Cd N material_index", doc="pieces, curves, points: attributes to carry onto the objects (space separated; the ones "
                                                  "that exist are sent; * is all of them): for pieces uv, Cd and N become Blender's UV map, "
                                                  "colour attribute and custom normals and material_index the face material; Cd on curves and "
                                                  "points becomes a colour attribute; anything else a generic attribute. radius and tilt always go"),
    Enum("pick", ["cycle", "random", "first"], doc="instances from several source objects (a whole collection wired into 'source') and "
                                                   "points that do not name one: cycle through them, pick at random, or always the first"),
    Int("seed", 0, doc="pick = random: the random seed"),
    Int("max_objects", 2000, min=1, max=MAX_OBJECTS_LIMIT, doc="Safety limit: more points than this is refused, not truncated"),
])
def scene_objects(geo, source_object, target_collection, mode, prefix, collection, source, id_attrib, piece_attrib, attributes, pick,
                  seed, max_objects):
    """Make Blender objects out of points. The geometry passes through unchanged; what happens in Blender is described by the
    manifest the link sends (see the manual). Objects are never deleted by a cook: ones that are no longer needed are hidden, and
    a Clean up button in the MiniHou panel removes them for good."""
    sources = names_from(source_object, "source")
    coll = names_from(target_collection, "collection")
    stale = SCENE_SOURCE in geo.detail or SCENE_COLLECTION in geo.detail          # left by an upstream scene_objects: not this node's wiring
    out = geo.copy() if (sources or coll or stale) else geo          # untouched when nothing is wired: the geometry itself passes through
    out.detail.pop(SCENE_SOURCE, None)
    out.detail.pop(SCENE_COLLECTION, None)
    if sources:
        out.detail[SCENE_SOURCE] = sources
    if coll:
        out.detail[SCENE_COLLECTION] = coll[0]
    plan(out, mode, source, id_attrib, max_objects, piece_attrib, attributes, pick, seed)       # validates: a bad setup is an error here
    return out


def names_from(geo, kind: str) -> list:
    """Blender object / collection names a wired input carries: ``kind`` is ``source`` or ``collection``.

    Detail attributes are read in order: ``object_name`` (a blender_object / object_input: the one object), ``collection_objects``
    (a blender_collection: every object in it), ``instance_objects`` (points from the instance node), and ``source`` /
    ``collection`` / ``collection_name`` (a name you set yourself, a string or a list of them)."""
    if geo is None:
        return []
    keys = ("object_name", "collection_objects", "instance_objects", "source") if kind == "source" else \
           ("collection_name", "collection", "object_collection")
    for k in keys:
        v = geo.detail.get(k)
        if isinstance(v, (list, tuple, np.ndarray)):
            v = [str(x).strip() for x in v if str(x).strip()]
            if v:
                return v
        elif isinstance(v, str) and v.strip():
            return [v.strip()]
    return []


def plan(geo: Geometry, mode: str, source: str, id_attrib: str, max_objects: int, piece_attrib: str = "piece", attributes: str = "",
         pick: str = "cycle", seed: int = 0):
    """The manifest columns for ``mode`` and the binary arrays that go with it (none for instances)."""
    if mode == "pieces":
        return plan_pieces(geo, piece_attrib, max_objects, attributes)
    if mode == "curves":
        return plan_curves(geo, attributes)
    if mode == "points":
        return plan_points(geo, attributes)
    return plan_instances(geo, source, id_attrib, max_objects, pick, seed), {}


MAX_PIECE_POINTS = 2_000_000


def plan_pieces(geo: Geometry, piece_attrib: str, max_objects: int, attributes: str = ""):
    """One mesh per piece. Columns (JSON): ``id`` (piece numbers), ``loc`` (each piece's centre), ``npts`` / ``ncorner`` / ``nprim``
    (counts per piece). Arrays: ``P`` float32 (points of all pieces, relative to their centres), ``vp`` int32 (corner -> point,
    local to the piece) and ``sizes`` int32 (corners per face), all pieces one after the other. Each wanted attribute that exists
    adds an array ``point:<name>`` (rows like ``P``), ``vertex:<name>`` (rows like ``vp``) or ``prim:<name>`` (rows like ``sizes``),
    and the column ``attributes`` lists their keys. The column ``materials`` (when the geometry has material names) is the name
    list for ``material_index``."""
    t = geo.topology
    offs = t.prim_offsets.astype(np.int64)
    sizes = np.diff(offs)
    mesh = np.flatnonzero(t.closed_mask() & (sizes >= 3))
    if not len(mesh):
        raise ValueError("pieces mode needs faces: there are none in the input")
    attr = geo.prim.get(piece_attrib) if piece_attrib else None
    if attr is not None:
        if attr.ndim != 1 or attr.dtype.kind not in "iu":
            raise ValueError(f"primitive attribute {piece_attrib!r} must be an integer to number the pieces")
        labels = attr[mesh].astype(np.int64)
    else:
        from .attrib_tools import piece_labels
        labels = piece_labels(geo)[1][mesh].astype(np.int64)
    uniq, inv = np.unique(labels, return_inverse=True)
    inv = inv.astype(np.int64).reshape(-1)
    if len(uniq) > max_objects:
        raise ValueError(f"{len(uniq):,} pieces is more than max_objects ({max_objects:,}); raise the limit (at most "
                         f"{MAX_OBJECTS_LIMIT:,}) or make fewer pieces")
    order = np.argsort(inv, kind="stable")
    prims = mesh[order]
    piece_of_prim = inv[order]
    s = sizes[prims]
    first = np.repeat(offs[prims], s)
    corner = first + np.arange(int(s.sum()), dtype=np.int64) - np.repeat(np.cumsum(s) - s, s)
    pt = t.vertex_point[corner].astype(np.int64)
    piece_of_corner = np.repeat(piece_of_prim, s)
    n = geo.npoints
    key = piece_of_corner * n + pt                                  # (piece, point) pairs: a point shared by two pieces is in both
    ukey, local = np.unique(key, return_inverse=True)
    local = local.astype(np.int64).reshape(-1)
    piece_of_point = ukey // n
    if len(ukey) > MAX_PIECE_POINTS:
        raise ValueError(f"the pieces have {len(ukey):,} points in all (limit {MAX_PIECE_POINTS:,})")
    P = np.asarray(geo.P, np.float64)[ukey % n]
    npts = np.bincount(piece_of_point, minlength=len(uniq))
    centre = np.stack([np.bincount(piece_of_point, weights=P[:, k], minlength=len(uniq)) for k in range(3)], 1) / npts[:, None]
    P_local = (P - centre[piece_of_point]).astype(np.float32)
    point_base = np.cumsum(npts) - npts
    vp = (local - point_base[piece_of_corner]).astype(np.int32)
    ncorner = np.bincount(piece_of_corner, minlength=len(uniq))
    nprim = np.bincount(piece_of_prim, minlength=len(uniq))
    arrays = {"P": np.ascontiguousarray(P_local), "vp": vp, "sizes": s.astype(np.int32)}
    carried = []
    for name in dict.fromkeys(str(attributes).split()):
        for domain, table, rows_ in (("point", geo.point, ukey % n), ("vertex", geo.vertex, corner), ("prim", geo.prim, prims)):
            a = table.get(name)
            if a is not None and name != "P":
                arrays[f"{domain}:{name}"] = np.ascontiguousarray(a[rows_])
                carried.append(f"{domain}:{name}")
    cols = {"sources": [], "id": uniq.tolist(), "loc": np.round(centre, 6).tolist(), "npts": npts.tolist(),
            "ncorner": ncorner.tolist(), "nprim": nprim.tolist(), "attributes": carried}
    names = materials.names(geo)
    if names:
        cols["materials"] = names                         # slot i of every piece gets material names[i] (see minihou.materials)
    return cols, arrays


def _wanted(attributes: str, always=()) -> tuple:
    """(names, everything) from the ``attributes`` parameter."""
    names = list(dict.fromkeys(str(attributes).split()))
    return [n for n in names if n != "*"] + [a for a in always if a not in names], "*" in names


def _supported(a) -> bool:
    return a.dtype.kind in "fiub" and a.ndim in (1, 2) and (a.ndim == 1 or a.shape[1] <= 4)


def plan_curves(geo: Geometry, attributes: str = ""):
    """One Blender curves object: ``P`` (a point per corner of every kept primitive, float32), ``sizes`` (points per curve, int32) and
    ``cyclic`` (uint8 per curve: closed primitives), plus ``point:<name>`` (rows like ``P``; vertex attributes too) and ``prim:<name>``
    (one per curve). Primitives with fewer than 2 corners are left out. Columns: ``npts``, ``ncurve``, ``attributes``."""
    t = geo.topology
    offs = t.prim_offsets.astype(np.int64)
    sizes = np.diff(offs)
    keep = np.flatnonzero(sizes >= 2)
    if not len(keep):
        raise ValueError("curves mode needs curves or polygons: the input has no primitive with 2 or more points")
    s = sizes[keep]
    n = int(s.sum())
    if n > MAX_PIECE_POINTS:
        raise ValueError(f"{n:,} curve points is more than the limit ({MAX_PIECE_POINTS:,})")
    corner = np.repeat(offs[keep], s) + np.arange(n, dtype=np.int64) - np.repeat(np.cumsum(s) - s, s)
    pt = t.vertex_point.astype(np.int64)[corner]
    arrays = {"P": np.ascontiguousarray(np.asarray(geo.P, np.float32)[pt]), "sizes": s.astype(np.int32),
              "cyclic": (t.closed_mask()[keep] & (s >= 3)).astype(np.uint8)}
    names, every = _wanted(attributes, ("radius", "tilt"))
    carried = []
    for domain, table, rows_ in (("point", geo.point, pt), ("point", geo.vertex, corner), ("prim", geo.prim, keep)):
        for name in (list(table.names()) if every else names):
            a = table.get(name)
            key = f"{domain}:{name}"
            if a is None or name == "P" or key in arrays or name.startswith("group_") or not _supported(a):
                continue
            arrays[key] = np.ascontiguousarray(a[rows_])
            carried.append(key)
    cols = {"sources": [], "id": [0], "loc": [[0.0, 0.0, 0.0]], "npts": [n], "ncurve": [int(len(keep))], "attributes": carried}
    return cols, arrays


def plan_points(geo: Geometry, attributes: str = ""):
    """One Blender point cloud: ``P`` (every point, float32) and ``point:<name>`` attributes. Column: ``npts``, ``attributes``."""
    n = geo.npoints
    if n == 0:
        raise ValueError("points mode needs points: the input has none")
    if n > MAX_PIECE_POINTS:
        raise ValueError(f"{n:,} points is more than the limit ({MAX_PIECE_POINTS:,})")
    arrays = {"P": np.ascontiguousarray(np.asarray(geo.P, np.float32))}
    names, every = _wanted(attributes, ("radius",))
    carried = []
    for name in (list(geo.point.names()) if every else names):
        a = geo.point.get(name)
        if a is None or name == "P" or name.startswith("group_") or not _supported(a):
            continue
        arrays[f"point:{name}"] = np.ascontiguousarray(a)
        carried.append(f"point:{name}")
    cols = {"sources": [], "id": [0], "loc": [[0.0, 0.0, 0.0]], "npts": [n], "attributes": carried}
    return cols, arrays


def plan_instances(geo: Geometry, source: str, id_attrib: str, max_objects: int, pick: str = "cycle", seed: int = 0) -> dict:
    """Which points become objects, which Blender object each duplicates, their ids and transforms (columnar, JSON-ready)."""
    n = geo.npoints
    inst = geo.point.get("instance")
    names = geo.detail.get("instance_objects")
    src_names: list = []
    if inst is not None and inst.ndim == 1 and isinstance(names, (list, tuple)) and names:
        src_names = [str(x) for x in names]
        pick = np.flatnonzero((inst > 0) & (inst <= len(src_names)))
        src_index = inst[pick].astype(np.int64) - 1
    else:
        wired = geo.detail.get(SCENE_SOURCE)
        src_names = [str(x) for x in wired] if wired else ([str(source).strip()] if str(source).strip() else [])
        if not src_names:
            raise ValueError("set 'source' to the Blender object to duplicate, wire an object into the 'source' input (or feed points "
                             "from the instance node, which name their objects)")
        k = len(src_names)
        if k == 1 or pick == "first":
            src_index = np.zeros(n, np.int64)
        elif pick == "random":
            src_index = np.random.default_rng(int(seed)).integers(0, k, n)
        else:
            src_index = np.arange(n) % k
        pick = np.arange(n)
    if len(pick) > max_objects:
        raise ValueError(f"{len(pick):,} objects is more than max_objects ({max_objects:,}); raise the limit (at most "
                         f"{MAX_OBJECTS_LIMIT:,}) or make fewer points")
    ids = np.arange(len(pick), dtype=np.int64)
    idattr = geo.point.get(id_attrib) if id_attrib else None
    if idattr is not None and idattr.ndim == 1 and idattr.dtype.kind in "iu":
        ids = idattr[pick].astype(np.int64)
        if len(np.unique(ids)) != len(ids):
            raise ValueError(f"point attribute {id_attrib!r} has repeated values, so it cannot identify objects; "
                             f"fix it or clear id_attrib to use point numbers")
    elif idattr is not None:
        raise ValueError(f"point attribute {id_attrib!r} must be an integer to identify objects")
    P = np.asarray(geo.P, np.float64)[pick]
    rot = geo.point.get("rot")
    rot = rows(rot)[pick, :3].astype(np.float64) if rot is not None and rot.ndim == 2 and rot.shape[1] == 3 else np.zeros((len(pick), 3))
    scale = geo.point.get("scale")
    if scale is not None and scale.ndim == 2 and scale.shape[1] == 3:
        sc = rows(scale)[pick].astype(np.float64)
    else:
        ps = geo.point.get("pscale")
        sc = np.repeat((ps[pick].astype(np.float64) if ps is not None and ps.ndim == 1 else np.ones(len(pick)))[:, None], 3, 1)
    cols = {"sources": src_names, "id": ids.tolist(), "src": src_index.tolist(), "loc": np.round(P, 6).tolist(),
            "rot": np.round(rot, 7).tolist(), "scale": np.round(sc, 6).tolist()}
    mats = materials.names(geo)
    if mats and materials.POINT_ATTR in geo.point:        # set_material on points: each object's own material (its first slot)
        idx = materials.indices(geo, "point")[pick]
        col = [mats[i] if 0 <= i < len(mats) else "" for i in idx]
        if any(col):
            cols["material"] = col
    return cols


def build_manifest(node_id: str, params: dict, geo: Geometry, key: str):
    """``(header, arrays)``: the message the add-on realises, ``{"type": "scene_objects", "node", "key", "mode", "prefix",
    "collection", ...columns}`` plus binary arrays for ``pieces``. Instance columns: ``sources`` (Blender object names), and per
    object ``id``, ``src`` (index into sources), ``loc``, ``rot`` (Euler XYZ radians), ``scale`` and, when ``set_material`` ran on the
    points, ``material`` (a name, ``""`` for none). Piece columns: see
    :func:`plan_pieces`."""
    body, arrays = plan(geo, params["mode"], params["source"], params["id_attrib"], params["max_objects"], params["piece_attrib"],
                        params["attributes"], params.get("pick", "cycle"), params.get("seed", 0))
    prefix = str(params["prefix"]).strip() or "MiniHou"
    wired = geo.detail.get(SCENE_COLLECTION)
    header = dict(type="scene_objects", node=node_id, key=key, mode=params["mode"], prefix=prefix[:60],
                  collection=(str(wired or params["collection"]).strip() or prefix)[:60], **body)
    return header, arrays


def build_sink_manifest(node_type: str, node_id: str, params: dict, geo: Geometry, key: str):
    """:func:`build_manifest` for a ``scene_objects`` node, the image manifest for an ``image_out`` node."""
    if node_type == "image_out":
        from .image_nodes import build_image_manifest
        return build_image_manifest(node_id, params, geo, key)
    return build_manifest(node_id, params, geo, key)
