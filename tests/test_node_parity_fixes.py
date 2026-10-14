"""Behavioral regressions for missing Houdini inputs and viewport controls."""
import json
import unittest

import numpy as np

from minihou import Evaluator, Geometry, Graph, handles
from tests.test_nodes import run


class NodeParityFixTests(unittest.TestCase):
    def test_reference_delete_is_domain_specific_and_preserves_position(self):
        src = run('grid').copy()
        src.point['weight'] = np.ones(src.npoints, np.float32)
        src.prim['weight'] = np.ones(src.nprims, np.float32)
        src.detail['tag'] = 1
        ref = Geometry.points_only(np.zeros((1, 3), np.float32))
        ref.point['weight'] = np.ones(1, np.float32)
        ref.detail['tag'] = 2
        out = run('attribute_delete', src, ref)
        self.assertNotIn('weight', out.point)
        self.assertIn('weight', out.prim)
        self.assertNotIn('tag', out.detail)
        np.testing.assert_array_equal(out.P, src.P)
        self.assertIn('weight', src.point)

    def test_delete_invert_and_domain(self):
        src = run('grid').copy()
        src.point['keep'] = np.ones(src.npoints, np.float32)
        out = run('attribute_delete', src, names='keep', domain='point', invert=True)
        self.assertEqual(set(out.point.names()), {'P', 'keep'})
        self.assertEqual(out.prim.names(), src.prim.names())

    def test_delete_reference_wired_through_evaluator(self):
        g = Graph.from_dict({'nodes': {
            'src': {'type': 'grid'}, 'ref': {'type': 'grid'},
            'del': {'type': 'attribute_delete', 'inputs': {'geo': 'src', 'reference': 'ref'}}}, 'display': 'del'})
        out = Evaluator().cook(g)
        self.assertEqual(out.point.names(), ['P'])

    def test_blur_distance_weights_use_rest_not_deformed_positions(self):
        rest = Geometry.from_arrays(np.array([[0, 0, 0], [1, 0, 0], [4, 0, 0]], np.float32), [0, 1, 2], [0, 3], closed=[False])
        src = rest.copy()
        src.point['P'] = np.array([[0, 0, 0], [3, 0, 0], [4, 0, 0]], np.float32)
        src.point['value'] = np.array([0, 0, 12], np.float32)
        out = run('attribute_blur', src, rest, names='value', iterations=1, strength=1, pin_boundary=False, weighting='distance')
        self.assertAlmostEqual(float(out.point['value'][1]), 3)
        changed = run('attribute_blur', src, names='value', iterations=1, strength=1, pin_boundary=False, weighting='distance')
        self.assertAlmostEqual(float(changed.point['value'][1]), 9)
        np.testing.assert_array_equal(src.point['value'], [0, 0, 12])

    def test_blur_rejects_incompatible_rest(self):
        with self.assertRaisesRegex(ValueError, 'matching'):
            run('attribute_blur', run('grid'), run('box'), weighting='distance')

    def test_scatter_group_keeps_original_primitive_indices(self):
        src = run('grid', columns=3, rows=2)
        out = run('scatter', src, count=1000, group='1')
        self.assertEqual(out.npoints, 1000)
        np.testing.assert_array_equal(out.point['primnum'], np.ones(1000))
        self.assertGreaterEqual(float(out.P[:, 0].min()), 0)

    def test_scatter_primitive_density_and_determinism(self):
        src = run('grid', columns=3, rows=2).copy()
        src.prim['density'] = np.array([0, 1], np.float32)
        a = run('scatter', src, count=500, density='density', seed=42)
        b = run('scatter', src, count=500, density='density', seed=42)
        np.testing.assert_array_equal(a.P, b.P)
        self.assertTrue(np.all(a.point['primnum'] == 1))

    def test_scatter_linear_point_density_has_correct_expected_position(self):
        src = Geometry.from_arrays(np.array([[0, 0, 0], [1, 0, 0], [0, 1, 0]], np.float32), [0, 1, 2], [0, 3])
        src.point['density'] = np.array([0, 1, 0], np.float32)
        out = run('scatter', src, count=20000, density='density', seed=8)
        np.testing.assert_allclose(out.P.mean(axis=0), [.5, .25, 0], atol=.008)
        self.assertTrue(np.all(out.P.sum(axis=1) <= 1.000001))

    def test_scatter_zero_and_invalid_density(self):
        src = run('grid').copy()
        src.point['density'] = np.zeros(src.npoints, np.float32)
        self.assertEqual(run('scatter', src, density='density').npoints, 0)
        bad = src.copy()
        bad.point['density'] = np.full(src.npoints, -1, np.float32)
        with self.assertRaisesRegex(ValueError, 'non-negative'):
            run('scatter', bad, density='density')
        with self.assertRaisesRegex(ValueError, 'surface mode'):
            run('scatter', src, mode='volume', group='0')

    def test_scatter_rejects_two_density_controls(self):
        src = run('grid').copy()
        src.point['density'] = np.ones(src.npoints, np.float32)
        with self.assertRaisesRegex(ValueError, 'either density or density_attrib'):
            run('scatter', src, density='density', density_attrib='density')

    def test_scatter_density_controls_keep_relaxation_and_ids(self):
        src = run('grid', columns=3, rows=3).copy()
        src.point['weight'] = np.ones(src.npoints, np.float32)
        for density_control in ('density_attrib', 'density'):
            with self.subTest(control=density_control):
                params = {density_control: 'weight', 'group': '0-3', 'count': 32, 'seed': 17}
                plain = run('scatter', src, **params)
                relaxed = run('scatter', src, relax_iterations=2, output_id=True, **params)
                self.assertEqual(relaxed.npoints, plain.npoints)
                np.testing.assert_array_equal(relaxed.point['id'], np.arange(32))
                self.assertFalse(np.allclose(plain.P, relaxed.P))
                np.testing.assert_allclose(relaxed.P[:, 2], 0, atol=1e-6)
                self.assertNotIn('id', plain.point)

    def test_solver_auxiliary_input_changes_invalidate_simulated_frames(self):
        g = Graph.from_dict({'nodes': {
            'src': {'type': 'grid'}, 'target': {'type': 'grid', 'params': {'center': [0, 0, 1]}},
            's': {'type': 'solver', 'inputs': {'geo': 'src', 'input2': 'target'},
                  'params': {'body': 'wrangle', 'params': json.dumps({'snippet': '@P.z += @opinput1_P.z'})}}}, 'display': 's'})
        ev = Evaluator()
        np.testing.assert_allclose(ev.cook(g, frame=4).P[:, 2], 3)
        g.nodes['target'].params['center'] = [0, 0, 2]
        np.testing.assert_allclose(ev.cook(g, frame=4).P[:, 2], 6)

    def test_solidify_thickness_gizmo_matches_offset_and_selection(self):
        src = run('grid', columns=2, rows=2)
        for offset in [-1, 0, 1]:
            params = {'offset': offset, 'thickness': .2}
            out = run('solidify', src, **params)
            h = handles.handles_for('solidify', [src], out, params)[0]
            self.assertEqual(h['param'], 'thickness')
            self.assertEqual(h['dir'][2], -1 if offset < 0 else 1)
            self.assertEqual(h['scale'], 2 if offset == 0 else 1)
        self.assertEqual(handles.handles_for('solidify', [src], src, {'group': 'nonexistent'}), [])

    def test_ray_lift_gizmo_uses_hit_normal(self):
        src = run('grid', center=[0, 0, 1], columns=2, rows=2)
        target = run('grid', columns=2, rows=2)
        out = run('ray', src, target, lift=.2)
        h = handles.handles_for('ray', [src, target], out, {'lift': .2})[0]
        self.assertEqual(h['param'], 'lift')
        np.testing.assert_allclose(h['dir'], [0, 0, 1])
        self.assertAlmostEqual(h['origin'][2], .2, places=6)

    def test_manual_uv_seam_splits_coplanar_faces(self):
        from minihou import edges, topo
        src = run('grid', columns=3, rows=2)
        internal = topo.edges(src).count == 2
        src = edges.write(src, edges.PREFIX+'cut', internal.astype(np.int32))
        plain = run('uv_flatten', src, seam_angle=180)
        cut = run('uv_flatten', src, seam_angle=180, seams='cut')
        self.assertEqual(len(np.unique(plain.prim['island'])), 1)
        self.assertEqual(len(np.unique(cut.prim['island'])), 2)
        self.assertTrue(np.isfinite(cut.vertex['uv']).all())

    def test_primitive_center_gizmos_target_source_parameters(self):
        for name in ('box', 'sphere'):
            src = run(name, center=[2, 3, 4])
            controls = handles.handles_for(name, [], src, {'center': [2, 3, 4]})
            moves = [h for h in controls if h.get('tool') == 'move']
            self.assertEqual([h['comp'] for h in moves], [0, 1, 2])
            self.assertTrue(all(h['param'] == 'center' for h in moves))
            np.testing.assert_allclose([h['origin'] for h in moves], [[2, 3, 4]]*3)

    def test_subdivide_separate_crease_source_and_named_edges(self):
        from minihou import edges, topo
        src = run('box')
        source = edges.write(src, edges.CREASE, np.full(len(topo.edges(src).points), 3, np.float32))
        source = edges.write(source, edges.PREFIX+'sharp', np.ones(len(topo.edges(src).points), np.int32))
        separate = run('subdivide', src, source, creases='sharp', depth=1)
        embedded = run('subdivide', source, creases='sharp', depth=1)
        np.testing.assert_array_equal(separate.P, embedded.P)
        self.assertNotIn(edges.CREASE, src.vertex)
        self.assertTrue(np.any(edges.values(separate, edges.CREASE) == 2))
        with self.assertRaisesRegex(ValueError, 'matching'):
            run('subdivide', src, run('grid'))

    def test_extrude_divisions_keep_all_walls_and_one_back(self):
        src = run('grid', columns=2, rows=2)
        out = run('polyextrude', src, divisions=4, distance=2, output_back=True, outgrp_back=True)
        self.assertEqual(out.nprims, 18)
        self.assertEqual(out.npoints, 20)
        self.assertEqual(int(out.prim['group_extrudeFront'].sum()), 1)
        self.assertEqual(int(out.prim['group_extrudeSide'].sum()), 16)
        self.assertEqual(int(out.prim['group_extrudeBack'].sum()), 1)
        np.testing.assert_allclose(np.unique(out.P[:, 2]), [0, .5, 1, 1.5, 2])
        self.assertFalse(any(n.startswith('group__extrude_work') for n in out.prim.names()))
        self.assertEqual(src.nprims, 1)

    def test_extrude_divisions_zero_amount_does_not_add_geometry(self):
        src = run('grid', columns=2, rows=2)
        out = run('polyextrude', src, divisions=5, distance=0)
        self.assertEqual((out.npoints, out.nprims), (src.npoints, src.nprims))
