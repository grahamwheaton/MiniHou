"""Object scale on the way into and out of MiniHou (bpy-free)."""
from __future__ import annotations
import sys
import types
import unittest
from pathlib import Path

import numpy as np

ROOT = Path(__file__).resolve().parents[1]
PKG = "_scale_space_addon"
package = types.ModuleType(PKG)
package.__path__ = [str(ROOT / "blender_addon/minihou_link")]
sys.modules[PKG] = package
from importlib import import_module
scale_space = import_module(PKG + ".scale_space")


class Matrix:
    def __init__(self, scale): self._scale = scale
    def to_scale(self): return self._scale


class Obj:
    def __init__(self, scale): self.matrix_world = Matrix(scale)


class ScaleSpaceTests(unittest.TestCase):
    def points(self):
        return np.array([[1, 1, 1], [-1, 2, 0.5]], np.float32)

    def test_unit_scale_does_nothing(self):
        self.assertIsNone(scale_space.world_scale(Obj((1.0, 1.0, 1.0))))
        arrays = {scale_space.P_KEY: self.points()}
        before = arrays[scale_space.P_KEY]
        scale_space.scale_arrays(arrays, None)
        self.assertIs(arrays[scale_space.P_KEY], before)

    def test_non_uniform_scale_is_applied_and_undone(self):
        s = scale_space.world_scale(Obj((15.421, 15.421, 1.0)))
        arrays = {scale_space.P_KEY: self.points(), "point/id": np.array([1, 2], np.int32)}
        scale_space.scale_arrays(arrays, s)
        np.testing.assert_allclose(arrays[scale_space.P_KEY], [[15.421, 15.421, 1], [-15.421, 30.842, 0.5]], rtol=1e-5)
        np.testing.assert_array_equal(arrays["point/id"], [1, 2])      # other arrays are untouched
        scale_space.unscale_arrays(arrays, s)
        np.testing.assert_allclose(arrays[scale_space.P_KEY], self.points(), rtol=1e-5)

    def test_zero_or_broken_scale_is_ignored(self):
        self.assertIsNone(scale_space.world_scale(Obj((0.0, 1.0, 1.0))))
        self.assertIsNone(scale_space.world_scale(Obj((float("nan"), 1.0, 1.0))))
        self.assertIsNone(scale_space.world_scale(object()))

    def test_negative_scale_round_trips(self):
        s = scale_space.world_scale(Obj((-2.0, 1.0, 3.0)))
        arrays = {scale_space.P_KEY: self.points()}
        scale_space.scale_arrays(arrays, s)
        scale_space.unscale_arrays(arrays, s)
        np.testing.assert_allclose(arrays[scale_space.P_KEY], self.points(), rtol=1e-6)

    def test_missing_positions_are_fine(self):
        arrays = {"point/id": np.array([1], np.int32)}
        scale_space.scale_arrays(arrays, (2.0, 2.0, 2.0))
        scale_space.unscale_arrays(arrays, (2.0, 2.0, 2.0))
        self.assertEqual(list(arrays), ["point/id"])


if __name__ == "__main__":
    unittest.main()
