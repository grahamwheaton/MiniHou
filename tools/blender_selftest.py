"""Runs *inside* Blender: the addon test-suite against a real engine subprocess.

    blender --background --factory-startup --python tools/blender_selftest.py

Exit code 0 = all passed. Also works with the ``bpy`` module:  python tools/blender_selftest.py
"""
import os
import sys
import unittest

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path[:0] = [ROOT, os.path.join(ROOT, "blender_addon")]

import bpy  # noqa: E402

print(f"[selftest] Blender {bpy.app.version_string}, Python {sys.version.split()[0]}, engine python: "
      f"{os.environ.get('MINIHOU_ENGINE_PYTHON', sys.executable)}", flush=True)
suite = unittest.defaultTestLoader.loadTestsFromNames([
    "tests.test_addon_state", "tests.test_blender_addon", "tests.test_blender_modifiers",
    "tests.test_blender_server_control",
    "tests.test_blender_source_objects",
    "tests.test_blender_geonodes",
    "tests.test_blender_modeling",
])
result = unittest.TextTestRunner(verbosity=2).run(suite)
sys.stdout.flush()
sys.exit(0 if result.wasSuccessful() else 1)
