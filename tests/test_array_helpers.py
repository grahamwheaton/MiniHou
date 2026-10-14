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

    def test_spacing_examples_cook_deterministically(self):
        for suffix in ('distance','relative','end_point'):
            graph=Graph.load(ROOT / 'examples' / ('array_'+suffix+'.json'))
            a=Evaluator().cook(graph);b=Evaluator().cook(graph)
            np.testing.assert_array_equal(a.P,b.P)
            self.assertEqual(a.npoints,40)

    def test_distance_relative_and_end_point_controls(self):
        out=self.cook('array_linear_points',{'count':3,'offset':[0,3,4],'distance':2,'use_distance':True})
        np.testing.assert_allclose(out.P,[[0,0,0],[0,1.2,1.6],[0,2.4,3.2]],atol=1e-6)
        source={'type':'box','params':{'size':[2,3,4]}}
        out=self.cook('array',{'count':3,'relative_spacing':True,'relative_offset':[1,.5,-1]},source)
        np.testing.assert_allclose(out.P[8:16]-out.P[:8],[np.array([2,1.5,-4])]*8)
        out=self.cook('array',{'count':3,'use_end_point':True,'origin':[1,2,3],'end':[5,6,7]},source)
        np.testing.assert_allclose(out.P[16:24]-out.P[:8],[np.array([4,4,4])]*8)
        out=self.cook('array',{'count':1,'use_end_point':True,'origin':[1,2,3]},source)
        self.assertEqual(out.npoints,8)
        with self.assertRaisesRegex(NodeError,'nonzero'):
            self.cook('array_linear_points',{'use_distance':True,'offset':[0,0,0]})
        with self.assertRaisesRegex(NodeError,'source'):
            self.cook('array_linear_points',{'relative_spacing':True})

    def test_relative_grid_and_control_precedence(self):
        source={'type':'box','params':{'size':[2,3,4]}}
        out=self.cook('grid_array',{'columns':2,'rows':2,'relative_spacing':True,'spacing_x':.5,'spacing_y':2},source)
        np.testing.assert_allclose(out.P[8:16]-out.P[:8],[[1,0,0]]*8)
        np.testing.assert_allclose(out.P[16:24]-out.P[:8],[[0,6,0]]*8)
        out=self.cook('array_linear_points',{'count':2,'use_end_point':True,'relative_spacing':True,'use_distance':True,'end':[7,8,9]})
        np.testing.assert_allclose(out.P,[[0,0,0],[7,8,9]])

    def test_group_gizmos_forward_public_parameters_and_finite_positions(self):
        from minihou.handles import handles_for, has_handles
        expected={'array':{'origin','offset'},'circle_array':{'radius','center','start_angle'},
                  'grid_array':{'origin','spacing_x','spacing_y'},'array_between_two_positions':{'start','end'}}
        for name,params in expected.items():
            graph=Graph.from_dict({'nodes':{'a':{'type':name}},'display':'a'})
            geo=Evaluator().cook(graph);nt=REGISTRY.get(name)
            self.assertTrue(has_handles(name,nt))
            hs=handles_for(name,[],geo,graph.effective_params('a'),nt)
            self.assertTrue(params <= {h['param'] for h in hs})
            for h in hs:
                self.assertTrue(np.isfinite(h['origin']).all())
                self.assertTrue(np.isfinite(h['dir']).all())
                if h['kind']!='guide':self.assertIn(h['param'],{p.name for p in nt.params})
        graph=Graph.from_dict({'nodes':{'a':{'type':'array','params':{'use_end_point':True}}},'display':'a'})
        hs=handles_for('array',[],Evaluator().cook(graph),graph.effective_params('a'),REGISTRY.get('array'))
        self.assertIn('end',{h['param'] for h in hs})
        self.assertNotIn('offset',{h['param'] for h in hs})

    def test_hidden_handle_references_are_not_forwarded_and_reload_leaves_no_provider(self):
        import json,tempfile
        from minihou.handles import PROVIDERS
        from minihou.library import _system_type
        graph=Graph.from_dict({'nodes':{'t':{'type':'array_circle_points'}},'display':'t',
                              'exposed':[{'name':'Ring Size','node':'t','param':'radius'}]})
        nt=_system_type('test_ring',Path('test_ring.json'),graph,json.dumps(graph.to_dict()),REGISTRY)
        # Centre is hidden: the radius handle and circle guide cannot reference it.
        self.assertEqual(nt.handles,())
        self.assertEqual(nt.guides,())
        self.assertNotIn('array',PROVIDERS)
        before=REGISTRY.get('array').handles
        self.library.refresh()
        self.assertEqual(REGISTRY.get('array').handles,before)

if __name__ == '__main__':
    unittest.main()
