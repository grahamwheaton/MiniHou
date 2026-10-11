"""Run unittest with this checkout ahead of an embedded runtime's configured _pth."""
import pathlib
import sys
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
import minihou

assert pathlib.Path(minihou.__file__).resolve().parent == ROOT / 'minihou'
print('Testing source:', minihou.__file__, flush=True)
if __name__ == '__main__':
    suite = (unittest.defaultTestLoader.loadTestsFromNames(sys.argv[1:]) if len(sys.argv) > 1
             else unittest.defaultTestLoader.discover(str(ROOT / 'tests'), top_level_dir=str(ROOT)))
    result = unittest.TextTestRunner(verbosity=2).run(suite)
    sys.exit(not result.wasSuccessful())
