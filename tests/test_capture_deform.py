import unittest
import numpy as np
from tests.test_edge_tools import mesh,run


class CaptureTests(unittest.TestCase):
    def test_soft_falloff_group_and_immutable_attributes(self):
        g = mesh([[0,0,0],[.5,0,0],[1,0,0],[2,0,0]],[[0,1,2,3]],[False])
        g.point['group_move']=[1,1,1,0]
        g.vertex['uv']=np.zeros((4,2),np.float32)
        g.prim['material_index']=[4]
        g.freeze()
        out = run('soft_transform',g,translate=[0,2,0],group='move')
        np.testing.assert_allclose(out.P[:,1],[2,1,0,0])
        np.testing.assert_array_equal(g.P[:,1],0)
        self.assertIs(out.topology,g.topology)
        self.assertIs(out.vertex['uv'],g.vertex['uv'])
        np.testing.assert_array_equal(out.prim['material_index'],[4])

    def test_lattice_affine_and_normal_transform(self):
        rest = run('lattice_grid',columns=3,rows=3,layers=3)
        changed = run('transform',rest,rotate=[20,30,40],scale=[2,1,3],translate=[3,2,1])
        g = mesh([[.1,.2,.3],[.5,0,.2],[-.2,.1,.1]],[[0,1,2]])
        g.point['N']=np.tile([0,0,1.],(3,1))
        g.vertex['N']=g.point['N'].copy()
        g.freeze(); rest.freeze(); changed.freeze()
        out = run('lattice',g,rest,changed)
        want = run('transform',g,rotate=[20,30,40],scale=[2,1,3],translate=[3,2,1])
        np.testing.assert_allclose(out.P,want.P,atol=1e-6)
        np.testing.assert_allclose(out.point['N'],want.point['N'],atol=1e-5)
        np.testing.assert_allclose(out.vertex['N'],want.point['N'],atol=1e-5)
        np.testing.assert_array_equal(run('lattice',g,rest,rest).P,g.P)

    def test_lattice_outside_and_bad_cage(self):
        rest = run('lattice_grid')
        changed = run('transform',rest,translate=[1,2,3])
        g = mesh([[2,0,0]],[])
        np.testing.assert_allclose(run('lattice',g,rest,changed).P,[[3,2,3]])
        np.testing.assert_allclose(run('lattice',g,rest,changed,outside='unchanged').P,g.P)
        bad = rest.copy(); bad.point['P']=rest.P.copy(); bad.point['P'][0]=bad.P[1]
        with self.assertRaises(ValueError):
            run('lattice',g,bad,changed)
        with self.assertRaises(ValueError):
            run('lattice_grid',size=[0,2,2])

    def test_point_deform_rigid_affine_and_rank_deficiency(self):
        rest = run('lattice_grid')
        changed = run('transform',rest,rotate=[20,30,40],scale=[2,1,3],translate=[3,2,1])
        g = mesh([[.1,.2,.3],[.5,0,.2],[-.2,.1,.1]],[[0,1,2]])
        g.vertex['uv']=np.zeros((3,2),np.float32)
        g.freeze()
        out = run('point_deform',g,rest,changed)
        want = run('transform',g,rotate=[20,30,40],scale=[2,1,3],translate=[3,2,1])
        np.testing.assert_allclose(out.P,want.P,atol=1e-5)
        self.assertIs(out.vertex['uv'],g.vertex['uv'])
        one = mesh([[0,0,0]],[])
        moved = mesh([[1,2,3]],[])
        np.testing.assert_allclose(run('point_deform',g,one,moved).P,g.P+[1,2,3],atol=1e-6)
        np.testing.assert_array_equal(run('point_deform',g,one,moved,radius=.01).P,g.P)
        with self.assertRaises(ValueError):
            run('point_deform',g,rest,one)

    def test_group_preservation_in_capture(self):
        rest=run('lattice_grid')
        moved=run('transform',rest,translate=[1,0,0])
        g=mesh([[0,0,0],[.5,0,0]],[[0,1]],[False])
        for typ in ['lattice','point_deform']:
            out=run(typ,g,rest,moved,group='0')
            np.testing.assert_allclose(out.P,[[1,0,0],[.5,0,0]])


if __name__=='__main__':
    unittest.main()
