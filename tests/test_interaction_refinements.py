import unittest
import numpy as np
from minihou import Graph, Evaluator
from minihou.edit import apply_ops

class Refinements(unittest.TestCase):
    def cook(self,nodes,target):
        with_eval=Evaluator()
        self.addCleanup(with_eval.close)
        return with_eval.cook(Graph.from_dict({"nodes":nodes,"display":target}),target)

    def test_box_axes_rotation_normals_and_input_bounds(self):
        nodes={"b":{"type":"box","params":{"size":[2,4,6],"center":[3,0,0]}},
               "bound":{"type":"box","inputs":{"geo":"b"},"params":{"axis_divisions":[2,3,4],"add_vertex_normals":True}}}
        g=self.cook(nodes,"bound")
        self.assertEqual(g.nprims,2*(6+12+8))
        np.testing.assert_allclose(g.P.min(0),[2,-2,-3])
        self.assertIn("N",g.vertex)
        nodes['bound']['params'].update(rotate=[0,0,90],uniform_scale=2)
        g=self.cook(nodes,'bound');np.testing.assert_allclose(g.P.max(0)-g.P.min(0),[8,4,12],atol=1e-5)

    def test_box_triangle_points_and_unwelded(self):
        g=self.cook({'b':{'type':'box','params':{'connectivity':'triangles','consolidate':False}}},'b')
        self.assertEqual((g.npoints,g.nprims),(24,12))
        g=self.cook({'b':{'type':'box','params':{'primitive_type':'points'}}},'b')
        self.assertEqual((g.npoints,g.nprims),(8,0))

    def test_edge_blast_removes_incident_faces_empty_preserves(self):
        nodes={'b':{'type':'box'},'x':{'type':'blast','inputs':{'geo':'b'},'params':{'domain':'edges','group':'p0-1'}}}
        self.assertEqual(self.cook(nodes,'x').nprims,4)
        nodes['x']['params']['group']=''
        self.assertEqual(self.cook(nodes,'x').nprims,6)

    def test_dissolve_chain_reconnects_and_preserves_branch(self):
        g=Graph.from_dict({'nodes':{'a':{'type':'box'},'b':{'type':'transform','inputs':{'geo':'a'}},'c':{'type':'transform','inputs':{'geo':'b'}},'o':{'type':'output','inputs':{'geo':'c'}},'branch':{'type':'output','inputs':{'geo':'b'}}},'display':'c'})
        out,_=apply_ops(g,[{'op':'dissolve','id':['b','c']}])
        self.assertEqual(out.nodes['o'].inputs['geo'],'a')
        self.assertEqual(out.nodes['branch'].inputs['geo'],'a')
        self.assertEqual(out.display,'a')
        self.assertIn('b',g.nodes)

    def test_primitive_transform_preserves_center(self):
        for typ in ['sphere','tube','torus','grid']:
            g=self.cook({'p':{'type':typ,'params':{'center':[3,2,1],'rotate':[0,90,0],'uniform_scale':2}}},'p')
            self.assertTrue(np.isfinite(g.P).all())
            np.testing.assert_allclose((g.P.min(0)+g.P.max(0))/2,[3,2,1],atol=1e-5)
