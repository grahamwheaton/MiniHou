"""Find Blender and launch it with the MiniHou addon (optionally starting the engine too).

    python tools/launch_blender.py --find                       # just print the Blender path
    python tools/launch_blender.py --engine examples/graphs/demo.json
    python tools/launch_blender.py --selftest                   # headless addon tests, exit code = result
    python tools/launch_blender.py --bench -- --points 1000000  # headless benchmark

Blender is located via --blender, $BLENDER_EXE, PATH, then the usual install locations (Windows: Program
Files, Steam, per-user installs). Arguments go to subprocess as a list, so spaces in paths are fine.
"""
import argparse
import glob
import os
import re
import shutil
import subprocess
import sys
import time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


def candidate_paths():
    env = os.environ
    if env.get("BLENDER_EXE"):
        yield env["BLENDER_EXE"]
    for name in ("blender", "blender.exe"):
        p = shutil.which(name)
        if p:
            yield p
    pats = []
    if os.name == "nt":
        for base in (env.get("ProgramFiles"), env.get("ProgramW6432"), env.get("LOCALAPPDATA")):
            if base:
                pats.append(os.path.join(base, "Blender Foundation", "Blender *", "blender.exe"))
                pats.append(os.path.join(base, "Programs", "Blender Foundation", "Blender *", "blender.exe"))
        for base in (env.get("ProgramFiles(x86)"), env.get("ProgramFiles")):
            if base:
                pats.append(os.path.join(base, "Steam", "steamapps", "common", "Blender", "blender.exe"))
    elif sys.platform == "darwin":
        pats.append("/Applications/Blender*.app/Contents/MacOS/Blender")
    else:
        pats += ["/usr/bin/blender", "/snap/bin/blender", os.path.expanduser("~/blender*/blender"),
                 "/opt/blender*/blender"]
    found = []
    for pat in pats:
        found += glob.glob(pat)

    def version_key(path):
        m = re.search(r"(\d+)\.(\d+)", path.replace("Blender Foundation", ""))
        return (int(m.group(1)), int(m.group(2))) if m else (0, 0)

    for p in sorted(found, key=version_key, reverse=True):      # newest first
        yield p


def find_blender(explicit=None):
    for p in ([explicit] if explicit else list(candidate_paths())):
        if p and os.path.isfile(p):
            return p
    return None


def blender_version(exe):
    try:
        out = subprocess.run([exe, "--background", "--version"], capture_output=True, text=True, timeout=60).stdout
        m = re.search(r"Blender (\d+)\.(\d+)(?:\.(\d+))?", out)
        return tuple(int(x) for x in m.groups() if x) if m else None
    except (OSError, subprocess.SubprocessError):
        return None


def main():
    argv = sys.argv[1:]
    passthrough = []
    if "--" in argv:
        i = argv.index("--")
        argv, passthrough = argv[:i], argv[i + 1:]
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("blend", nargs="?", help="optional .blend file to open")
    ap.add_argument("--blender", help="path to blender executable (default: auto-detect)")
    ap.add_argument("--find", action="store_true", help="print the Blender path and exit")
    ap.add_argument("--engine", metavar="GRAPH", help="also start `python -m minihou serve GRAPH`")
    ap.add_argument("--host", default="127.0.0.1")
    ap.add_argument("--port", type=int, default=8765)
    ap.add_argument("--selftest", action="store_true", help="run the addon tests headless; exit code = result")
    ap.add_argument("--bench", action="store_true", help="run bench/bench_link.py headless (args after --)")
    args = ap.parse_args(argv)

    exe = find_blender(args.blender)
    if not exe:
        print("Could not find Blender. Pass --blender PATH or set BLENDER_EXE, e.g.\n"
              '  set BLENDER_EXE=C:\\Program Files\\Blender Foundation\\Blender 5.1\\blender.exe', file=sys.stderr)
        return 2
    print(f"Blender: {exe}")
    if args.find:
        return 0
    ver = blender_version(exe)
    if ver:
        print(f"version: {'.'.join(map(str, ver))}" + ("" if ver >= (4, 2) else "   WARNING: addon is written for Blender 4.2+ (tested on 5.1)"))

    env = dict(os.environ)
    env.setdefault("MINIHOU_ENGINE_PYTHON", sys.executable)       # the engine runs under *this* python
    if args.selftest:
        cmd = [exe, "--background", "--factory-startup", "--python", os.path.join(ROOT, "tools", "blender_selftest.py")]
        return subprocess.call(cmd, cwd=ROOT, env=env)
    if args.bench:
        cmd = [exe, "--background", "--factory-startup", "--python", os.path.join(ROOT, "bench", "bench_link.py"), "--"] + passthrough
        return subprocess.call(cmd, cwd=ROOT, env=env)

    engine = None
    if args.engine:
        engine = subprocess.Popen([sys.executable, "-u", "-m", "minihou", "serve", args.engine, "--host", args.host,
                                   "--port", str(args.port)], cwd=ROOT)
        time.sleep(1.0)
        if engine.poll() is not None:
            print("engine exited immediately; see its message above", file=sys.stderr)
            return engine.returncode or 1
    cmd = [exe] + ([args.blend] if args.blend else []) + ["--python", os.path.join(ROOT, "tools", "blender_bootstrap.py"),
                                                          "--", "--host", args.host, "--port", str(args.port)]
    print("launching:", " ".join(f'"{c}"' if " " in c else c for c in cmd))
    try:
        return subprocess.call(cmd, cwd=ROOT, env=env)
    finally:
        if engine and engine.poll() is None:
            engine.terminate()


if __name__ == "__main__":
    sys.exit(main())
