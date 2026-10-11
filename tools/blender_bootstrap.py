"""Runs *inside* Blender (``blender --python tools/blender_bootstrap.py -- --port 8765``).

Registers the addon straight from the repo (no install step) and connects to the engine.
"""
import argparse
import os
import sys

import bpy

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
ADDON_PARENT = os.path.join(ROOT, "blender_addon")
if ADDON_PARENT not in sys.path:
    sys.path.insert(0, ADDON_PARENT)


def main():
    argv = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else []
    ap = argparse.ArgumentParser()
    ap.add_argument("--host", default="127.0.0.1")
    ap.add_argument("--port", type=int, default=8765)
    ap.add_argument("--no-connect", action="store_true")
    args = ap.parse_args(argv)

    import importlib
    import minihou_link
    try:
        minihou_link.unregister()               # reloading the script in a running session
    except Exception:
        pass
    importlib.reload(minihou_link)
    minihou_link.register()
    wm = bpy.context.window_manager
    wm.minihou_host, wm.minihou_port = args.host, args.port
    print(f"[minihou] addon registered from {ADDON_PARENT}", flush=True)
    if not args.no_connect:
        from minihou_link import runtime

        def connect_later():
            runtime.get_runtime().connect(args.host, args.port)
            return None                          # run once

        bpy.app.timers.register(connect_later, first_interval=0.5)


main()
