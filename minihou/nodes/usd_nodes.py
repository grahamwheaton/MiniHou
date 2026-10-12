"""USD files as geometry: the meshes, curves and points of a ``.usd`` / ``.usda`` / ``.usdc`` / ``.usdz`` file (see :mod:`minihou.usd_io`)."""
import json
import os

from .. import timectx, usd_io
from ..sdk import Bool, Enum, Str, node

READ_EXTENSIONS = usd_io.USD_EXTENSIONS + (".usdz",)


def resolve(file: str) -> str:
    """The file to read: relative paths are relative to the folder the engine runs in (its workspace root)."""
    return os.path.abspath(os.path.expanduser(file)) if file else ""


def _key(params) -> str:
    """Changes when the file on disk changes (its modification time and size), and with the frame for animated files."""
    path = resolve(params["file"])
    stamp = usd_io.file_stamp(path) if path else ""
    return stamp + (f":{timectx.frame():g}" if params["time"] == "frame" else "")


@node("usd_import", category="source", key_extra=_key, params=[
    Str("file", "", doc="USD file (.usd, .usda, .usdc, .usdz); a relative path starts in the folder the engine runs in"),
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
