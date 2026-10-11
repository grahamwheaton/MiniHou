"""Run integrated engine/browser, actual Blender and offline RBD acceptance.

Use a prepared external Python containing numpy, optional MuJoCo and Playwright.
No installs are performed. Logs and machine-readable results go to build/validation.
"""
import argparse
import json
import os
from pathlib import Path
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
from tools.launch_blender import find_blender


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--blender")
    args = parser.parse_args()
    blender = find_blender(args.blender)
    if not blender:
        raise RuntimeError("Install/find Blender or pass --blender")
    dest = ROOT / "build/validation"
    dest.mkdir(parents=True, exist_ok=True)
    env = dict(os.environ, MINIHOU_ENGINE_PYTHON=sys.executable)
    browsers = ROOT / "build/runtime/browsers"
    if browsers.is_dir():
        env.setdefault("PLAYWRIGHT_BROWSERS_PATH", str(browsers))
    results = []

    def run(name, command, timeout=600):
        start = time.monotonic()
        print("Running", name, flush=True)
        with (dest / (name + ".log")).open("w", encoding="utf-8") as log:
            process = subprocess.run(command, cwd=ROOT, env=env, stdout=log, stderr=subprocess.STDOUT, timeout=timeout)
        results.append(dict(name=name, exit_code=process.returncode, seconds=round(time.monotonic() - start, 2),
                            log=str(dest / (name + ".log"))))
        (dest / "results.json").write_text(json.dumps(results, indent=2), encoding="utf-8")
        if process.returncode:
            raise RuntimeError(name + " failed; see " + str(dest / (name + ".log")))

    base = [blender, "--background", "--factory-startup", "--python-exit-code", "1"]
    run("engine-browser", [sys.executable, str(ROOT / "tools/test_worktree.py")])
    log = (dest / "engine-browser.log").read_text(encoding="utf-8")
    if "setUpClass (tests.test_editor_e2e.EditorE2E) ... skipped" in log or "playwright not installed" in log:
        raise RuntimeError("Chromium acceptance was skipped; prepare trusted Playwright browsers and rerun")
    run("blender-full", base + ["--python", "tools/blender_selftest.py"])
    run("rbd-wall", base + ["--python", "tools/blender_rbd_smoke.py"])
    run("rbd-falling", base + ["--python", "tools/blender_rbd_falling_smoke.py"])
    rbd = ROOT / "dist/rbd_acceptance"
    run("rbd-offline", base + [str(rbd / "RBD_Brick_Wall_Baked.blend"), "--python", "tools/render_rbd_bake.py", "--",
                              "--start", "11", "--end", "13", "--output", str(rbd / "offline_verified_"),
                              "--reference", str(rbd / "wall_live_0012.png"), "--reference-frame", "12"])
    # This launcher prepends this checkout even for embedded Python distributions.
    from tests.test_blender_addon import Engine
    engine = Engine(ROOT / "examples/modifiers/transform.json")
    try:
        run("geonodes-example", base + ["--python", "tools/blender_geonodes_demo.py", "--",
                                      "--port", str(engine.port), "--saved-port", "8765",
                                      "--save", str(ROOT / "dist/MiniHou-Wired-Groups.blend")])
    finally:
        engine.stop()
    print(json.dumps(results, indent=2), flush=True)


if __name__ == "__main__":
    main()
