"""Build clean-commit combined addon/source ZIPs and a recoverable local Git bundle."""
import hashlib
import json
from pathlib import Path
import subprocess
import zipfile

ROOT = Path(__file__).resolve().parents[1]
DIST = ROOT / "dist"


def git(*args):
    return subprocess.check_output(["git", *args], cwd=ROOT).decode("utf-8").strip()


def main():
    if git("status", "--porcelain"):
        raise RuntimeError("Commit the reviewed source before packaging")
    DIST.mkdir(exist_ok=True)
    files = git("ls-files").splitlines()
    addon = DIST / "MiniHou-Link-integrated.zip"
    source = DIST / "MiniHou-integrated-source.zip"
    bundle = DIST / "MiniHou-integrated.bundle"
    with zipfile.ZipFile(addon, "w", zipfile.ZIP_DEFLATED) as archive:
        for name in files:
            if name.startswith("blender_addon/"):
                archive.write(ROOT / name, name.removeprefix("blender_addon/"))
    with zipfile.ZipFile(source, "w", zipfile.ZIP_DEFLATED) as archive:
        for name in files:
            archive.write(ROOT / name, "MiniHou/" + name)
        demo = DIST / "MiniHou-Wired-Groups.blend"
        if demo.exists():
            archive.write(demo, "MiniHou/examples/MiniHou-Wired-Groups.blend")
    branch = git("branch", "--show-current")
    refs = ["HEAD", branch] if branch else ["HEAD"]
    subprocess.run(["git", "bundle", "create", str(bundle), *refs], cwd=ROOT, check=True)
    subprocess.run(["git", "bundle", "verify", str(bundle)], cwd=ROOT, check=True)
    for path in (addon, source):
        with zipfile.ZipFile(path) as archive:
            bad = archive.testzip()
            if bad:
                raise RuntimeError("Corrupt archive member: " + bad)
    artifacts = {p.name: {"bytes": p.stat().st_size, "sha256": hashlib.sha256(p.read_bytes()).hexdigest()}
                 for p in (addon, source, bundle)}
    result = {"commit": git("rev-parse", "HEAD"), "branch": git("branch", "--show-current"), "artifacts": artifacts}
    (DIST / "integration-artifacts.json").write_text(json.dumps(result, indent=2), encoding="utf-8")
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
