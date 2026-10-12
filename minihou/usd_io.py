"""Universal Scene Description (USD) export and import of cooked geometry.

The graph stays JSON (a recipe: nodes, parameters, wires); USD carries the *result*, for Houdini, Blender, Maya, Omniverse and
usdview. ``pxr`` (``pip install usd-core``) is optional: nothing here imports it until a file is read or written.

**What a stage looks like** (``write_stage``): ``/MiniHou`` (the default prim, Z up, metres) holds one ``Xform`` per output
(the display node, plus each extra output). A geometry is split by what USD can hold, under that Xform:

* ``mesh``: a ``Mesh`` of the closed faces with 3+ corners (``subdivisionScheme = none``, right handed: faces wind counter
  clockwise seen from outside, as in MiniHou and Blender);
* ``curves``: a ``BasisCurves`` (linear, not periodic) of the open primitives. USD curves do not index into a shared point
  table, so their points are expanded;
* ``points``: a ``Points`` of the points no primitive uses.

Closed primitives with fewer than 3 corners and open ones with fewer than 2 cannot be drawn by USD; they are left out and counted
in the Xform's ``customData`` (``minihou:dropped``).

**Attributes** become primvars. MiniHou's point / corner / primitive tables are USD's ``vertex`` / ``faceVarying`` (``vertex`` on
curves, whose corners and points coincide) / ``uniform``. ``N`` is written as ``normals``, ``Cd`` as ``displayColor``, ``Alpha`` as
``displayOpacity`` and ``uv`` as ``primvars:st``, so other tools find them; everything else keeps its own name (made a valid USD
identifier; the original is in the primvar's ``customData``). Detail attributes are JSON in the Xform's ``customData``
(``minihou:detail``). The graph itself can be stored in the layer's ``customLayerData`` (``minihou:graph``).

**Round trip.** Every prim also carries ``primvars:minihou:point_index`` (the point's number in the exported geometry) and, for
meshes and curves, ``primvars:minihou:prim_index``, so reading a MiniHou file back rebuilds the exact point table and primitive
order (points shared by a mesh and a curve are shared again). Groups are int primvars (``group_<name>``, ``edge_group_<name>``) and
also face / point ``GeomSubset``s on the mesh, which is what other tools look for; on reading, the primvars win and subsets fill in
for files that have only those.

**Reading** (``read_geometry``) accepts any USD file: ``Mesh`` (indexed or not, any orientation, subsets), ``BasisCurves`` (cubic
curves come in as their control polygon) and ``Points``, with each prim's world transform applied. Other prim types are skipped and
listed in the report.
"""
from __future__ import annotations

import json
import os
import re
from dataclasses import dataclass, field
from typing import Dict, List, Optional, Tuple

import numpy as np

from .geometry import Geometry

USD_EXTENSIONS = (".usd", ".usda", ".usdc")
ROOT = "MiniHou"
SPECIAL = {"N": "normals", "Cd": "displayColor", "Alpha": "displayOpacity", "uv": "st"}      # MiniHou name -> USD name
INTERNAL = "minihou:"                                                                          # primvars that are bookkeeping
_IDENT = re.compile(r"[^A-Za-z0-9_]")


class UsdUnavailable(RuntimeError):
    pass


def pxr():
    """The ``pxr`` modules, or a clear error when ``usd-core`` is not installed."""
    try:
        from pxr import Gf, Sdf, Tf, Usd, UsdGeom, Vt
    except ImportError as e:
        raise UsdUnavailable("USD support needs the 'usd-core' package: pip install usd-core") from e
    return Gf, Sdf, Tf, Usd, UsdGeom, Vt


def available() -> bool:
    try:
        pxr()
        return True
    except UsdUnavailable:
        return False


def valid_name(name: str) -> str:
    n = _IDENT.sub("_", str(name))
    return ("_" + n) if (not n or n[0].isdigit()) else n


# -- numpy <-> Vt --------------------------------------------------------------------------------------------------------
def _vt(a: np.ndarray):
    """(Vt array, Sdf type name, element size) for a MiniHou attribute array."""
    Gf, Sdf, Tf, Usd, UsdGeom, Vt = pxr()
    T = Sdf.ValueTypeNames
    k = 1 if a.ndim == 1 else a.shape[1]
    if a.dtype.kind == "f":
        a = np.ascontiguousarray(a, np.float32)
        if k == 1:
            return Vt.FloatArray.FromNumpy(a), T.FloatArray, 1
        if 2 <= k <= 4:
            return getattr(Vt, f"Vec{k}fArray").FromNumpy(a), getattr(T, f"Float{k}Array"), 1
        return Vt.FloatArray.FromNumpy(a.reshape(-1)), T.FloatArray, k
    if a.dtype.kind == "b":
        flat = np.ascontiguousarray(a.reshape(-1))
        return Vt.BoolArray.FromNumpy(flat), T.BoolArray, k
    a = np.ascontiguousarray(a, np.int32)
    if k == 1:
        return Vt.IntArray.FromNumpy(a), T.IntArray, 1
    if 2 <= k <= 4:
        return getattr(Vt, f"Vec{k}iArray").FromNumpy(a), getattr(T, f"Int{k}Array"), 1
    return Vt.IntArray.FromNumpy(a.reshape(-1)), T.IntArray, k


