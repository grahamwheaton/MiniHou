"""Scene objects: results that become real Blender objects, not part of the one MiniHou mesh.

``scene_objects`` is a sink (like a named ``output``): it passes its input through, and the live link (and a Blender modifier's
cook) turns what it receives into a **manifest** the add-on realises as ordinary scene objects:

* ``instances``: one linked duplicate of a Blender object per point, placed with the point's position, ``rot`` and ``scale``;
* ``pieces``: one mesh object per piece of the geometry (a ``piece`` primitive attribute, or connected pieces), each with its origin
  at the piece's centre.

You can then give each object its own modifiers, materials, constraints; they are found again by a stable id, so the next cook moves
the same objects instead of making new ones. See :func:`build_manifest` for the wire form.
"""
import numpy as np

from ..geometry import Geometry, rows
from ..sdk import Enum, Int, Str, node

MAX_OBJECTS_LIMIT = 10_000


@node("scene_objects", inputs=["geo"], category="output", params=[
    Enum("mode", ["instances", "pieces"], doc="instances: one Blender object per point (a linked duplicate of the object the point "
                                              "names), placed with the point's position, rot and scale; pieces: one mesh object per "
                                              "piece of the geometry (fracture pieces, say), origin at its centre"),
    Str("prefix", "MiniHou", doc="Object names start with this (Prefix_0001, ...)"),
    Str("collection", "", doc="Blender collection the objects go in (created if missing; empty: the prefix)"),
    Str("source", "", doc="Blender object to duplicate for points that do not say which (no 'instance' attribute); "
                          "points from the instance node name their objects themselves"),
    Str("id_attrib", "id", doc="instances: integer point attribute that gives each object a stable identity; without it the point "
                               "number is used (so changing how many points there are can hand objects to other points)"),
    Str("piece_attrib", "piece", doc="pieces: integer primitive attribute that numbers the pieces (it is the objects' identity); "
                                     "when the geometry has none, connected pieces are used, numbered in order"),
    Str("attributes", "uv Cd N material_index", doc="pieces: attributes to carry onto each piece's mesh (space separated; the ones that exist "
                                                  "are sent): uv, Cd and N become Blender's UV map, colour attribute and custom normals, "
                                                  "material_index the face material, anything else a generic attribute"),
    Int("max_objects", 2000, min=1, max=MAX_OBJECTS_LIMIT, doc="Safety limit: more points than this is refused, not truncated"),
])
def scene_objects(geo, mode, prefix, collection, source, id_attrib, piece_attrib, attributes, max_objects):
    """Make Blender objects out of points. The geometry passes through unchanged; what happens in Blender is described by the
    manifest the link sends (see the manual). Objects are never deleted by a cook: ones that are no longer needed are hidden, and
    a Clean up button in the MiniHou panel removes them for good."""
    plan(geo, mode, source, id_attrib, max_objects, piece_attrib, attributes)       # validates: a bad setup is an error on this node
    return geo


def plan(geo: Geometry, mode: str, source: str, id_attrib: str, max_objects: int, piece_attrib: str = "piece", attributes: str = ""):
    """The manifest columns for ``mode`` and the binary arrays that go with it (none for instances)."""
    if mode == "pieces":
        return plan_pieces(geo, piece_attrib, max_objects, attributes)
    return plan_instances(geo, source, id_attrib, max_objects), {}


MAX_PIECE_POINTS = 2_000_000


def plan_pieces(geo: Geometry, piece_attrib: str, max_objects: int, attributes: str = ""):
    """One mesh per piece. Columns (JSON): ``id`` (piece numbers), ``loc`` (each piece's centre), ``npts`` / ``ncorner`` / ``nprim``
    (counts per piece). Arrays: ``P`` float32 (points of all pieces, relative to their centres), ``vp`` int32 (corner -> point,
    local to the piece) and ``sizes`` int32 (corners per face), all pieces one after the other. Each wanted attribute that exists
    adds an array ``point:<name>`` (rows like ``P``), ``vertex:<name>`` (rows like ``vp``) or ``prim:<name>`` (rows like ``sizes``),
    and the column ``attributes`` lists their keys."""
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
    return cols, arrays


def plan_instances(geo: Geometry, source: str, id_attrib: str, max_objects: int) -> dict:
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
        if not str(source).strip():
            raise ValueError("set 'source' to the Blender object to duplicate (or feed points from the instance node, which name their objects)")
        src_names = [str(source).strip()]
        pick = np.arange(n)
        src_index = np.zeros(n, np.int64)
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
    return {"sources": src_names, "id": ids.tolist(), "src": src_index.tolist(), "loc": np.round(P, 6).tolist(),
            "rot": np.round(rot, 7).tolist(), "scale": np.round(sc, 6).tolist()}


def build_manifest(node_id: str, params: dict, geo: Geometry, key: str):
    """``(header, arrays)``: the message the add-on realises, ``{"type": "scene_objects", "node", "key", "mode", "prefix",
    "collection", ...columns}`` plus binary arrays for ``pieces``. Instance columns: ``sources`` (Blender object names), and per
    object ``id``, ``src`` (index into sources), ``loc``, ``rot`` (Euler XYZ radians), ``scale``. Piece columns: see
    :func:`plan_pieces`."""
    body, arrays = plan(geo, params["mode"], params["source"], params["id_attrib"], params["max_objects"], params["piece_attrib"],
                        params["attributes"])
    prefix = str(params["prefix"]).strip() or "MiniHou"
    header = dict(type="scene_objects", node=node_id, key=key, mode=params["mode"], prefix=prefix[:60],
                  collection=(str(params["collection"]).strip() or prefix)[:60], **body)
    return header, arrays
