import unittest
import numpy as np
from minihou import edges, topo
from tests.test_edge_tools import mesh, run


class ProfileTests(unittest.TestCase):
    def test_skin_multiple_profiles_and_attributes(self):
        g = mesh([[x,y,z] for z in [0,1,3] for x,y in [(0,0),(2,0),(2,2),(0,2)]],
                 [[0,1,2,3],[4,5,6,7],[8,9,10,11]])
        g.point['id'] = np.arange(12)
        g.vertex['uv'] = g.P[g.topology.vertex_point,:2]
        g.prim['material_index'] = [2,3,4]
        g = run('edge_group',g,name='rings')
        g.freeze()
        out = run('skin',g)
        self.assertEqual((out.npoints,out.nprims),(12,8))
        self.assertEqual((topo.edges(out).count == 1).sum(),8)
        np.testing.assert_array_equal(out.prim['material_index'],[2]*4+[3]*4)
        self.assertEqual(edges.mask(out,'rings').sum(),12)
        np.testing.assert_array_equal(out.point['id'],g.point['id'])
        self.assertTrue(np.all(out.topology.prim_sizes()==4))

    def test_skin_open_and_validation(self):
        g = mesh([[x,0,z] for z in [0,1,2] for x in [0,1,2]],[[0,1,2],[3,4,5],[6,7,8]],[False]*3)
        self.assertEqual(run('skin',g).nprims,4)
        self.assertEqual(run('skin',g,keep_profiles=True).nprims,7)
        with self.assertRaises(ValueError):
            run('skin',g,group='0')
        bad = mesh([[0,0,0],[1,0,0],[0,1,0]],[[0,1,2],[0,1,2]])
        with self.assertRaises(ValueError):
            run('skin',bad)

    def test_offset_rotated_plane_winding_attributes(self):
        from minihou.nodes._util import euler_xyz_matrix
        R = euler_xyz_matrix([30,20,50])
        points = np.array([[0,0,0],[2,0,0],[2,2,0],[0,2,0]],float)
        for face in [[0,1,2,3],[3,2,1,0]]:
            g = mesh(points@R.T+[4,5,6],[face])
            g.point['id'] = np.arange(4)
            g.vertex['uv'] = points[face,:2]
            g = run('edge_group',g,name='outline')
            g.freeze()
            out = run('polyexpand2d',g,distance=0.5)
            q = (out.P[out.topology.vertex_point]-[4,5,6])@R
            np.testing.assert_allclose(q[:,:2].min(0),[-.5,-.5],atol=1e-5)
            np.testing.assert_allclose(q[:,:2].max(0),[2.5,2.5],atol=1e-5)
            np.testing.assert_allclose(q[:,2],0,atol=1e-5)
            np.testing.assert_array_equal(out.vertex['uv'],g.vertex['uv'])
            self.assertEqual(edges.mask(out,'outline').sum(),4)
            self.assertIs(run('polyexpand2d',g,distance=0),g)
            with self.assertRaises(ValueError):
                run('polyexpand2d',g,distance=-1.1)

    def test_offset_rejects_concave_nonplanar_degenerate(self):
        for pts in [[[0,0,0],[2,0,0],[1,.5,0],[2,2,0],[0,2,0]],
                    [[0,0,0],[2,0,0],[2,2,1],[0,2,0]],
                    [[0,0,0],[0,0,0],[1,1,0]]]:
            with self.assertRaises(ValueError):
                run('polyexpand2d',mesh(pts,[list(range(len(pts)))]))


if __name__ == '__main__':
    unittest.main()