def _np(v, ncomp: int = 1) -> np.ndarray:
    """A Vt array as numpy shaped (n,) or (n, ncomp)."""
    a = np.asarray(v)
    if ncomp > 1 and a.ndim == 1:
        a = a.reshape(-1, ncomp)
    return a


def _json_safe(v):
    if isinstance(v, np.ndarray):
        return v.tolist()
    if isinstance(v, (np.integer,)):
        return int(v)
    if isinstance(v, (np.floating,)):
        return float(v)
    if isinstance(v, (np.bool_,)):
        return bool(v)
    if isinstance(v, dict):
        return {str(k): _json_safe(x) for k, x in v.items()}
    if isinstance(v, (list, tuple)):
        return [_json_safe(x) for x in v]
    if isinstance(v, (str, int, float, bool)) or v is None:
        return v
    return str(v)


# -- writing -------------------------------------------------------------------------------------------------------------
@dataclass
class WriteReport:
    path: str = ""
    outputs: Dict[str, dict] = field(default_factory=dict)

    def to_dict(self) -> dict:
        return {"path": self.path, "outputs": self.outputs}


def _set_primvar(prim, usd_name: str, arr: np.ndarray, interp: str, original: str, role: str = "", extra: Optional[dict] = None):
    Gf, Sdf, Tf, Usd, UsdGeom, Vt = pxr()
    v, tname, esize = _vt(arr)
    if role == "color" and tname == Sdf.ValueTypeNames.Float3Array:
        tname = Sdf.ValueTypeNames.Color3fArray
    elif role == "texcoord" and tname == Sdf.ValueTypeNames.Float2Array:
        tname = Sdf.ValueTypeNames.TexCoord2fArray
    pv = UsdGeom.PrimvarsAPI(prim).CreatePrimvar(usd_name, tname, interp, esize if esize > 1 else -1)
    pv.Set(v)
    pv.GetAttr().SetCustomDataByKey("minihou:name", original)
    for k, x in (extra or {}).items():
        pv.GetAttr().SetCustomDataByKey(k, x)
    return pv


def _unique_name(base: str, used: set, suffix: str) -> str:
    name = base
    if name in used:
        name = f"{base}__{suffix}"
    k = 2
    while name in used:
        name, k = f"{base}__{suffix}{k}", k + 1
    used.add(name)
    return name


def _write_attributes(prim, tables: List[Tuple[Dict[str, np.ndarray], str, str]]):
    """Write attribute dictionaries as primvars. ``tables``: [(attrs, interpolation, MiniHou domain)]. The domain is stored on
    each primvar, because on curves a point attribute and a corner attribute are both ``vertex``."""
    Gf, Sdf, Tf, Usd, UsdGeom, Vt = pxr()
    used: set = set()
    geom = UsdGeom.PointBased(prim) if prim.IsA(UsdGeom.PointBased) else None
    for attrs, interp, dom in tables:
        tag = {"minihou:domain": dom}
        for name, arr in attrs.items():
            special = SPECIAL.get(name)
            a = np.asarray(arr)
            if special == "normals" and a.ndim == 2 and a.shape[1] == 3 and a.dtype.kind == "f" and geom is not None \
                    and "normals" not in used and prim.IsA(UsdGeom.Mesh):
                used.add("normals")
                na = geom.CreateNormalsAttr()
                na.Set(Vt.Vec3fArray.FromNumpy(np.ascontiguousarray(a, np.float32)))
                geom.SetNormalsInterpolation(interp)
                na.SetCustomDataByKey("minihou:name", "N")
                na.SetCustomDataByKey("minihou:domain", dom)
                continue
            if special == "displayColor" and a.ndim == 2 and a.shape[1] == 3 and a.dtype.kind == "f":
                _set_primvar(prim, _unique_name("displayColor", used, dom), a, interp, name, role="color", extra=tag)
                continue
            if special == "displayOpacity" and a.ndim == 1 and a.dtype.kind == "f":
                _set_primvar(prim, _unique_name("displayOpacity", used, dom), a, interp, name, extra=tag)
                continue
            if special == "st" and a.ndim == 2 and a.shape[1] == 2 and a.dtype.kind == "f":
                _set_primvar(prim, _unique_name("st", used, dom), a, interp, name, role="texcoord", extra=tag)
                continue
            base = valid_name(name)
            if base.startswith("minihou"):
                base = "attr_" + base
            _set_primvar(prim, _unique_name(base, used, dom), a, interp, name, extra=tag)


def _gather_table(table, idx: np.ndarray, exclude=()) -> Dict[str, np.ndarray]:
    return {n: np.ascontiguousarray(a[idx]) for n, a in table.items() if n not in exclude}


