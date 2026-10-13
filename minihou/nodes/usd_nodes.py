"""USD files as geometry: the meshes, curves and points of a ``.usd`` / ``.usda`` / ``.usdc`` / ``.usdz`` file (see :mod:`minihou.usd_io`)."""
import json
import os

from .. import paths, timectx, usd_io
from ..actions import action
from ..sdk import Action, Bool, Enum, Float, Int, Str, node

READ_EXTENSIONS = usd_io.USD_EXTENSIONS + (".usdz",)


def resolve(file: str) -> str:
    """The file to read: relative paths are relative to the workspace root (see :mod:`minihou.paths`)."""
    return paths.resolve(file)


def _key(params) -> str:
    """Changes when the file on disk changes (its modification time and size), and with the frame for animated files."""
    path = resolve(params["file"])
    stamp = usd_io.file_stamp(path) if path else ""
    return stamp + (f":{_file_time(timectx.frame(), params):g}" if params["time"] == "frame" else "")


def _file_time(frame: float, params) -> float:
    """The time in the file for ``frame``: (frame - time_offset) / time_scale (offset 10: the file's first frame shows at frame 10)."""
    return (float(frame) - float(params["time_offset"])) / float(params["time_scale"])


@node("usd_import", category="source", key_extra=_key, actions=[
    Action("reload", "Reload", doc="Read the file again now (a changed file is noticed by itself; this is for when it cannot be)")], params=[
    Str("file", "", kind="file:open:.usd,.usda,.usdc,.usdz", doc="USD file (.usd, .usda, .usdc, .usdz); a relative path starts in the workspace folder"),
    Enum("missing_file", ["error", "empty"], doc="error: a missing or unset file fails the node; empty: it gives empty geometry (a graph that opens "
                                                "before its files are in place)"),
    Float("time_offset", 0.0, soft_min=-100.0, soft_max=100.0, doc="Frames the file's animation is delayed by (file time = (frame - offset) / scale)"),
    Float("time_scale", 1.0, min=0.0001, soft_max=10.0, doc="Speed of the file's animation: 2 plays it half as fast"),
    Enum("time", ["default", "frame"], doc="frame: read the file's animation at the current frame (default: the un-animated values)"),
    Str("prim_path", "", label="Primitives", doc="Only read these prims and what is below them: a prim path, or several patterns separated by spaces, with * and ? "
                                                 "wildcards and a leading ^ to exclude, e.g. `/World/Props /World/Set/chair* ^/World/Props/lights` "
                                                 "(empty: the whole file)"),
    Str("purpose", "default render", doc="Which purposes to read: default, render, proxy, guide (several, separated by spaces) or all"),
    Str("path_attrib", "", label="Create Path Attribute", doc="Name of an integer primitive attribute telling which USD prim each primitive came from "
                                                             "(an index into the detail attribute `usd_paths`); empty: none"),
    Enum("up_axis", ["z", "keep"], doc="z: turn a Y-up file to Z-up like MiniHou and Blender; keep: leave the coordinates as they are"),
    Bool("apply_units", True, doc="Scale by the file's metersPerUnit (a centimetre file becomes metres); off keeps the file's numbers"),
    Int("reload", 0, ui="hidden", doc="Counts the Reload button, so a reload is a different cook"),
])
def usd_import(file, missing_file, time_offset, time_scale, time, prim_path, purpose, path_attrib, up_axis, apply_units, reload):
    """Meshes, curves and points of a USD file, in world space, merged into one geometry: the file's own transforms are applied,
    primvars become attributes (`st` as `uv`, `displayColor` as `Cd`, normals as `N`, others by name), GeomSubsets become groups, and a
    file exported by MiniHou comes back with its point order, groups and detail attributes. Other prim types (cubes, spheres, ...) are
    skipped and listed in the detail attribute `usd_skipped`. Needs the usd-core package (pip install usd-core).

    Settings follow Houdini's USD Import where MiniHou has the same idea: file (with a picker), Reload, time offset and scale, what to do
    with a missing file, a Primitives pattern, purposes and a path attribute. Variants, traversal, display as, pivot and unpack have no
    counterpart: MiniHou reads the geometry, not the stage."""
    path = resolve(file) if file else ""
    if not path or not os.path.isfile(path):
        if missing_file == "empty":
            return usd_io.assemble([])
        if not file:
            raise ValueError("set 'file' to a USD file")
        if not path.lower().endswith(READ_EXTENSIONS):
            raise ValueError(f"{os.path.basename(path)}: not a USD file (expected {', '.join(READ_EXTENSIONS)})")
        raise ValueError(f"no such file: {path}")
    if not path.lower().endswith(READ_EXTENSIONS):
        raise ValueError(f"{os.path.basename(path)}: not a USD file (expected {', '.join(READ_EXTENSIONS)})")
    if path_attrib and not usd_io.valid_name(path_attrib) == path_attrib:
        raise ValueError(f"path_attrib {path_attrib!r} is not a valid attribute name (letters, digits and _)")
    geo, rep = usd_io.read_geometry(path, prim_path=prim_path.strip(), up_axis=up_axis, apply_units=apply_units,
                                    time=_file_time(timectx.frame(), dict(time_offset=time_offset, time_scale=time_scale)) if time == "frame" else None,
                                    purposes=usd_io.parse_purposes(purpose), prim_attrib=path_attrib)
    out = geo.copy()
    out.detail["usd_prims"] = rep.prims
    if rep.skipped:
        out.detail["usd_skipped"] = "; ".join(rep.skipped[:20]) + (f"; and {len(rep.skipped) - 20} more" if len(rep.skipped) > 20 else "")
    if rep.notes:
        out.detail["usd_notes"] = "; ".join(dict.fromkeys(rep.notes))[:500]
    return out


