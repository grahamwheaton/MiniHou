"""Import addon submodules that don't need bpy (state.py, protocol.py, client.py) without importing
the addon's __init__ (which imports bpy)."""
import importlib
import sys
import types
from pathlib import Path

ADDON_DIR = Path(__file__).resolve().parent.parent / "blender_addon" / "minihou_link"
PURE_PKG = "minihou_link_pure"


def pure_module(name: str):
    if PURE_PKG not in sys.modules:
        pkg = types.ModuleType(PURE_PKG)
        pkg.__path__ = [str(ADDON_DIR)]
        sys.modules[PURE_PKG] = pkg
    return importlib.import_module(f"{PURE_PKG}.{name}")