def write_geometry(stage, path: str, geo: Geometry) -> dict:
    """Write ``geo`` under the Xform ``path`` of ``stage``; returns the counts written."""
    Gf, Sdf, Tf, Usd, UsdGeom, Vt = pxr()
    xf = UsdGeom.Xform.Define(stage, path)
    t = geo.topology
    n = geo.npoints
    P = np.ascontiguousarray(geo.P, np.float32)
    vp = t.vertex_point.astype(np.int64)
    offs = t.prim_offsets.astype(np.int64)
    sizes = np.diff(offs)
    closed = t.closed_mask()
    mesh_prims = np.flatnonzero(closed & (sizes >= 3))
    curve_prims = np.flatnonzero(~closed & (sizes >= 2))
    dropped = {"faces": int((closed & (sizes < 3)).sum()), "curves": int((~closed & (sizes < 2)).sum())}
    used = np.zeros(n, bool)
    if len(vp):
        used[vp] = True
    loose = np.flatnonzero(~used)
    out = {"faces": 0, "curves": 0, "points": 0, "dropped": dropped}

    def corners_of(prims: np.ndarray) -> np.ndarray:
        """Corner (vertex) numbers of ``prims`` in order."""
        if not len(prims):
            return np.zeros(0, np.int64)
        s = sizes[prims]
        first = np.repeat(offs[prims], s)
        k = np.arange(int(s.sum()), dtype=np.int64) - np.repeat(np.cumsum(s) - s, s)
        return first + k

    point_t = geo.point
    # -- mesh ---------------------------------------------------------------------------------------------------------
    if len(mesh_prims):
        cv = corners_of(mesh_prims)
        pts = np.unique(vp[cv])                                       # original point numbers the mesh uses, ascending
        local = np.zeros(n, np.int64)
        local[pts] = np.arange(len(pts))
        mesh = UsdGeom.Mesh.Define(stage, f"{path}/mesh")
        mesh.CreatePointsAttr().Set(Vt.Vec3fArray.FromNumpy(np.ascontiguousarray(P[pts])))
        mesh.CreateFaceVertexCountsAttr().Set(Vt.IntArray.FromNumpy(sizes[mesh_prims].astype(np.int32)))
        mesh.CreateFaceVertexIndicesAttr().Set(Vt.IntArray.FromNumpy(local[vp[cv]].astype(np.int32)))
        mesh.CreateSubdivisionSchemeAttr().Set("none")
        mesh.CreateOrientationAttr().Set("rightHanded")
        mesh.CreateDoubleSidedAttr().Set(False)
        prim = mesh.GetPrim()
        lo, hi = P[pts].min(0), P[pts].max(0)
        mesh.CreateExtentAttr().Set(Vt.Vec3fArray.FromNumpy(np.stack([lo, hi]).astype(np.float32)))
        _set_primvar(prim, "minihou:point_index", pts.astype(np.int32), "vertex", "")
        _set_primvar(prim, "minihou:prim_index", mesh_prims.astype(np.int32), "uniform", "")
        _write_attributes(prim, [(_gather_table(point_t, pts, ("P",)), "vertex", "point"),
                                 (_gather_table(geo.vertex, cv), "faceVarying", "vertex"),
                                 (_gather_table(geo.prim, mesh_prims), "uniform", "prim")])
        _write_group_subsets(mesh, geo, mesh_prims, pts)
        out["faces"] = int(len(mesh_prims))
    # -- curves -------------------------------------------------------------------------------------------------------
    if len(curve_prims):
        cv = corners_of(curve_prims)
        src = vp[cv]                                                   # expanded: one USD point per corner
        cur = UsdGeom.BasisCurves.Define(stage, f"{path}/curves")
        cur.CreateTypeAttr().Set("linear")
        cur.CreateWrapAttr().Set("nonperiodic")
        cur.CreateCurveVertexCountsAttr().Set(Vt.IntArray.FromNumpy(sizes[curve_prims].astype(np.int32)))
        cur.CreatePointsAttr().Set(Vt.Vec3fArray.FromNumpy(np.ascontiguousarray(P[src])))
        cur.CreateWidthsAttr().Set(Vt.FloatArray([0.01]))
        cur.SetWidthsInterpolation("constant")
        prim = cur.GetPrim()
        cur.CreateExtentAttr().Set(Vt.Vec3fArray.FromNumpy(np.stack([P[src].min(0), P[src].max(0)]).astype(np.float32)))
        _set_primvar(prim, "minihou:point_index", src.astype(np.int32), "vertex", "")
        _set_primvar(prim, "minihou:prim_index", curve_prims.astype(np.int32), "uniform", "")
        _write_attributes(prim, [(_gather_table(point_t, src, ("P",)), "vertex", "point"),
                                 (_gather_table(geo.vertex, cv), "vertex", "vertex"),
                                 (_gather_table(geo.prim, curve_prims), "uniform", "prim")])
        out["curves"] = int(len(curve_prims))
    # -- loose points -------------------------------------------------------------------------------------------------
    if len(loose):
        pp = UsdGeom.Points.Define(stage, f"{path}/points")
        pp.CreatePointsAttr().Set(Vt.Vec3fArray.FromNumpy(np.ascontiguousarray(P[loose])))
        pp.CreateWidthsAttr().Set(Vt.FloatArray([0.02]))
        pp.SetWidthsInterpolation("constant")
        prim = pp.GetPrim()
        pp.CreateExtentAttr().Set(Vt.Vec3fArray.FromNumpy(np.stack([P[loose].min(0), P[loose].max(0)]).astype(np.float32)))
        _set_primvar(prim, "minihou:point_index", loose.astype(np.int32), "vertex", "")
        _write_attributes(prim, [(_gather_table(point_t, loose, ("P",)), "vertex", "point")])
        out["points"] = int(len(loose))
    xf.GetPrim().SetCustomDataByKey("minihou:counts", {"points": int(n), "prims": int(t.nprims)})
    xf.GetPrim().SetCustomDataByKey("minihou:dropped", dropped)
    if geo.detail:
        xf.GetPrim().SetCustomDataByKey("minihou:detail", json.dumps(_json_safe(geo.detail), allow_nan=False, default=str)
                                        if _finite(geo.detail) else json.dumps(_json_safe(_clean(geo.detail)), default=str))
    return out


