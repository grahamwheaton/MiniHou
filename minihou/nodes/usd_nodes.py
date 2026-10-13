"""USD files as geometry: the meshes, curves and points of a ``.usd`` / ``.usda`` / ``.usdc`` / ``.usdz`` file (see :mod:`minihou.usd_io`)."""
import json
import os

from .. import paths, timectx, usd_io
from ..actions import action
from ..sdk import Action, Bool, Enum, Str, node

READ_EXTENSIONS = usd_io.USD_EXTENSIONS + (".usdz",)


def resolve(file: str) -> str:
    """The file to read: relative paths are relative to the workspace root (see :mod:`minihou.paths`)."""
    return paths.resolve(file)


def _key(params) -> str:
    """Changes when the file on disk changes (its modification time and size), and with the frame for animated files."""
    path = resolve(params["file"])
    stamp = usd_io.file_stamp(path) if path else ""
    return stamp + (f":{timectx.frame():g}" if params["time"] == "frame" else "")


@node("usd_import", category="source", key_extra=_key, params=[
    Str("file", "", kind="file:open:.usd,.usda,.usdc,.usdz", doc="USD file (.usd, .usda, .usdc, .usdz); a relative path starts in the workspace folder"),
    Str("prim_path", "", doc="Only read below this prim, e.g. /World/Props (empty: the whole file)"),
    Enum("time", ["default", "frame"], doc="frame: read the file's animation at the current frame (default: the un-animated values)"),
    Enum("up_axis", ["z", "keep"], doc="z: turn a Y-up file to Z-up like MiniHou and Blender; keep: leave the coordinates as they are"),
    Bool("apply_units", True, doc="Scale by the file's metersPerUnit (a centimetre file becomes metres); off keeps the file's numbers"),
])
def usd_import(file, prim_path, time, up_axis, apply_units):
    """Meshes, curves and points of a USD file, in world space, merged into one geometry: the file's own transforms are applied,
    primvars become attributes (`st` as `uv`, `displayColor` as `Cd`, normals as `N`, others by name), GeomSubsets become groups, and a
    file exported by MiniHou comes back with its point order, groups and detail attributes. Other prim types (cubes, spheres, point
    instancers, ...) are skipped and listed in the detail attribute `usd_skipped`. Needs the usd-core package (pip install usd-core)."""
    if not file:
        raise ValueError("set 'file' to a USD file")
    path = resolve(file)
    if not path.lower().endswith(READ_EXTENSIONS):
        raise ValueError(f"{os.path.basename(path)}: not a USD file (expected {', '.join(READ_EXTENSIONS)})")
    if not os.path.isfile(path):
        raise ValueError(f"no such file: {path}")
    geo, rep = usd_io.read_geometry(path, prim_path=prim_path, time=timectx.frame() if time == "frame" else None,
                                    up_axis=up_axis, apply_units=apply_units)
    out = geo.copy()
    out.detail["usd_prims"] = rep.prims
    if rep.skipped:
        out.detail["usd_skipped"] = "; ".join(rep.skipped[:20]) + (f"; and {len(rep.skipped) - 20} more" if len(rep.skipped) > 20 else "")
    if rep.notes:
        out.detail["usd_notes"] = "; ".join(dict.fromkeys(rep.notes))[:500]
    return out


# -- export ------------------------------------------------------------------------------------------------------------------
def export_target(params) -> str:
    """The file a ``usd_export`` node writes: its ``file`` resolved against the workspace, with the extension the ``format`` asks for."""
    path = paths.resolve(params["file"])
    if not path:
        raise ValueError("set 'file' to the USD file to write")
    base, ext = os.path.splitext(path)
    fmt = params["format"]
    ext = ext.lower()
    if ext and ext not in usd_io.USD_EXTENSIONS:
        raise ValueError(f"{os.path.basename(path)}: a USD file ends in {', '.join(usd_io.USD_EXTENSIONS)}")
    if not ext or (fmt == "usdc" and ext == ".usda") or (fmt == "usda" and ext != ".usda"):
        path = base + (".usda" if fmt == "usda" else ".usdc")
    return path


@node("usd_export", inputs=["geo"], category="output", actions=[
    Action("save", "Save to Disk", doc="Cook this node at full size and write the USD file now. Cooking itself never writes anything.")], params=[
    Str("file", "exports/scene.usdc", kind="file:save:.usdc,.usda,.usd",
        doc="The USD file to write (.usd, .usda text, .usdc binary); a relative path starts in the workspace folder"),
    Enum("format", ["auto", "usdc", "usda"], doc="auto: by the file's extension (.usd is binary); usdc: binary; usda: readable text. A format that "
                                                 "disagrees with the extension changes the extension"),
    Str("name", "", doc="Name of this geometry's prim under the root (empty: the node's name)"),
    Str("root", "MiniHou", doc="Name of the root prim of the file (its default prim)"),
    Enum("up_axis", ["z", "y"], doc="z: as MiniHou and Blender have it; y: the file says Y up and its root turns the geometry, so the numbers stay"),
    Enum("units", list(usd_io.UNITS), doc="The unit the file declares. The geometry is scaled to match, so a metre stays a metre "
                                         "(centimeters multiplies the coordinates by 100)"),
    Bool("materials", True, doc="Write the materials the faces use (Material prims bound to the mesh); off writes geometry only"),
    Bool("embed_graph", True, doc="Keep the graph inside the file, so MiniHou can open the recipe back from the result"),
    Bool("overwrite", True, doc="Replace the file when it exists (off: Save to Disk refuses)"),
])
def usd_export(geo, file, format, name, root, up_axis, units, materials, embed_graph, overwrite):
    """Passes its input on unchanged; its **Save to Disk** button writes the cooked geometry to a USD file (always at full size, whatever the
    proxy level). Settings: format (binary or text), the root prim, up axis, units, whether materials and the graph are written. Houdini,
    Blender, Maya and usdview open the result; a file written here reads back through `usd_import` with its points, groups and details."""
    return geo


@action("usd_export", "save")
def _save(session, node_id, p):
    path = export_target(p)
    opts = {"up_axis": p["up_axis"], "units": p["units"], "root": p["root"] or "MiniHou", "materials": p["materials"]}
    try:
        rep = session.export_usd(path, node=node_id, overwrite=p["overwrite"], embed_graph=p["embed_graph"], name=p["name"] or node_id, options=opts)
    except FileExistsError:
        raise FileExistsError(f"{os.path.basename(path)} already exists; turn on 'overwrite' to replace it") from None
    o = list(rep["outputs"].values())
    n = lambda k: sum(x.get(k, 0) for x in o)      # noqa: E731
    parts = [f"{n('faces'):,} faces", f"{n('curves'):,} curves", f"{n('points'):,} loose points"] + ([f"{n('instances'):,} instances"] if n("instances") else [])
    return {"ok": True, "path": paths.to_param(path), "report": rep, "message": f"Saved {', '.join(parts)} to {paths.to_param(path)}"}
