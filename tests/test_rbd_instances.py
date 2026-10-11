"""RBD ownership through the actual per-connection modifier API."""
import importlib.util
import unittest
import numpy as np
from minihou import Graph, Evaluator
from minihou.link.instances import InstanceManager
from minihou.link.session import Session
from tools.make_rbd_examples import node


@unittest.skipUnless(importlib.util.find_spec('mujoco'), 'optional physics extra not installed')
class RBDInstances(unittest.TestCase):
    def test_owned_worlds_and_disconnect_cleanup(self):
        graph = dict(nodes={'in': node('modifier_input'), 'c': node('rbd_configure', geo='in'),
                           's': node('rbd_solver', bodies='c')}, display='s',
                     exposed=[dict(name='Velocity', node='c', param='velocity')])
        ev = Evaluator()
        geo = ev.cook(Graph.from_dict(dict(nodes={'b': node('box')}, display='b')))
        session = Session(Graph.from_dict(dict(nodes={'b': node('box')}, display='b')))
        manager = InstanceManager(session)
        self.addCleanup(manager.clear)
        h = dict(instance_id='one', graph=graph, frame=5, params={'Velocity': [3, 0, 0]})
        _, one = manager.cook(h, geo)
        h.update(instance_id='two', params={'Velocity': [-3, 0, 0]})
        _, two = manager.cook(h, geo)
        self.assertGreater(one['point/P'][:, 0].mean(), 0)
        self.assertLess(two['point/P'][:, 0].mean(), 0)
        first = manager.instances['one'].rbd.worlds['s'][1]
        second = manager.instances['two'].rbd.worlds['s'][1]
        self.assertIsNot(first.model, second.model)
        manager.remove('one')
        self.assertIsNone(first.model)
        self.assertIsNotNone(second.model)
        manager.clear()
        self.assertIsNone(second.model)

    def test_different_collider_geometry_and_attributes_invalidate(self):
        from tests.test_rbd import drop
        ev = Evaluator()
        self.addCleanup(ev.close)
        graph = drop()
        floor = ev.cook(graph, target='floor').copy()
        ev.cook(graph, feeds={'floor': ('floor1', floor.freeze())}, frame=50)
        first = ev.rbd.worlds['sim'][1]
        changed = floor.copy()
        changed.prim['rbd_friction'] = np.full(changed.nprims, .2, np.float32)
        ev.cook(graph, feeds={'floor': ('floor2', changed.freeze())}, frame=50)
        self.assertIsNone(first.model)
        self.assertAlmostEqual(float(ev.rbd.worlds['sim'][1].pieces[-1].values['friction']), .2)


if __name__ == '__main__':
    unittest.main()
