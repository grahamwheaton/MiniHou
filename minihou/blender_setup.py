"""Finding Blender, putting the add-on into it, and starting it connected to this engine.

Used by the editor's "Blender" panel and by the packed app, so a user never has to touch ``Preferences > Add-ons``:

* :func:`list_blenders` -- installed Blender executables (newest first).
* :func:`build_addon_zip` -- the add-on as a zip (``minihou_link/`` at its top level) for dragging into Blender.
* :func:`install_addon` -- runs the chosen Blender in the background once, unpacks the add-on into its user add-ons folder,
  enables it and saves the preferences.
* :func:`launch_blender` -- opens Blender with the add-on registered straight from a copy and connected to this engine.

Only executables found by :func:`list_blenders` (or named by ``$BLENDER_EXE``) are ever run: the web API passes an index, not a path.
"""
from __future__ import annotations

import glob
import io
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import zipfile
from pathlib import Path
from typing import Dict, Iterator, List, Optional

ADDON = "minihou_link"
MIN_BLENDER = (4, 2)


# ---------------------------------------------------------------------------------------------- finding Blender
def candidate_paths(platform=None, env=None, which=shutil.which) -> Iterator[str]:
    """Blender executables to try, best first. ``platform``/``env`` are parameters so this is testable anywhere."""
    platform = platform or ("windows" if os.name == "nt" else "mac" if sys.platform == "darwin" else "linux")
    env = os.environ if env is None else env
    if env.get("BLENDER_EXE"):
        yield env["BLENDER_EXE"]
    for name in ("blender", "blender.exe"):
        p = which(name)
        if p:
            yield p
    pats = []
    if platform == "windows":
        for base in (env.get("ProgramFiles"), env.get("ProgramW6432"), env.get("LOCALAPPDATA")):
            if base:
                pats.append(os.path.join(base, "Blender Foundation", "Blender *", "blender.exe"))
                pats.append(os.path.join(base, "Programs", "Blender Foundation", "Blender *", "blender.exe"))
        for base in (env.get("ProgramFiles(x86)"), env.get("ProgramFiles")):
            if base:
                pats.append(os.path.join(base, "Steam", "steamapps", "common", "Blender", "blender.exe"))
    elif platform == "mac":
        pats.append("/Applications/Blender*.app/Contents/MacOS/Blender")
    else:
        pats += ["/usr/bin/blender", "/snap/bin/blender", os.path.expanduser("~/blender*/blender"),
                 "/opt/blender*/blender"]
    found = []
    for pat in pats:
        found += glob.glob(pat)

    def version_key(path):
        m = re.search(r"Blender[ -]?(\d+)\.(\d+)", path.replace("Blender Foundation", ""))
        return (int(m.group(1)), int(m.group(2))) if m else (0, 0)

    seen = set()
    for p in sorted(found, key=version_key, reverse=True):      # newest first
        if p not in seen:
            seen.add(p)
            yield p


def find_blender(explicit=None) -> Optional[str]:
    for p in ([explicit] if explicit else list(candidate_paths())):
        if p and os.path.isfile(p):
            return p
    return None


def blender_version(exe) -> Optional[tuple]:
    """Ask the executable (slow: starts Blender). :func:`path_version` guesses from the path without running anything."""
    try:
        out = subprocess.run([exe, "--background", "--version"], capture_output=True, text=True, timeout=60,
                             encoding="utf-8", errors="replace").stdout
        m = re.search(r"Blender (\d+)\.(\d+)(?:\.(\d+))?", out)
        return tuple(int(x) for x in m.groups() if x) if m else None
    except (OSError, subprocess.SubprocessError):
        return None


def path_version(path: str) -> Optional[tuple]:
    m = re.search(r"Blender[ -]?(\d+)\.(\d+)", path.replace("Blender Foundation", ""))
    return (int(m.group(1)), int(m.group(2))) if m else None