def _finite(v) -> bool:
    try:
        json.dumps(_json_safe(v), allow_nan=False)
        return True
    except ValueError:
        return False


def _clean(v):
    """``v`` with NaN / infinity replaced by None (JSON has no such numbers)."""
    if isinstance(v, float) and not np.isfinite(v):
        return None
    if isinstance(v, dict):
        return {k: _clean(x) for k, x in v.items()}
    if isinstance(v, (list, tuple)):
        return [_clean(x) for x in v]
    return v


def _write_group_subsets(mesh, geo: Geometry, mesh_prims: np.ndarray, pts: np.ndarray) -> None:
    """Face and point ``GeomSubset``s for every prim / point group, for tools that look for those (the primvars stay the truth)."""
    Gf, Sdf, Tf, Usd, UsdGeom, Vt = pxr()
    used: set = set()
    for kind, table, rows, family, element in (("prim", geo.prim, mesh_prims, "minihou_face_groups", "face"),
                                               ("point", geo.point, pts, "minihou_point_groups", "point")):
        wrote = False
        for name, a in table.items():
            if not name.startswith("group_") or a.ndim != 1:
                continue
            idx = np.flatnonzero(np.asarray(a)[rows] != 0)
            if not len(idx):
                continue
            sub_name = _unique_name("g_" + valid_name(name[len("group_"):]), used, kind)
            sub = UsdGeom.Subset.CreateGeomSubset(mesh, sub_name, element, Vt.IntArray.FromNumpy(idx.astype(np.int32)), family,
                                                  UsdGeom.Tokens.unrestricted)
            sub.GetPrim().SetCustomDataByKey("minihou:group", name)
            wrote = True
        if wrote:
            UsdGeom.Subset.SetFamilyType(mesh, family, UsdGeom.Tokens.unrestricted)


def write_stage(path, outputs: Dict[str, Geometry], graph: Optional[dict] = None, overwrite: bool = False,
                frame: Optional[float] = None) -> WriteReport:
    """Write ``outputs`` (name -> geometry) to the USD file ``path`` (.usd/.usda/.usdc)."""
    Gf, Sdf, Tf, Usd, UsdGeom, Vt = pxr()
    path = os.fspath(path)
    if not path.lower().endswith(USD_EXTENSIONS):
        raise ValueError("a USD file must end in .usd, .usda or .usdc")
    if os.path.exists(path) and not overwrite:
        raise FileExistsError(f"{os.path.basename(path)} already exists")
    os.makedirs(os.path.dirname(os.path.abspath(path)), exist_ok=True)
    tmp = path + ".part" + os.path.splitext(path)[1]                   # write beside the target, then swap in: no half files
    if os.path.exists(tmp):
        os.remove(tmp)
    try:
        stage = Usd.Stage.CreateNew(tmp)
        UsdGeom.SetStageUpAxis(stage, UsdGeom.Tokens.z)
        UsdGeom.SetStageMetersPerUnit(stage, 1.0)
        root = UsdGeom.Xform.Define(stage, f"/{ROOT}")
        stage.SetDefaultPrim(root.GetPrim())
        report = WriteReport(path)
        used: set = set()
        for name, geo in outputs.items():
            base = valid_name(name)
            nm = _unique_name(base, used, "out")
            report.outputs[name] = dict(write_geometry(stage, f"/{ROOT}/{nm}", geo), prim=f"/{ROOT}/{nm}")
        layer = stage.GetRootLayer()
        data = {"minihou": {"format": 1, "outputs": json.dumps(list(outputs))}}      # no lists: the text format cannot hold them
        if graph is not None:
            data["minihou"]["graph"] = json.dumps(graph, sort_keys=True)
        if frame is not None:
            data["minihou"]["frame"] = float(frame)
        layer.customLayerData = data
        layer.documentation = "Exported from MiniHou"
        stage.GetRootLayer().Save()
        del stage
        os.replace(tmp, path)
    finally:
        if os.path.exists(tmp):
            os.remove(tmp)
    return report


