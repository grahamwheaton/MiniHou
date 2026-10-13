"""Where a node's relative file path starts: the workspace root.

The editor engine has one workspace root (the folder it was started for; graphs, the disk cache and imported files live there).
A node that reads or writes a file (``usd_import``, ``usd_export``, ``image_file``, ``image_out``) resolves a relative path against it,
so a graph file means the same thing whichever folder the process was started from. Without an editor (the library, the tests) the
root is the current folder, as before.
"""
from __future__ import annotations

import os
from pathlib import Path
from typing import Optional

_ROOT: Optional[Path] = None


def set_root(path) -> None:
    """Make ``path`` the folder relative file parameters start in (``None``: the current folder again)."""
    global _ROOT
    _ROOT = Path(path).resolve() if path else None


def root() -> Path:
    return _ROOT if _ROOT is not None else Path(os.getcwd()).resolve()


def resolve(file) -> str:
    """The absolute path a file parameter means ('' stays '')."""
    f = str(file or "").strip()
    if not f:
        return ""
    p = Path(os.path.expanduser(f))
    return os.path.normpath(str(p if p.is_absolute() else root() / p))


def to_param(path) -> str:
    """What to store in a file parameter for ``path``: relative to the root (with ``/``) when it is inside it, else the absolute path."""
    p = Path(os.path.abspath(os.path.expanduser(str(path))))
    try:
        return p.resolve().relative_to(root()).as_posix()
    except (ValueError, OSError):
        return str(p)