def list_blenders(platform=None, env=None, which=shutil.which) -> List[Dict]:
    """``[{"path", "version"}]`` for every installed Blender found (version from the path, None if unknown)."""
    out, seen = [], set()
    for p in candidate_paths(platform, env, which):
        if p in seen or not os.path.isfile(p):
            continue
        seen.add(p)
        v = path_version(p)
        out.append({"path": p, "version": ".".join(map(str, v)) if v else None,
                    "supported": v is None or v >= MIN_BLENDER})
    return out


# ---------------------------------------------------------------------------------------------- the add-on as files
def addon_source() -> Optional[Path]:
    """The ``minihou_link`` folder: next to the package in a checkout, or inside a frozen app. ``$MINIHOU_ADDON_DIR`` overrides."""
    cands: List[Path] = []
    if os.environ.get("MINIHOU_ADDON_DIR"):
        cands.append(Path(os.environ["MINIHOU_ADDON_DIR"]))
    if getattr(sys, "frozen", False):
        base = Path(getattr(sys, "_MEIPASS", None) or Path(sys.executable).parent)
        cands += [base / "blender_addon" / ADDON, Path(sys.executable).parent / "blender_addon" / ADDON]
    here = Path(__file__).resolve().parent
    cands += [here.parent / "blender_addon" / ADDON, here / "_addon" / ADDON]
    for c in cands:
        if (c / "__init__.py").is_file():
            return c
    return None


def addon_files(src: Path) -> List[Path]:
    return sorted(p for p in src.rglob("*") if p.is_file() and "__pycache__" not in p.parts and p.suffix != ".pyc")


def build_addon_zip() -> bytes:
    """The add-on as a zip with ``minihou_link/`` at the top level (Blender's Install from Disk and drag-and-drop layout)."""
    src = addon_source()
    if src is None:
        raise FileNotFoundError("the Blender add-on files are not part of this installation")
    buf = io.BytesIO()
    with zipfile.ZipFile(buf, "w", zipfile.ZIP_DEFLATED) as z:
        for p in addon_files(src):
            info = zipfile.ZipInfo(f"{ADDON}/{p.relative_to(src).as_posix()}", date_time=(2024, 1, 1, 0, 0, 0))
            info.compress_type = zipfile.ZIP_DEFLATED
            info.external_attr = 0o644 << 16
            z.writestr(info, p.read_bytes())
    return buf.getvalue()


def data_dir() -> Path:
    """Where the app keeps what it writes for Blender (``$MINIHOU_HOME`` or ``~/.minihou``)."""
    d = Path(os.environ.get("MINIHOU_HOME") or Path.home() / ".minihou") / "blender"
    d.mkdir(parents=True, exist_ok=True)
    return d


# ---------------------------------------------------------------------------------------------- running Blender
INSTALL_SCRIPT = '''
import json, os, shutil, sys, zipfile
import bpy
argv = sys.argv[sys.argv.index("--") + 1:]
zip_path, out_path = argv[0], argv[1]
res = {"ok": False, "blender": bpy.app.version_string}
try:
    base = bpy.utils.user_resource("SCRIPTS", path="addons", create=True)
    target = os.path.join(base, "%(addon)s")
    if os.path.isdir(target):
        shutil.rmtree(target)
    with zipfile.ZipFile(zip_path) as z:
        z.extractall(base)
    import addon_utils
    bpy.utils.refresh_script_paths()          # a folder created after startup is not on sys.path yet
    addon_utils.modules_refresh()
    mod = addon_utils.enable("%(addon)s", default_set=True, persistent=True)
    bpy.ops.wm.save_userpref()
    res.update(ok=mod is not None, path=target)
    if mod is None:
        res["error"] = "unpacked, but Blender did not enable the add-on (see its console)"
except Exception as e:
    res["error"] = repr(e)
with open(out_path, "w") as f:
    json.dump(res, f)
''' % {"addon": ADDON}