# -- reading -------------------------------------------------------------------------------------------------------------
@dataclass
class Part:
    P: np.ndarray
    point_attrs: Dict[str, np.ndarray] = field(default_factory=dict)
    point_index: Optional[np.ndarray] = None
    vp: np.ndarray = field(default_factory=lambda: np.zeros(0, np.int64))        # corner -> local point
    sizes: np.ndarray = field(default_factory=lambda: np.zeros(0, np.int64))
    closed: bool = True
    corner_attrs: Dict[str, np.ndarray] = field(default_factory=dict)
    prim_attrs: Dict[str, np.ndarray] = field(default_factory=dict)
    prim_index: Optional[np.ndarray] = None


def _read_primvars(prim, tm, kind: str, npoints: int, ncorner: int, nprim: int, n_expand: Optional[np.ndarray] = None):
    """(point attrs, corner attrs, prim attrs, point_index, prim_index) of a USD prim."""
    Gf, Sdf, Tf, Usd, UsdGeom, Vt = pxr()
    pa: Dict[str, np.ndarray] = {}
    ca: Dict[str, np.ndarray] = {}
    qa: Dict[str, np.ndarray] = {}
    point_index = prim_index = None
    consts: Dict[str, np.ndarray] = {}

    def put(name: str, arr: np.ndarray, interp: str, dom: str = ""):
        # MiniHou files say which table an attribute came from (on curves point and corner attributes are both "vertex")
        if dom == "point" or (not dom and interp in ("vertex", "varying")):
            pa[name] = arr
        elif dom == "vertex" or (not dom and interp == "faceVarying"):
            ca[name] = arr
        elif dom == "prim" or (not dom and interp == "uniform"):
            qa[name] = arr
        elif interp == "constant":
            consts[name] = arr
    api = UsdGeom.PrimvarsAPI(prim)
    for pv in api.GetPrimvars():
        raw = pv.GetPrimvarName()
        if raw.startswith(INTERNAL):
            val = pv.ComputeFlattened(tm)
            if val is None:
                continue
            if raw == INTERNAL + "point_index":
                point_index = np.asarray(val, np.int64)
            elif raw == INTERNAL + "prim_index":
                prim_index = np.asarray(val, np.int64)
            continue
        val = pv.ComputeFlattened(tm)
        if val is None:
            continue
        orig = pv.GetAttr().GetCustomDataByKey("minihou:name") or ""
        arr = np.asarray(val)
        es = pv.GetElementSize()
        if es > 1 and arr.ndim == 1:
            arr = arr.reshape(-1, es)
        tn = str(pv.GetTypeName())
        name = orig
        if not name:
            name = {"displayColor": "Cd", "displayOpacity": "Alpha"}.get(raw, "uv" if raw == "st" or tn.startswith("texCoord2") else raw)
        interp = pv.GetInterpolation()
        if tn.startswith("double"):
            arr = arr.astype(np.float32)
        elif tn.startswith(("int64", "uint")):
            arr = arr.astype(np.int32)
        put(name, arr, interp, pv.GetAttr().GetCustomDataByKey("minihou:domain") or "")
    if prim.IsA(UsdGeom.PointBased):
        pb = UsdGeom.PointBased(prim)
        na = pb.GetNormalsAttr()
        if na and na.HasAuthoredValue():
            nv = na.Get(tm)
            if nv is not None and len(nv):
                put("N", np.asarray(nv, np.float32), pb.GetNormalsInterpolation(), na.GetCustomDataByKey("minihou:domain") or "")
    return pa, ca, qa, point_index, prim_index, consts


@dataclass
class ReadReport:
    prims: int = 0
    skipped: List[str] = field(default_factory=list)
    notes: List[str] = field(default_factory=list)
    up_axis: str = "Z"
    meters_per_unit: float = 1.0
    graph: Optional[str] = None
    detail: dict = field(default_factory=dict)

    def to_dict(self) -> dict:
        return {"prims": self.prims, "skipped": self.skipped, "notes": self.notes, "up_axis": self.up_axis,
                "meters_per_unit": self.meters_per_unit, "has_graph": self.graph is not None}


def read_graph(path) -> Optional[dict]:
    """The MiniHou graph stored in a USD file written with one, else None."""
    Gf, Sdf, Tf, Usd, UsdGeom, Vt = pxr()
    layer = Sdf.Layer.FindOrOpen(os.fspath(path))
    if layer is None:
        raise ValueError(f"cannot open {os.fspath(path)} as USD")
    g = (layer.customLayerData or {}).get("minihou", {}).get("graph")
    return json.loads(g) if g else None


