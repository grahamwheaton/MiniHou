"""Build the packed MiniHou application with PyInstaller (one folder, zipped), then smoke-test it.

    py -m pip install pyinstaller numpy
    py tools/build_app.py                  # -> dist/MiniHou/ and dist/MiniHou-<os>.zip, numpy-only (small, portable)
    py tools/build_app.py --with-accel     # also bundle numba and scipy (several times larger; faster heavy nodes)
    py tools/build_app.py --no-smoke       # skip starting the built program and fetching its pages

The program contains the engine, the editor, the example graphs and the Blender add-on (the editor's top bar installs it into
Blender or offers it as a zip to drag in). Build on each operating system you want to ship for: PyInstaller does not cross-compile.
"""
import argparse
import json
import os
import platform
import shutil
import subprocess
import sys
import tempfile
import time
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
DIST = ROOT / "dist"
NAME = "MiniHou"


def build(with_accel: bool) -> Path:
    sep = os.pathsep
    cmd = [sys.executable, "-m", "PyInstaller", "--noconfirm", "--clean", "--onedir", "--windowed", "--name", NAME,
           "--distpath", str(DIST), "--workpath", str(ROOT / "build" / "pyinstaller"), "--specpath", str(ROOT / "build"),
           "--paths", str(ROOT),
           "--collect-submodules", "minihou",
           "--collect-all", "webview",
           "--icon", str(ROOT / "minihou/assets/minihou.ico"),
           "--add-data", f"{ROOT / 'minihou/assets'}{sep}minihou/assets",
           "--add-data", f"{ROOT / 'minihou' / 'web' / 'static'}{sep}minihou/web/static",
           "--add-data", f"{ROOT / 'blender_addon' / 'minihou_link'}{sep}blender_addon/minihou_link",
           "--add-data", f"{ROOT / 'examples' / 'graphs'}{sep}examples/graphs"]
    for mod in ("bpy", "tkinter", "matplotlib", "PIL", "IPython", "pytest", "playwright"):
        cmd += ["--exclude-module", mod]
    if with_accel:
        for pkg in ("numba", "llvmlite", "scipy"):
            cmd += ["--collect-all", pkg]
    else:
        for mod in ("numba", "llvmlite", "scipy"):
            cmd += ["--exclude-module", mod]
    cmd.append(str(ROOT / "tools" / "minihou_app.py"))
    print(" ".join(cmd), flush=True)
    subprocess.run(cmd, check=True, cwd=ROOT)
    return DIST / NAME


def exe_path(folder: Path) -> Path:
    return folder / (NAME + (".exe" if os.name == "nt" else ""))


def smoke(folder: Path, timeout: float = 90.0) -> None:
    """Start the built program in a scratch workspace and check the editor, the add-on zip and the Blender list answer."""
    tmp = Path(tempfile.mkdtemp(prefix="minihou_smoke_"))
    env = dict(os.environ, MINIHOU_HOME=str(tmp / "home"), PYTHONUNBUFFERED="1", LOCALAPPDATA=str(tmp / "local"))
    log = tmp / "out.log"
    app_log = tmp / "local" / "MiniHou" / "app.log"
    def output():
        return log.read_text(encoding="utf-8", errors="replace") + (app_log.read_text(encoding="utf-8", errors="replace") if app_log.exists() else "")
    with open(log, "w", encoding="utf-8") as lf:
        proc = subprocess.Popen([str(exe_path(folder)), "app", "--no-browser", "--workspace", str(tmp / "ws"), "--http-port", "0", "--port", "0"],
                                stdout=lf, stderr=subprocess.STDOUT, env=env)
    try:
        url = None
        end = time.monotonic() + timeout
        while time.monotonic() < end and not url:
            if proc.poll() is not None:
                raise RuntimeError("the program exited early:\n" + output())
            for line in output().splitlines():
                if line.startswith("editor: "):
                    url = line.split("editor: ", 1)[1].strip()
            time.sleep(0.3)
        if not url:
            raise RuntimeError("no editor URL after %d s:\n%s" % (timeout, output()))
        get = lambda p: urllib.request.urlopen(url.rstrip("/") + p, timeout=30).read()
        state = json.loads(get("/api/state"))
        assert state["nodes"], "no node types"
        assert b"MiniHou" in get("/"), "editor page missing"
        zdata = get("/addon/minihou_link.zip")
        assert zdata[:2] == b"PK" and len(zdata) > 10_000, "add-on zip missing"
        info = json.loads(get("/api/blender"))
        assert info["addon"]["available"], "add-on files are not inside the app"
        assert (tmp / "ws" / "examples").is_dir(), "examples were not copied to the workspace"
        print(f"smoke test OK: {len(state['nodes'])} node types, add-on zip {len(zdata)} bytes, graph {state.get('path')}")
    finally:
        proc.terminate()
        try:
            proc.wait(10)
        except subprocess.TimeoutExpired:
            proc.kill()
        shutil.rmtree(tmp, ignore_errors=True)


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--with-accel", action="store_true", help="bundle numba and scipy too")
    ap.add_argument("--no-smoke", action="store_true")
    args = ap.parse_args()
    folder = build(args.with_accel)
    if not args.no_smoke:
        smoke(folder)
    system = {"Windows": "windows", "Darwin": "macos", "Linux": "linux"}.get(platform.system(), platform.system().lower())
    archive = shutil.make_archive(str(DIST / f"{NAME}-{system}" ), "zip", DIST, NAME)
    print(f"built {folder}\narchive {archive} ({os.path.getsize(archive) / 1e6:.1f} MB)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
