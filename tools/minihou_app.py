"""Entry point of the packed application (PyInstaller builds this file; see tools/build_app.py)."""
import multiprocessing
import sys

from minihou.__main__ import main

if __name__ == "__main__":
    multiprocessing.freeze_support()          # numba/threading helpers must not re-run the app in a frozen child process
    sys.exit(main())
