"""blender --background --factory-startup --python tools/blender_modeling_smoke.py"""
import pathlib
import sys
import unittest

ROOT=pathlib.Path(__file__).resolve().parents[1]
sys.path[:0]=[str(ROOT),str(ROOT/'blender_addon')]
import bpy
import minihou
assert pathlib.Path(minihou.__file__).resolve().parent==ROOT/'minihou'
print('Blender:',bpy.app.version_string,'source:',minihou.__file__,flush=True)
suite=unittest.defaultTestLoader.loadTestsFromName('tests.test_blender_modeling')
result=unittest.TextTestRunner(verbosity=2).run(suite)
sys.exit(not result.wasSuccessful())
