"""Entry point of the packed application (PyInstaller builds this file; see tools/build_app.py)."""
import multiprocessing
import sys
import os
from pathlib import Path

if sys.stdout is None or sys.stderr is None:
    logs = Path(os.environ.get("LOCALAPPDATA", str(Path.home()))) / "MiniHou"
    logs.mkdir(parents=True, exist_ok=True)
    stream = (logs / "app.log").open("a", encoding="utf-8", buffering=1)
    sys.stdout = sys.stdout or stream
    sys.stderr = sys.stderr or stream

from minihou.__main__ import main

if __name__ == "__main__":
    multiprocessing.freeze_support()          # numba/threading helpers must not re-run the app in a frozen child process
    sys.exit(main())