def read_geometry(path, prim_path: str = "", time: Optional[float] = None, up_axis: str = "z", apply_units: bool = True,
                  purposes=("default", "render")) -> Tuple[Geometry, ReadReport]:
    """Read the meshes, curves and points under ``prim_path`` (default: the whole stage) into one Geometry in world space."""
    Gf, Sdf, Tf, Usd, UsdGeom, Vt = pxr()
    path = os.fspath(path)
    try:
        stage = Usd.Stage.Open(path)
    except Exception as e:  # noqa: BLE001 - pxr raises its own types
        raise ValueError(f"cannot open {os.path.basename(path)} as USD: {e}") from None
    if stage is None:
        raise ValueError(f"cannot open {os.path.basename(path)} as USD")
    tm = Usd.TimeCode.Default() if time is None else Usd.TimeCode(float(time))
    mpu = float(UsdGeom.GetStageMetersPerUnit(stage) or 1.0) if stage.HasAuthoredMetadata("metersPerUnit") else 1.0   # USD's own fallback is cm; unauthored: take it as metres
    rep = ReadReport(up_axis=str(UsdGeom.GetStageUpAxis(stage)), meters_per_unit=mpu)
    meta = (stage.GetRootLayer().customLayerData or {}).get("minihou", {})
    rep.graph = meta.get("graph")
    start = stage.GetPrimAtPath(prim_path) if prim_path else stage.GetPseudoRoot()
    if not start or not start.IsValid():
        raise ValueError(f"no prim {prim_path!r} in {os.path.basename(path)}")
    xcache = UsdGeom.XformCache(tm)
    M_up = np.eye(3)
    if up_axis == "z" and rep.up_axis == "Y":
        M_up = np.array([[1.0, 0, 0], [0, 0, 1.0], [0, -1.0, 0]])         # row-vector form of (x, y, z) -> (x, -z, y)
        rep.notes.append("Y-up stage turned to Z-up")
    unit = rep.meters_per_unit if apply_units and rep.meters_per_unit not in (0.0, 1.0) else 1.0
    if unit != 1.0:
        rep.notes.append(f"scaled by metersPerUnit {unit:g}")
    parts: List[Part] = []
    for prim in Usd.PrimRange(start, Usd.TraverseInstanceProxies(Usd.PrimDefaultPredicate)):
        if not prim.IsA(UsdGeom.Imageable):
            continue
        purpose = UsdGeom.Imageable(prim).ComputePurpose()
        if purpose not in purposes:
            if prim.IsA(UsdGeom.Gprim):
                rep.skipped.append(f"{prim.GetPath()} ({purpose} purpose)")
            continue
        kind = None
        if prim.IsA(UsdGeom.Mesh):
            kind = "mesh"
        elif prim.IsA(UsdGeom.BasisCurves):
            kind = "curves"
        elif prim.IsA(UsdGeom.Points):
            kind = "points"
        elif prim.IsA(UsdGeom.Gprim) or prim.IsA(UsdGeom.PointInstancer):
            rep.skipped.append(f"{prim.GetPath()} ({prim.GetTypeName()})")
            continue
        else:
            continue
        pb = UsdGeom.PointBased(prim)
        pts = pb.GetPointsAttr().Get(tm)
        if pts is None or not len(pts):
            continue
        P = np.asarray(pts, np.float64).reshape(-1, 3)
        M = np.array(xcache.GetLocalToWorldTransform(prim), np.float64).reshape(4, 4)
        flip = bool(np.linalg.det(M[:3, :3]) < 0)
        if prim.IsA(UsdGeom.Mesh):
            mesh = UsdGeom.Mesh(prim)
            counts = np.asarray(mesh.GetFaceVertexCountsAttr().Get(tm), np.int64)
            idx = np.asarray(mesh.GetFaceVertexIndicesAttr().Get(tm), np.int64)
            if mesh.GetOrientationAttr().Get(tm) == "leftHanded":
                flip = not flip
            part = Part(P=P, vp=idx, sizes=counts, closed=True)
            np_, nc_, nq_ = len(P), len(idx), len(counts)
        elif prim.IsA(UsdGeom.BasisCurves):
            cur = UsdGeom.BasisCurves(prim)
            counts = np.asarray(cur.GetCurveVertexCountsAttr().Get(tm), np.int64)
            if str(cur.GetTypeAttr().Get(tm)) != "linear":
                rep.notes.append(f"{prim.GetPath()}: cubic curve read as its control polygon")
            vcount = int(counts.sum())
            if vcount > len(P):
                rep.notes.append(f"{prim.GetPath()}: curve counts exceed its points; skipped")
                continue
            part = Part(P=P, vp=np.arange(vcount, dtype=np.int64), sizes=counts, closed=False)
            flip = False
            np_, nc_, nq_ = len(P), vcount, len(counts)
        else:
            part = Part(P=P)
            flip = False
            np_, nc_, nq_ = len(P), 0, 0
        pa, ca, qa, pidx, qidx, consts = _read_primvars(prim, tm, kind, np_, nc_, nq_)
        if kind == "points":
            part.vp, part.sizes = np.zeros(0, np.int64), np.zeros(0, np.int64)
        for k_, v_ in consts.items():                                    # a constant primvar is the same on every primitive
            if nq_ and k_ not in qa:
                qa[k_] = np.repeat(v_[:1], nq_, axis=0) if v_.ndim > 1 else np.repeat(v_[:1], nq_)
        part.point_attrs = {k: v for k, v in pa.items() if len(v) == np_}
        part.corner_attrs = {k: v for k, v in ca.items() if len(v) == nc_}
        part.prim_attrs = {k: v for k, v in qa.items() if len(v) == nq_}
        part.point_index = pidx if pidx is not None and len(pidx) == np_ else None
        part.prim_index = qidx if qidx is not None and len(qidx) == nq_ else None
        for dropped in set(pa) | set(ca) | set(qa):
            if dropped not in part.point_attrs and dropped not in part.corner_attrs and dropped not in part.prim_attrs:
                rep.notes.append(f"{prim.GetPath()}: primvar {dropped!r} has a size that fits no domain; skipped")
        if kind == "mesh":
            _add_subset_groups(UsdGeom.Mesh(prim), part, tm)
        # world space
        Pw = P @ (M[:3, :3] @ M_up) + (M[3, :3] @ M_up)
        part.P = (Pw * unit).astype(np.float32)
        nrm_m = np.linalg.inv(M[:3, :3]).T
        for table in (part.point_attrs, part.corner_attrs, part.prim_attrs):
            nv = table.get("N")
            if nv is not None and nv.ndim == 2 and nv.shape[1] == 3:
                v = nv.astype(np.float64) @ nrm_m @ M_up
                v /= np.maximum(np.linalg.norm(v, axis=1, keepdims=True), 1e-12)
                table["N"] = v.astype(np.float32)
        if flip and kind == "mesh":                                      # reverse each face's winding (first corner stays)
            part = _reverse_winding(part)
        parts.append(part)
        rep.prims += 1
    geo = assemble(parts)
    detail = _read_detail(stage)
    if detail:
        geo.detail.update(detail)
    return geo, rep


