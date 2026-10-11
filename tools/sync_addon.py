"""Copy the files the Blender addon shares with the engine into the addon folder.

``minihou/link/protocol.py`` and ``minihou/link/client.py`` are the single source of truth; the addon
carries verbatim copies (a Blender addon must be self-contained, and symlinks are unreliable on
Windows). ``tests/test_addon_sync.py`` fails if they drift. Run: ``python tools/sync_addon.py``
"""
import shutil
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SHARED = ["protocol.py", "client.py"]
SRC = ROOT / "minihou" / "link"
DST = ROOT / "blender_addon" / "minihou_link"


def main() -> int:
    for name in SHARED:
        shutil.copyfile(SRC / name, DST / name)
        print(f"copied {name}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