@action("usd_import", "reload")
def _reload(session, node_id, p):
    session.edit([{"op": "set", "id": node_id, "params": {"reload": int(p["reload"]) + 1}}])
    return {"ok": True, "message": "Reloaded " + (p["file"] or "the node")}


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
    Enum("frames", ["current", "range"], doc="current: the frame the timeline is on; range: an animation, cooked at every frame from start to end "
                                             "(what changes between frames becomes time samples)"),
    Int("start", 1, soft_min=0, soft_max=240, doc="range: first frame"),
    Int("end", 24, soft_min=0, soft_max=240, doc="range: last frame"),
    Int("step", 1, min=1, soft_max=10, doc="range: every this many frames"),
    Enum("up_axis", ["z", "y"], doc="z: as MiniHou and Blender have it; y: the file says Y up and its root turns the geometry, so the numbers stay"),
    Enum("units", list(usd_io.UNITS), doc="The unit the file declares. The geometry is scaled to match, so a metre stays a metre "
                                         "(centimeters multiplies the coordinates by 100)"),
    Bool("materials", True, doc="Write the materials the faces use (Material prims bound to the mesh); off writes geometry only"),
    Bool("embed_graph", True, doc="Keep the graph inside the file, so MiniHou can open the recipe back from the result"),
    Bool("overwrite", True, doc="Replace the file when it exists (off: Save to Disk refuses)"),
])
def usd_export(geo, file, format, name, root, frames, start, end, step, up_axis, units, materials, embed_graph, overwrite):
    """Passes its input on unchanged; its **Save to Disk** button writes the cooked geometry to a USD file (always at full size, whatever the
    proxy level). Settings: format (binary or text), the root prim, the current frame or a frame range (an animation), up axis, units,
    whether materials and the graph are written. Houdini,
    Blender, Maya and usdview open the result; a file written here reads back through `usd_import` with its points, groups and details."""
    return geo


MAX_FRAMES = 20000


def export_frames(params):
    """The frames a ``usd_export`` node writes: ``None`` for the current one, else start, start + step, ... up to end."""
    if params["frames"] != "range":
        return None
    start, end, step = int(params["start"]), int(params["end"]), int(params["step"])
    if end < start:
        raise ValueError(f"the frame range {start}..{end} is empty: 'end' must not be below 'start'")
    n = (end - start) // step + 1
    if n > MAX_FRAMES:
        raise ValueError(f"{n} frames is more than the {MAX_FRAMES} one export may write; raise 'step' or shorten the range")
    return [float(start + i * step) for i in range(n)]


@action("usd_export", "save")
def _save(session, node_id, p):
    path = export_target(p)
    frames = export_frames(p)
    opts = {"up_axis": p["up_axis"], "units": p["units"], "root": p["root"] or "MiniHou", "materials": p["materials"]}
    try:
        rep = session.export_usd(path, node=node_id, overwrite=p["overwrite"], embed_graph=p["embed_graph"], name=p["name"] or node_id,
                                 options=opts, frames=frames)
    except FileExistsError:
        raise FileExistsError(f"{os.path.basename(path)} already exists; turn on 'overwrite' to replace it") from None
    o = list(rep["outputs"].values())
    n = lambda k: sum(x.get(k, 0) for x in o)      # noqa: E731
    parts = [f"{n('faces'):,} faces", f"{n('curves'):,} curves", f"{n('points'):,} loose points"] + ([f"{n('instances'):,} instances"] if n("instances") else [])
    where = paths.to_param(path)
    when = f" over {len(frames)} frames ({frames[0]:g}-{frames[-1]:g})" if frames else ""
    return {"ok": True, "path": where, "report": rep, "message": f"Saved {', '.join(parts)}{when} to {where}"}
