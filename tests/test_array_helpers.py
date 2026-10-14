"""Bundled array systems: editable controls, exact layout and allocation guards."""
import unittest
from pathlib import Path
import numpy as np
from minihou import REGISTRY, Evaluator, Graph, NodeError
from minihou.library import NodeLibrary

ROOT = Path(__file__).resolve().parents[1]
NAMES = ('array', 'circle_array', 'grid_array', 'array_between_two_positions')

class ArrayHelperTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.library = NodeLibrary([ROOT / 'nodes'])
        cls.library.refresh()

    @classmethod
    def tearDownClass(cls):
        cls.library.unload()

    def cook(self, typ, params=None, source=None):
        nodes = {'a': {'type': typ, 'params': params or {}}}
        if source:
            nodes['s'] = source
            nodes['a']['inputs'] = {'source': 's'}
        return Evaluator().cook(Graph.from_dict({'nodes': nodes, 'display': 'a'}))

    def test_library_loads_and_examples_are_deterministic(self):
        self.assertFalse([f for f in self.library.status()['files'] if f['error']])
        for name in NAMES:
            nt = REGISTRY.get(name)
            self.assertEqual([i.name for i in nt.inputs], ['source'])
            self.assertEqual(nt.category, 'system')
            g = Graph.load(ROOT / 'examples' / (name + '.json'))
            a, b = Evaluator().cook(g), Evaluator().cook(g)
            np.testing.assert_array_equal(a.P, b.P)
            self.assertGreater(a.nprims, 0)

    def test_linear_offset_and_source_are_preserved(self):
        source = {'type':'box','params':{'size':[1,2,3]}}
        base = Evaluator().cook(Graph.from_dict({'nodes':{'s':source},'display':'s'}))
        out = self.cook('array', {'count':3,'offset':[2,-1,0.5], 'origin':[4,3,2]}, source)
        self.assertEqual(out.npoints, base.npoints * 3)
        self.assertEqual(out.nprims, base.nprims * 3)
        for i in range(3):
            np.testing.assert_allclose(out.P[i*base.npoints:(i+1)*base.npoints], base.P + [4+2*i,3-i,2+0.5*i])
        np.testing.assert_array_equal(out.point['array_index'], np.repeat(np.arange(3), base.npoints))

    def test_ring_has_no_duplicate_endpoint_and_rotation_is_optional(self):
        out = self.cook('array_circle_points',{'count':4,'radius':2,'center':[1,2,3], 'start_angle':90})
        np.testing.assert_allclose(out.P, [[1,4,3],[-1,2,3],[1,0,3],[3,2,3]], atol=1e-6)
        self.assertNotIn('rot',out.point)
        rotated = self.cook('array_circle_points',{'count':4,'rotate_copies':True})
        np.testing.assert_allclose(rotated.point['rot'][:,2], [0,np.pi/2,np.pi,3*np.pi/2], atol=1e-6)

    def test_grid_signed_spacing_and_single_row(self):
        out = self.cook('array_grid_points',{'columns':3,'rows':1,'spacing_x':-2,'origin':[1,3,5]})
        np.testing.assert_array_equal(out.P, [[1,3,5],[-1,3,5],[-3,3,5]])
        grid = self.cook('array_grid_points',{'columns':2,'rows':2,'spacing_x':3,'spacing_y':4})
        np.testing.assert_array_equal(grid.P, [[0,0,0],[3,0,0],[0,4,0],[3,4,0]])

    def test_between_endpoints_and_degenerate_cases(self):
        np.testing.assert_allclose(self.cook('array_between_points',{'count':3,'start':[1,2,3],'end':[3,6,9]}).P, [[1,2,3],[2,4,6],[3,6,9]])
        np.testing.assert_array_equal(self.cook('array_between_points',{'count':1,'start':[2,3,4]}).P, [[2,3,4]])
        np.testing.assert_array_equal(self.cook('array_between_points',{'count':2,'start':[2,3,4],'end':[2,3,4]}).P, [[2,3,4],[2,3,4]])

    def test_rejects_invalid_counts_and_grid_product_before_allocation(self):
        for typ in ('array_linear_points','array_circle_points','array_between_points'):
            for count in (0,10001):
                with self.assertRaises((ValueError,NodeError)):
                    self.cook(typ,{'count':count})
        with self.assertRaisesRegex(NodeError,'10000 total'):
            self.cook('grid_array',{'columns':10000,'rows':2})

    def test_realised_source_memory_guard_is_retained(self):
        source={'type':'sphere','params':{'rows':30,'columns':30}}
        with self.assertRaisesRegex(NodeError,'safety limit'):
            self.cook('array',{'count':10000},source)

if __name__ == '__main__':
    unittest.main()