BOOTSTRAP = '''
import argparse, importlib, os, sys
import bpy
here = os.path.dirname(os.path.abspath(__file__))
if here not in sys.path:
    sys.path.insert(0, here)
argv = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else []
ap = argparse.ArgumentParser()
ap.add_argument("--host", default="127.0.0.1")
ap.add_argument("--port", type=int, default=8765)
args = ap.parse_args(argv)
import %(addon)s
try:
    %(addon)s.unregister()
except Exception:
    pass
importlib.reload(%(addon)s)
%(addon)s.register()
wm = bpy.context.window_manager
wm.minihou_host, wm.minihou_port = args.host, args.port
def connect_later():
    from %(addon)s import runtime
    runtime.get_runtime().connect(args.host, args.port)
    return None
bpy.app.timers.register(connect_later, first_interval=0.5)
print("[minihou] add-on registered and connecting to %%s:%%d" %% (args.host, args.port), flush=True)
''' % {"addon": ADDON}


def _clean_env() -> dict:
    env = dict(os.environ)
    if getattr(sys, "frozen", False):                  # a frozen app's own Python must not leak into Blender's
        for k in ("PYTHONHOME", "PYTHONPATH", "PYTHONSAFEPATH", "LD_LIBRARY_PATH", "LD_LIBRARY_PATH_ORIG"):
            env.pop(k, None)
    return env


def install_addon(exe: str, *, timeout: float = 180.0) -> dict:
    """Install and enable the add-on in the user's Blender (``exe``); never raises. ``{"ok", "message", ...}``."""
    try:
        data = build_addon_zip()
    except FileNotFoundError as e:
        return {"ok": False, "message": str(e)}
    with tempfile.TemporaryDirectory(prefix="minihou_install_") as tmp:
        zpath, script, out = (os.path.join(tmp, n) for n in ("minihou_link.zip", "install.py", "result.json"))
        Path(zpath).write_bytes(data)
        Path(script).write_text(INSTALL_SCRIPT, encoding="utf-8")
        try:
            proc = subprocess.run([exe, "--background", "--python", script, "--", zpath, out], capture_output=True, text=True,
                                  timeout=timeout, encoding="utf-8", errors="replace", env=_clean_env())
        except subprocess.TimeoutExpired:
            return {"ok": False, "message": f"Blender did not finish within {int(timeout)} s"}
        except OSError as e:
            return {"ok": False, "message": f"could not start Blender: {e}"}
        try:
            res = json.loads(Path(out).read_text(encoding="utf-8"))
        except (OSError, ValueError):
            tail = "\n".join((proc.stderr or proc.stdout or "").strip().splitlines()[-6:])
            return {"ok": False, "message": "Blender ran but did not report a result" + (f":\n{tail}" if tail else "")}
    if res.get("ok"):
        return {"ok": True, "blender": res.get("blender"), "path": res.get("path"),
                "message": f"Installed and enabled in Blender {res.get('blender', '')}. If Blender is already open, restart it "
                           "(or enable MiniHou Link in Preferences > Add-ons)."}
    return {"ok": False, "message": res.get("error") or "installation failed", "blender": res.get("blender")}


def prepare_launch_dir() -> Path:
    """A fresh copy of the add-on plus a bootstrap script, so Blender can run it without installing anything."""
    d = data_dir()
    src = addon_source()
    if src is None:
        raise FileNotFoundError("the Blender add-on files are not part of this installation")
    target = d / ADDON
    if target.exists():
        shutil.rmtree(target, ignore_errors=True)
    shutil.copytree(src, target, ignore=shutil.ignore_patterns("__pycache__", "*.pyc"))
    (d / "bootstrap.py").write_text(BOOTSTRAP, encoding="utf-8")
    return d


def launch_command(exe: str, port: int, host: str = "127.0.0.1") -> List[str]:
    d = prepare_launch_dir()
    return [exe, "--python", str(d / "bootstrap.py"), "--", "--host", host, "--port", str(port)]


def launch_blender(exe: str, port: int, host: str = "127.0.0.1") -> subprocess.Popen:
    """Start Blender (own window, outlives the engine's request) with the add-on registered and connecting to ``port``."""
    cmd = launch_command(exe, port, host)
    kw: dict = {"env": _clean_env(), "stdin": subprocess.DEVNULL}
    if os.name == "nt":
        kw["creationflags"] = subprocess.CREATE_NEW_PROCESS_GROUP | getattr(subprocess, "DETACHED_PROCESS", 0)
    else:
        kw["start_new_session"] = True
    return subprocess.Popen(cmd, **kw)