def _read_detail(stage) -> dict:
    """Detail attributes of a MiniHou export (all outputs merged; the first output wins on a clash)."""
    out: dict = {}
    root = stage.GetPrimAtPath(f"/{ROOT}")
    if not root or not root.IsValid():
        return out
    for child in root.GetChildren():
        raw = child.GetCustomDataByKey("minihou:detail")
        if raw:
            try:
                for k, v in json.loads(raw).items():
                    out.setdefault(k, v)
            except ValueError:
                pass
    return out


def _add_subset_groups(mesh, part: Part, tm) -> None:
    """Face / point ``GeomSubset``s as groups, when the file has no ``group_*`` primvar of that name already."""
    Gf, Sdf, Tf, Usd, UsdGeom, Vt = pxr()
    nq, npnt = len(part.sizes), len(part.P)
    for sub in UsdGeom.Subset.GetAllGeomSubsets(mesh):
        ind = sub.GetIndicesAttr().Get(tm)
        if ind is None:
            continue
        ind = np.asarray(ind, np.int64)
        orig = sub.GetPrim().GetCustomDataByKey("minihou:group")
        name = orig or "group_" + sub.GetPrim().GetName()
        if not name.startswith("group_"):
            name = "group_" + name
        element = str(sub.GetElementTypeAttr().Get())
        if element == "face":
            if name in part.prim_attrs or name in part.point_attrs:
                continue
            a = np.zeros(nq, np.int32)
            a[ind[(ind >= 0) & (ind < nq)]] = 1
            part.prim_attrs[name] = a
        elif element == "point":
            if name in part.point_attrs or name in part.prim_attrs:
                continue
            a = np.zeros(npnt, np.int32)
            a[ind[(ind >= 0) & (ind < npnt)]] = 1
            part.point_attrs[name] = a


def _reverse_winding(part: Part) -> Part:
    """Each face's corners in reverse order (keeping the first corner first); corner attributes follow."""
    s = part.sizes
    offs = np.concatenate([[0], np.cumsum(s)])
    first = np.repeat(offs[:-1], s)
    k = np.arange(int(s.sum()), dtype=np.int64) - np.repeat(offs[:-1], s)
    src = first + np.where(k == 0, 0, np.repeat(s, s) - k)
    part.vp = part.vp[src]
    part.corner_attrs = {n: a[src] for n, a in part.corner_attrs.items()}
    return part


def _zeros_like(a: np.ndarray, n: int) -> np.ndarray:
    return np.zeros((n,) + a.shape[1:], a.dtype)


