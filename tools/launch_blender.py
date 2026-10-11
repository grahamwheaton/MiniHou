"""Find Blender and launch it with the MiniHou addon (optionally starting the engine too).

    python tools/launch_blender.py --find                       # just print the Blender path
    python tools/launch_blender.py --engine examples/graphs/demo.json
    python tools/launch_blender.py --selftest                   # headless addon tests, exit code = result
    python tools/launch_blender.py --bench -- --points 1000000  # headless benchmark
    python tools/launch_blender.py --gui-test                   # interactive smoke test in a real Blender window

Blender is located via --blender, $BLENDER_EXE, PATH, then the usual install locations (Windows: Program
Files, Steam, per-user installs). Arguments go to subprocess as a list, so spaces in paths are fine.
"""
import argparse
import glob
import json
import os
import re
import shutil
import subprocess
import shutil as _shutil
import socket
import sys
import tempfile
import time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


def candidate_paths(platform=None, env=None, which=shutil.which):
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


def find_blender(explicit=None):
    for p in ([explicit] if explicit else list(candidate_paths())):
        if p and os.path.isfile(p):
            return p
    return None


def blender_version(exe):
    try:
        out = subprocess.run([exe, "--background", "--version"], capture_output=True, text=True, timeout=60,
                             encoding="utf-8", errors="replace").stdout
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
    ap.add_argument("--http-port", type=int, default=8766, help="engine's web editor / API port (-1 = off)")
    ap.add_argument("--editor", action="store_true", help="also open the node editor in a web browser")
    ap.add_argument("--selftest", action="store_true", help="run the addon tests headless; exit code = result")
    ap.add_argument("--bench", action="store_true", help="run bench/bench_link.py headless (args after --)")
    ap.add_argument("--gui-test", action="store_true",
                    help="start the engine and a real Blender window, run tools/gui_smoketest.py, report and exit")
    ap.add_argument("--keep-open", action="store_true", help="with --gui-test: leave Blender open afterwards")
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

    if args.gui_test:
        return gui_test(exe, env, args.keep_open, extra=passthrough)

    engine = None
    running = _running_engine(args.http_port)
    if args.engine and running:
        # e.g. started by an AI assistant through MCP: reuse it instead of failing on a busy port
        print(f"an engine is already running ({running.get('path')}); opening {args.engine} in it")
        try:
            running = _engine_client(args.http_port).post("/api/open", {"path": os.path.abspath(args.engine)})
        except Exception as e:  # noqa: BLE001 - still launch Blender against the running engine
            print(f"  could not open it there: {e}", file=sys.stderr)
    elif args.engine:
        graph = os.path.abspath(args.engine)
        root = ROOT if os.path.commonpath([graph, ROOT]) == ROOT else os.path.dirname(graph)   # editor's file menu
        engine = subprocess.Popen([sys.executable, "-u", "-m", "minihou", "serve", graph, "--host", args.host,
                                   "--port", str(args.port), "--http-port", str(args.http_port), "--root", root], cwd=ROOT)
        time.sleep(1.0)
        if engine.poll() is not None:
            print("engine exited immediately; see its message above", file=sys.stderr)
            return engine.returncode or 1
    if (args.engine or running) and args.http_port >= 0:
        url = f"http://127.0.0.1:{args.http_port}/"
        print(f"node editor: {url}")
        if args.editor:
            import webbrowser
            webbrowser.open(url)
    cmd = [exe] + ([args.blend] if args.blend else []) + ["--python", os.path.join(ROOT, "tools", "blender_bootstrap.py"),
                                                          "--", "--host", args.host, "--port", str(args.port)]
    print("launching:", " ".join(f'"{c}"' if " " in c else c for c in cmd))
    try:
        return subprocess.call(cmd, cwd=ROOT, env=env)
    finally:
        if engine and engine.poll() is None:
            engine.terminate()


def _engine_client(http_port):
    sys.path.insert(0, ROOT)
    from minihou.mcp import EngineClient
    return EngineClient(f"http://127.0.0.1:{http_port}")


def _running_engine(http_port):
    """State of a MiniHou engine already serving on ``http_port``, or None."""
    if http_port < 0:
        return None
    try:
        return _engine_client(http_port).get("/api/graph", timeout=2.0)
    except Exception:  # noqa: BLE001 - nothing there (or not ours)
        return None


def _free_port():
    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    port = s.getsockname()[1]
    s.close()
    return port


def _start_engine(graph, port, env):
    p = subprocess.Popen([sys.executable, "-u", "-m", "minihou", "serve", graph, "--port", str(port), "--http-port", "-1"], cwd=ROOT, env=env,
                         stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, encoding="utf-8", errors="replace")
    seen = []
    for _ in range(50):
        line = p.stdout.readline()
        if not line:
            break
        seen.append(line)
        if f":{port}" in line:
            return p, seen
    p.kill()
    raise RuntimeError("engine did not start:\n" + "".join(seen))


