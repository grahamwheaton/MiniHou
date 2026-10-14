import unittest
import numpy as np

from minihou.geometry import Geometry
from minihou.nodes.transform import transform
from minihou.proportional import FALLOFFS, falloff_weights, selection_weights


class ProportionalTransformTests(unittest.TestCase):
    def apply(self, geo, **kwargs):
        params = dict(translate=(0, 0, 0), rotate=(0, 0, 0), scale=(1, 1, 1),
                      uniform_scale=1, pivot=(0, 0, 0), group='0', proportional_enabled=True,
                      proportional_radius=2, proportional_falloff='linear')
        params.update(kwargs)
        return transform(geo, **params)

    def test_selection_radius_and_source_preserved(self):
        geo = Geometry.points_only([[0, 0, 0], [1, 0, 0], [2, 0, 0]])
        before = geo.P.copy()
        result = self.apply(geo, translate=(0, 2, 0))
        np.testing.assert_allclose(result.P[:, 1], [2, 1, 0])
        np.testing.assert_array_equal(geo.P, before)
        old = self.apply(geo, translate=(0, 2, 0), proportional_enabled=False)
        np.testing.assert_allclose(old.P[:, 1], [2, 0, 0])

    def test_rotation_uses_arc_and_scale_uses_factor(self):
        geo = Geometry.points_only([[0, 0, 0], [1, 0, 0]])
        result = self.apply(geo, rotate=(0, 0, 180))
        np.testing.assert_allclose(result.P[1], [0, 1, 0], atol=1e-6)
        result = self.apply(geo, scale=(3, 1, 1))
        np.testing.assert_allclose(result.P[1], [2, 0, 0])

    def test_selected_matches_legacy_axes_and_normals(self):
        geo = Geometry.points_only([[1, 2, 3], [2, 4, 6]])
        geo.point['N'] = np.array([[1, 0, 0], [0, 1, 0]], np.float32)
        kwargs = dict(group='', rotate=(30, 40, 50), scale=(2, 3, 4),
                      translate=(1, 3, 2), axis_rotation=(12, 20, 30), pivot=(1, 1, 1))
        weighted = self.apply(geo, **kwargs)
        legacy = self.apply(geo, proportional_enabled=False, **kwargs)
        np.testing.assert_allclose(weighted.P, legacy.P, atol=2e-6)
        np.testing.assert_allclose(weighted.point['N'], legacy.point['N'], atol=1e-6)

    def test_connected_path_does_not_cross_close_gap(self):
        geo = Geometry.from_arrays([[0, 0, 0], [0, 2, 0], [0.1, 0, 0], [0.2, 0, 0]],
                                   [0, 1, 2], [0, 3], [False])
        result = self.apply(geo, translate=(1, 0, 0), proportional_connected=True)
        np.testing.assert_allclose(result.P[2:], geo.P[2:])
        ordinary = self.apply(geo, translate=(1, 0, 0))
        self.assertGreater(ordinary.P[2, 0], .1)

    def test_projected_depth_and_empty_selection(self):
        geo = Geometry.points_only([[0, 0, 0], [1, 0, 100]])
        result = self.apply(geo, translate=(0, 2, 0), proportional_projected=True)
        self.assertAlmostEqual(result.P[1, 1], 1)
        np.testing.assert_array_equal(self.apply(geo, group='99', translate=(1, 0, 0)).P, geo.P)

    def test_falloffs_and_deterministic_random(self):
        for mode in FALLOFFS:
            a = falloff_weights(np.array([0, .5, 1, np.inf]), 1, mode)
            self.assertTrue(np.isfinite(a).all())
            self.assertTrue(((a >= 0) & (a <= 1)).all())
            np.testing.assert_array_equal(a[2:], [0, 0])
            np.testing.assert_array_equal(a, falloff_weights(np.array([0, .5, 1, np.inf]), 1, mode))
        geo = Geometry.points_only([[0, 0, 0], [1, 0, 0]])
        self.assertEqual(selection_weights(geo, [True, False], 2, 'random')[0], 1)
        with self.assertRaises(ValueError):
            self.apply(geo, proportional_radius=0)


if __name__ == '__main__':
    unittest.main()
