"""Choosing files for the editor: a folder listing for the in-app browser and the operating system's own open / save dialog.

The editor is a web page, and a page never learns the real path of a file it is given, so a file parameter asks the engine
(which runs on the same machine) to do the choosing. In order of preference:

1. the desktop window's native dialog (``WebServer.pick_file``, set by :mod:`minihou.desktop`),
2. the system dialog of Python's ``tkinter`` in a separate process (so a dialog never blocks the engine),
3. the editor's own browser (:func:`list_dir`), which works everywhere, also on a machine without a screen.
"""
from __future__ import annotations

import json
import os
import re
import string
import subprocess
import sys
import threading
from pathlib import Path
from typing import Dict, List, Optional, Sequence

MAX_ENTRIES = 5000
_EXT = re.compile(r"^\.[A-Za-z0-9]{1,8}$")
_BUSY = threading.Lock()                     # one system dialog at a time


def clean_exts(exts) -> List[str]:
    """The extensions asked for (``.usd``...), anything else dropped."""
    if isinstance(exts, str):
        exts = exts.split(",")
    out: List[str] = []
    for e in exts or []:
        e = str(e).strip().lower()
        if _EXT.match(e) and e not in out:
            out.append(e)
    return out[:12]


def list_dir(path, exts: Sequence[str] = (), root=None, hidden: bool = False, places: bool = False, fallback: bool = False) -> Dict:
    """The folders and (matching) files of ``path`` for the editor's file browser. Names and sizes only, never contents.
    ``path`` may be a file (its folder is listed) or empty (``root``). ``fallback``: a folder that does not exist lists its nearest
    existing parent instead (the file parameter's old value may point at something that has gone)."""
    base = Path(os.path.expanduser(str(path))) if path else Path(root or os.getcwd())
    try:
        base = base.resolve()
        if fallback:
            while not base.exists() and base.parent != base:
                base = base.parent
        if base.is_file():
            base = base.parent
        if not base.is_dir():
            raise FileNotFoundError(str(base))
        want = tuple(clean_exts(exts))
        entries: List[dict] = []
        total = 0
        with os.scandir(base) as it:
            for e in it:
                name = e.name
                if not hidden and name.startswith("."):
                    continue
                try:
                    is_dir = e.is_dir()
                    if not is_dir and want and not name.lower().endswith(want):
                        continue
                    st = e.stat()
                except OSError:
                    continue                                   # a broken link, a file in use, no rights
                total += 1
                if len(entries) < MAX_ENTRIES:
                    entries.append({"name": name, "dir": bool(is_dir), "size": 0 if is_dir else int(st.st_size), "mtime": int(st.st_mtime)})
    except (OSError, ValueError) as e:
        raise FileNotFoundError(f"cannot list {path}: {e}") from None
    entries.sort(key=lambda x: (not x["dir"], x["name"].lower()))
    out = {"path": str(base), "parent": str(base.parent) if base.parent != base else None, "entries": entries,
           "truncated": total > len(entries), "sep": os.sep}
    if places:
        out["places"] = _places(root)
    return out


def _places(root) -> List[dict]:
    home = Path.home()
    found = [{"name": "Workspace", "path": str(root or os.getcwd())}, {"name": "Home", "path": str(home)}]
    for sub in ("Desktop", "Documents", "Downloads"):
        p = home / sub
        if p.is_dir():
            found.append({"name": sub, "path": str(p)})
    if os.name == "nt":
        for c in string.ascii_uppercase:
            d = f"{c}:\\"
            try:
                if os.path.isdir(d):
                    found.append({"name": d, "path": d})
            except OSError:
                pass
    else:
        found.append({"name": "/", "path": "/"})
    return found


_TK_SCRIPT = r"""
import json, sys
import tkinter as tk
from tkinter import filedialog
mode, title, start, exts = sys.argv[1:5]
exts = [e for e in exts.split(",") if e]
root = tk.Tk(); root.withdraw()
try: root.attributes("-topmost", True)
except Exception: pass
kinds = [("Supported files", " ".join("*" + e for e in exts))] if exts else []
kinds.append(("All files", "*.*"))
import os
folder, name = None, ""
if start and os.path.isdir(start):
    folder = start
elif start:
    folder, name = os.path.dirname(start) or None, os.path.basename(start)
kw = dict(title=title, filetypes=kinds)
if folder and os.path.isdir(folder): kw["initialdir"] = folder
if mode == "save":
    if name: kw["initialfile"] = name
    if exts: kw["defaultextension"] = exts[0]
    path = filedialog.asksaveasfilename(**kw)
else:
    path = filedialog.askopenfilename(**kw)
print(json.dumps({"path": path or ""}))
"""


def system_dialog_available() -> bool:
    if os.environ.get("MINIHOU_NATIVE_DIALOG", "1") == "0":        # tests and servers without a person at the screen
        return False
    if getattr(sys, "frozen", False):            # a bundled app: sys.executable is the app itself, not a Python
        return False
    try:
        import importlib.util
        return importlib.util.find_spec("tkinter") is not None and (os.name == "nt" or sys.platform == "darwin" or bool(os.environ.get("DISPLAY") or os.environ.get("WAYLAND_DISPLAY")))
    except (ImportError, ValueError):
        return False


def system_dialog(mode: str, exts: Sequence[str] = (), title: str = "", start: str = "") -> Optional[dict]:
    """The operating system's open / save dialog (tkinter, in a child process). ``{"path": ...}`` (empty when cancelled), or ``None``
    when there is no such dialog on this machine."""
    if not system_dialog_available():
        return None
    if not _BUSY.acquire(blocking=False):
        return {"path": "", "busy": True}
    try:
        r = subprocess.run([sys.executable, "-c", _TK_SCRIPT, "save" if mode == "save" else "open", title or "MiniHou", start or "", ",".join(clean_exts(exts))],
                           capture_output=True, text=True, encoding="utf-8", timeout=900)
        if r.returncode != 0:
            return None
        line = (r.stdout or "").strip().splitlines()[-1:] or [""]
        return {"path": str(json.loads(line[0]).get("path") or "")}
    except (OSError, subprocess.SubprocessError, ValueError):
        return None
    finally:
        _BUSY.release()