def gui_test(exe, env, keep_open=False, timeout=600, extra=()):
    """Engine + interactive Blender + scripted scenario. Returns 0 if every check passed."""
    work = tempfile.mkdtemp(prefix="minihou_gui_")
    graph = os.path.join(work, "demo.json")
    _shutil.copyfile(os.path.join(ROOT, "examples", "graphs", "demo.json"), graph)   # the test edits this copy
    out = os.path.join(work, "results.json")
    shots = os.path.join(ROOT, "gui_test_output")
    blender_log = os.path.join(work, "blender_console.txt")
    port = _free_port()
    procs = []
    try:
        engine, engine_lines = _start_engine(graph, port, env)
        procs.append(engine)
        print("engine:", engine_lines[-1].strip())
        if sys.platform.startswith("linux") and not env.get("DISPLAY") and not env.get("WAYLAND_DISPLAY"):
            xvfb = _shutil.which("Xvfb")
            if not xvfb:
                print("no display and no Xvfb: cannot open a Blender window", file=sys.stderr)
                return 2
            env["DISPLAY"] = ":97"
            env["MINIHOU_X11_SHOT"] = "1"
            procs.append(subprocess.Popen([xvfb, ":97", "-screen", "0", "1600x900x24", "-nolisten", "tcp"],
                                          stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL))
            time.sleep(1.5)
            print("started Xvfb on :97")
        cmd = [exe, "--factory-startup", "--window-geometry", "0", "0", "1600", "900",
               "--python", os.path.join(ROOT, "tools", "gui_smoketest.py"), "--",
               "--port", str(port), "--graph", graph, "--out", out, "--shots", shots] + (["--keep-open"] if keep_open else []) + list(extra)
        print("launching Blender (window will close by itself when the test ends)...", flush=True)
        with open(blender_log, "w", encoding="utf-8", errors="replace") as logf:
            crash_file = os.path.join(tempfile.gettempdir(), "blender.crash.txt")
            started = time.time()
            b = subprocess.Popen(cmd, cwd=ROOT, env=env, stdout=logf, stderr=subprocess.STDOUT)
            crashed = False
            while b.poll() is None:
                # On Windows a crash leaves the process alive behind an error-report dialog: detect Blender's own crash
                # report and stop at once instead of waiting for the timeout.
                if os.path.exists(crash_file) and os.path.getmtime(crash_file) >= started - 1:
                    time.sleep(1.0)
                    b.kill()
                    crashed = True
                    break
                if not keep_open and time.time() - started > timeout:
                    b.kill()
                    print(f"Blender did not finish within {timeout}s (killed)")
                    break
                time.sleep(0.5)
            if crashed:
                with open(crash_file, encoding="utf-8", errors="replace") as cf:
                    report = cf.read()
                print("BLENDER CRASHED. Crash report:\n" + report[:3000])
                _shutil.copyfile(crash_file, os.path.join(work, "blender.crash.txt"))
        with open(blender_log, encoding="utf-8", errors="replace") as f:
            console = f.read()
        os.makedirs(shots, exist_ok=True)
        _shutil.copyfile(blender_log, os.path.join(shots, "blender_console.txt"))
        if os.path.exists(os.path.join(work, "blender.crash.txt")):
            _shutil.copyfile(os.path.join(work, "blender.crash.txt"), os.path.join(shots, "blender.crash.txt"))
        problems = [l for l in console.splitlines() if "Traceback" in l or l.startswith(("Error", "ERROR")) or "Exception" in l]
        if not os.path.exists(out):
            print("no results written; Blender console follows:\n" + console[-4000:])
            return 1
        with open(out, encoding="utf-8") as f:
            res = json.load(f)
        print(f"\nBlender {res['blender']}: {len(res['checks'])} checks in {res.get('seconds')} s")
        for c in res["checks"]:
            print(f"  {'PASS' if c['ok'] else 'FAIL'}  {c['name']}" + (f"  [{c['detail']}]" if c["detail"] else ""))
        print(f"  panel drawn {res['panel_draws']} times by Blender's UI")
        if res.get("stats"):
            print("  stats:", json.dumps(res["stats"]))
        if problems:
            print("\nconsole lines that look like errors:")
            for l in problems[:20]:
                print("  " + l)
        print("\nscreenshots:", ", ".join(res["screenshots"]) or "none")
        print("full Blender console:", blender_log)
        return 0 if res["ok"] and not problems else 1
    finally:
        for p in reversed(procs):
            if p.poll() is None:
                p.terminate()
                try:
                    p.wait(5)
                except subprocess.TimeoutExpired:
                    p.kill()


if __name__ == "__main__":
    sys.exit(main())
