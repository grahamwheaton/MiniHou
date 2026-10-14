"""Focused group export tests inside an isolated factory-startup Blender."""
import sys,unittest
from pathlib import Path
root=Path(__file__).resolve().parents[1]
sys.path[:0]=[str(root),str(root/'blender_addon')]
import minihou,bpy
print('MINIHOU_IMPORT',minihou.__file__,flush=True)
print('BLENDER',bpy.app.version_string,flush=True)
r=unittest.TextTestRunner(verbosity=2).run(unittest.defaultTestLoader.loadTestsFromName('tests.test_group_blender_export'))
sys.exit(0 if r.wasSuccessful() else 1)