def _union(per_part: List[Dict[str, np.ndarray]], sizes: List[int]) -> Dict[str, np.ndarray]:
    """Concatenate attribute dictionaries part by part, zero-filling where a part lacks an attribute."""
    names: List[str] = []
    for d in per_part:
        for k in d:
            if k not in names:
                names.append(k)
    out = {}
    for k in names:
        ref = next(d[k] for d in per_part if k in d)
        out[k] = np.concatenate([d[k] if k in d and d[k].shape[1:] == ref.shape[1:] else _zeros_like(ref, n)
                                 for d, n in zip(per_part, sizes)]) if len(per_part) else ref
    return out


def assemble(parts: List[Part]) -> Geometry:
    """One Geometry from the parts. MiniHou files (every part has ``point_index``) get their point table and primitive order back."""
    if not parts:
        return Geometry.from_arrays(np.zeros((0, 3), np.float32))
    exact = all(p.point_index is not None for p in parts)
    sizes_pts = [len(p.P) for p in parts]
    if exact:
        idx = [p.point_index for p in parts]
        N = int(max((i.max() for i in idx if len(i)), default=-1)) + 1
        mapping = idx
        P = np.zeros((N, 3), np.float32)
        have = np.zeros(N, bool)
        names: List[str] = []
        for p in parts:
            for k in p.point_attrs:
                if k not in names:
                    names.append(k)
        pa = {}
        for k in names:
            ref = next(p.point_attrs[k] for p in parts if k in p.point_attrs)
            pa[k] = _zeros_like(ref, N)
        for p, ix in zip(parts, idx):
            new = ~have[ix]
            # a point seen twice inside one part (a curve crossing itself) is the same point: the first row wins
            first_in_part = np.zeros(len(ix), bool)
            _, first = np.unique(ix, return_index=True)
            first_in_part[first] = True
            take = new & first_in_part
            P[ix[take]] = p.P[take]
            for k, v in p.point_attrs.items():
                if v.shape[1:] == pa[k].shape[1:]:
                    pa[k][ix[take]] = v[take]
            have[ix[take]] = True
    else:
        offs = np.concatenate([[0], np.cumsum(sizes_pts)])
        N = int(offs[-1])
        mapping = [np.arange(n, dtype=np.int64) + o for n, o in zip(sizes_pts, offs[:-1])]
        P = np.concatenate([p.P for p in parts]).astype(np.float32)
        pa = _union([p.point_attrs for p in parts], sizes_pts)
    # primitives
    vp = [m[p.vp] if len(p.vp) else np.zeros(0, np.int64) for p, m in zip(parts, mapping)]
    sizes = [p.sizes for p in parts]
    closed = [np.full(len(p.sizes), p.closed, bool) for p in parts]
    nprim = [len(p.sizes) for p in parts]
    ncorner = [len(p.vp) for p in parts]
    vp_all = np.concatenate(vp) if vp else np.zeros(0, np.int64)
    sizes_all = np.concatenate(sizes) if sizes else np.zeros(0, np.int64)
    closed_all = np.concatenate(closed) if closed else np.zeros(0, bool)
    ca = _union([p.corner_attrs for p in parts], ncorner)
    qa = _union([p.prim_attrs for p in parts], nprim)
    if all(p.prim_index is not None for p in parts if len(p.sizes)) and sum(nprim):
        order = np.argsort(np.concatenate([p.prim_index for p in parts if len(p.sizes)]), kind="stable")
        offs_c = np.concatenate([[0], np.cumsum(sizes_all)])
        corner_order = np.concatenate([np.arange(offs_c[i], offs_c[i + 1]) for i in order]) if len(order) < 200000 else \
            _corner_order(sizes_all, order, offs_c)
        vp_all, sizes_all, closed_all = vp_all[corner_order], sizes_all[order], closed_all[order]
        ca = {k: v[corner_order] for k, v in ca.items()}
        qa = {k: v[order] for k, v in qa.items()}
    prim_offsets = np.concatenate([[0], np.cumsum(sizes_all)]).astype(np.int32)
    geo = Geometry.from_arrays(P, vp_all.astype(np.int32), prim_offsets, closed_all if len(closed_all) else None)
    for k, v in pa.items():
        if k != "P":
            geo.point[k] = v
    for k, v in ca.items():
        geo.vertex[k] = v
    for k, v in qa.items():
        geo.prim[k] = v
    geo.validate()
    return geo.freeze()


def _corner_order(sizes: np.ndarray, order: np.ndarray, offs: np.ndarray) -> np.ndarray:
    s = sizes[order]
    first = np.repeat(offs[order], s)
    k = np.arange(int(s.sum()), dtype=np.int64) - np.repeat(np.cumsum(s) - s, s)
    return first + k


def file_stamp(path) -> str:
    """Changes whenever the file on disk does (used in cache keys)."""
    try:
        st = os.stat(path)
        return f"{st.st_mtime_ns}:{st.st_size}"
    except OSError:
        return "missing"
