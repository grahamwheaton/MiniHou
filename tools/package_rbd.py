"""Package this worktree only; includes licenses and a relocatable prepared runtime."""
from pathlib import Path
import json
import zipfile

ROOT = Path(__file__).resolve().parents[1]
DIST = ROOT / 'dist'


def allowed(path):
    return '__pycache__' not in path.parts and path.suffix not in ('.pyc', '.blend1')


def add_tree(archive, folder, prefix=''):
    for path in sorted(folder.rglob('*')):
        if path.is_file() and allowed(path):
            if folder == DIST / 'rbd_acceptance' and path.suffix == '.png' and not path.name.startswith(
                    ('wall_live_', 'wall_baked_', 'falling_live_', 'offline_verified_')):
                continue
            archive.write(path, prefix + path.relative_to(ROOT).as_posix())


if __name__ == '__main__':
    DIST.mkdir(exist_ok=True)
    with zipfile.ZipFile(DIST / 'MiniHou-Link-RBD.zip', 'w', zipfile.ZIP_DEFLATED) as z:
        for path in (ROOT / 'blender_addon/minihou_link').rglob('*'):
            if path.is_file() and allowed(path):
                z.write(path, path.relative_to(ROOT / 'blender_addon').as_posix())
    with zipfile.ZipFile(DIST / 'MiniHou-RBD-Windows.zip', 'w', zipfile.ZIP_DEFLATED) as z:
        prefix = 'MiniHou-RBD/'
        for folder in ('minihou', 'blender_addon', 'nodes', 'docs', 'tests', 'bench', 'tools', 'examples', 'build/runtime/python312'):
            add_tree(z, ROOT / folder, prefix)
        for name in ('README.md', 'pyproject.toml', 'Start-RBD.cmd'):
            z.write(ROOT / name, prefix + name)
        z.write(DIST / 'MiniHou-Link-RBD.zip', prefix + 'MiniHou-Link-RBD.zip')
        add_tree(z, DIST / 'rbd_acceptance', prefix)
    print(json.dumps({p.name: p.stat().st_size for p in DIST.glob('*.zip')}, indent=2))
