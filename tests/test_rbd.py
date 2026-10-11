"""Real optional-backend acceptance tests, no Blender required."""
import copy
import importlib.util
import unittest
import numpy as np

from minihou import Evaluator, Graph, NodeError
from minihou import timectx
from minihou.rbd import Manager, pieces
from tools.make_rbd_examples import wall, falling, node


def drop(**settings):
    return Graph.from_dict(dict(nodes={
        'box': node('box', dict(center=[0, 0, 3])),
        'body': node('rbd_configure', **dict(geo='box')),
        'floor': node('box', dict(size=[20, 20, 1], center=[0, 0, -.5])),
        'sim': node('rbd_solver', settings, bodies='body', colliders='floor'),
    }, display='sim'))


@unittest.skipUnless(importlib.util.find_spec('mujoco'), 'optional physics extra not installed')
class RBDTests(unittest.TestCase):
    def setUp(self):
        self.ev = Evaluator()
        self.addCleanup(self.ev.close)

    def test_ground_contact_and_rigidity(self):
        g = drop()
        initial = self.ev.cook(g, frame=1)
        result = self.ev.cook(g, frame=90)
        self.assertGreater(result.P[:8, 2].min(), -.005)
        self.assertLess(result.P[:8, 2].min(), .015)
        a, b = initial.P[:8], result.P[:8]
        np.testing.assert_allclose(np.linalg.norm(a[:, None]-a, axis=2), np.linalg.norm(b[:, None]-b, axis=2), atol=1e-6)
        world = self.ev.rbd.worlds['sim'][1]
        self.assertGreater(world.data.ncon, 0)

    def test_scrub_replay_reset_and_subframes(self):
        g = drop()
        direct = self.ev.cook(g, frame=60).P.copy()
        self.ev.cook(g, frame=5)
        for f in range(6, 61):
            stepped = self.ev.cook(g, frame=f)
        np.testing.assert_array_equal(direct, stepped.P)
        # Force real physics replay, not merely an evaluator geometry cache hit.
        self.ev.cache.clear()
        self.ev.rbd.worlds['sim'][1].frames.clear()
        self.ev.rbd.worlds['sim'][1].cache_bytes = 0
        self.ev.cook(g, frame=4)
        np.testing.assert_array_equal(direct, self.ev.cook(g, frame=60).P)
        g.set_param('sim', 'reset', 1)
        np.testing.assert_array_equal(direct, self.ev.cook(g, frame=60).P)
        np.testing.assert_array_equal(direct, self.ev.cook(g, frame=60.9).P)

    def test_world_isolation_invalidation_and_cleanup(self):
        other = Evaluator()
        self.addCleanup(other.close)
        g = drop()
        self.ev.cook(g, frame=30)
        old = self.ev.rbd.worlds['sim'][1]
        other.cook(g, frame=8)
        self.assertIsNot(old.model, other.rbd.worlds['sim'][1].model)
        g.set_param('floor', 'center', [0, 0, 1])
        raised = self.ev.cook(g, frame=90)
        self.assertIsNone(old.model)
        self.assertGreater(raised.P[:8, 2].min(), 1.49)
        current = self.ev.rbd.worlds['sim'][1]
        g.set_param('body', 'velocity', [2, 0, 0])
        self.ev.cook(g, frame=90)
        self.assertIsNone(current.model)
        current = self.ev.rbd.worlds['sim'][1]
        self.ev.cook(g, frame=90, fps=30)
        self.assertIsNone(current.model)
        current = self.ev.rbd.worlds['sim'][1]
        self.ev.close()
        self.assertIsNone(current.model)
        self.assertFalse(self.ev.rbd.worlds)

    def test_spin_and_data_preservation(self):
        g = drop(gravity=[0, 0, 0], include_colliders=False)
        g.set_param('body', 'angular_velocity', [0, 0, 2])
        # Feeds emulate imported production geometry with all attribute domains.
        mesh = self.ev.cook(g, target='box').copy()
        rng = np.random.default_rng(1)
        mesh.point['Cd'] = rng.random((mesh.npoints, 3))
        mesh.vertex['uv'] = rng.random((mesh.nvertices, 2))
        mesh.vertex['N'] = np.tile([1., 0, 0], (mesh.nvertices, 1))
        mesh.point['N'] = np.tile([1., 0, 0], (mesh.npoints, 1))
        mesh.prim['N'] = np.tile([1., 0, 0], (mesh.nprims, 1))
        mesh.prim['material_index'] = np.arange(mesh.nprims, dtype=np.int32) % 2
        mesh.prim['group_inside'] = np.ones(mesh.nprims, np.int32)
        feeds = {'box': ('tagged', mesh.freeze())}
        result = self.ev.cook(g, frame=10, feeds=feeds)
        self.assertIs(mesh.topology, result.topology)
        for dom, attr in [('point', 'Cd'), ('vertex', 'uv'), ('prim', 'material_index'), ('prim', 'group_inside')]:
            np.testing.assert_array_equal(mesh.table(dom)[attr], result.table(dom)[attr])
        for dom in ('point', 'vertex', 'prim'):
            self.assertGreater(np.abs(result.table(dom)['N'][:, 1]).max(), .3)
            np.testing.assert_allclose(np.linalg.norm(result.table(dom)['N'], axis=1), 1, atol=1e-6)
        np.testing.assert_array_equal(mesh.vertex['N'][:, 1], 0)

    def test_brick_wall_projectile_knockdown(self):
        g = Graph.from_dict(wall())
        initial = self.ev.cook(g, frame=1)
        result = self.ev.cook(g, frame=72)
        # First 48 boxes, eight points each; projectile is a separately identified body.
        centers0 = initial.P[:384].reshape(48, 8, 3).mean(1)
        centers = result.P[:384].reshape(48, 8, 3).mean(1)
        self.assertGreater(np.count_nonzero(np.linalg.norm(centers-centers0, axis=1) > .3), 8)
        self.assertGreater(centers[:, 1].max(), 1.)
        self.assertGreater(result.P[:, 2].min(), -.501)  # includes floor bottom
        self.assertGreater(np.abs(result.prim['rbd_w'][:288]).max(), .1)

    def test_fracture_and_bounded_cache(self):
        g = Graph.from_dict(falling())
        self.ev.cook(g, frame=48)
        world = self.ev.rbd.worlds['sim'][1]
        self.assertEqual(len(world.pieces), 26)
        world.settings['cache_mb'] = .001
        for f in range(49, 65):
            world.pose(f)
        self.assertLessEqual(world.cache_bytes, 32*1024**2)
        # Fit several snapshots in a small budget and prove eviction.
        world.frames.clear(); world.cache_bytes = 0
        world.settings['cache_mb'] = .02
        for f in range(65, 85):
            world.pose(f)
        self.assertLessEqual(world.cache_bytes, .02*1024**2)
        self.assertLess(len(world.frames), 20)

    def test_reject_unsupported_geometry_and_time(self):
        for kind in ('torus', 'grid'):
            g = drop()
            g.nodes['box'].type = kind
            g.nodes['box'].params = {}
            with self.assertRaisesRegex(NodeError, 'concave|open|solid'):
                self.ev.cook(g)
        data = drop().to_dict()
        data['nodes']['animated'] = node('wrangle', dict(snippet='@P.z += @Time'), geo='box')
        data['nodes']['body']['inputs']['geo'] = 'animated'
        with self.assertRaisesRegex(NodeError, 'static'):
            self.ev.cook(Graph.from_dict(data), frame=4)


if __name__ == '__main__':
    unittest.main()
