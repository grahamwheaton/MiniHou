"""Distance-based reproduction of Graham's rounded-corner inset screenshot."""
import unittest
import numpy as np
from tests.test_nodes import run
from minihou.nodes import _build as B


def rounded_face():
    arc=np.array([[.75+.25*np.cos(a),.75+.25*np.sin(a),0.] for a in np.linspace(0,np.pi/2,9)])
    points=np.vstack([[0,0,0],[1,0,0],arc,[0,1,0]])
    return B.make(points,np.arange(len(points)),np.array([0,len(points)]))


class PlanarInsetTests(unittest.TestCase):
    def test_rounded_corner_exact_perpendicular_distance(self):
        geo=rounded_face()
        for mode in ('connected','individual'):
            for tool,sign in (('inset',1),('outset',-1)):
                out=run(tool,geo,amount=.06,mode=mode);out.validate()
                front=out.P[out.topology.vertex_point[:geo.nvertices]]
                edges=np.roll(geo.P,-1,axis=0)-geo.P
                normals=np.c_[-edges[:,1],edges[:,0],np.zeros(len(edges))]
                normals/=np.linalg.norm(normals,axis=1)[:,None]
                # BOTH endpoints of every output edge have the specified signed
                # perpendicular distance to its source line, including all arc chords.
                for moved in (front,np.roll(front,-1,axis=0)):
                    np.testing.assert_allclose(np.sum((moved-geo.P)*normals,axis=1),sign*.06,atol=1e-7)

    def test_rotated_face_preserves_plane_and_attributes(self):
        geo=rounded_face();geo.vertex['uv']=geo.P[:,:2].copy();geo.prim['material']=np.array([3],np.int32)
        geo.point['group_boundary']=np.ones(geo.npoints,np.int32)
        geo=run('transform',geo,rotate=[23,31,17],translate=[2,-3,1])
        out=run('inset',geo,amount=.05)
        np.testing.assert_array_equal(out.vertex['uv'][:geo.nvertices],geo.vertex['uv'])
        self.assertTrue(np.all(out.prim['material']==3));self.assertTrue(np.all(out.point['group_boundary']==1))
        p=geo.P;normal=np.cross(p[1]-p[0],p[-1]-p[0]);normal/=np.linalg.norm(normal)
        np.testing.assert_allclose((out.P-p[0])@normal,0,atol=1e-6)

    def test_connected_coplanar_faces_share_offset_vertices(self):
        geo=B.make(np.array([[0,0,0],[1,0,0],[2,0,0],[0,1,0],[1,1,0],[2,1,0]]),np.array([0,1,4,3,1,2,5,4]),np.array([0,4,8]))
        out=run('inset',geo,amount=.1)
        vp=out.topology.vertex_point
        self.assertEqual(vp[1],vp[4]);self.assertEqual(vp[2],vp[7])
        front=out.P[np.unique(vp[:8])]
        np.testing.assert_allclose(front.min(0),[.1,.1,0],atol=1e-7)
        np.testing.assert_allclose(front.max(0),[1.9,.9,0],atol=1e-7)

    def test_clear_errors_for_collapse_nonplanar_and_self_crossing(self):
        with self.assertRaisesRegex(ValueError,'collapses|reverses'):
            run('inset',rounded_face(),amount=.6)
        with self.assertRaisesRegex(ValueError,'planar'):
            run('inset',run('box'),group='0 2',amount=.1)
        geo=B.make(np.array([[0,0,0],[2,0,0],[0,2,0],[2,2,0],[0,1,0]]),np.arange(5),np.array([0,5]))
        with self.assertRaisesRegex(ValueError,'self-intersects|degenerate'):
            run('inset',geo,amount=.1)

    def test_planar_region_with_hole_offsets_both_boundaries(self):
        points=np.array([[-2,-2,0],[2,-2,0],[2,2,0],[-2,2,0],[-1,-1,0],[1,-1,0],[1,1,0],[-1,1,0]])
        faces=np.array([0,1,5,4,1,2,6,5,2,3,7,6,3,0,4,7])
        geo=B.make(points,faces,np.arange(0,17,4))
        out=run('inset',geo,amount=.1)
        front=out.P[out.topology.vertex_point[:16]]
        self.assertTrue(np.all(np.isclose(np.abs(front[:,:2]),1.9)|np.isclose(np.abs(front[:,:2]),1.1)))

    def test_numpy_accelerated_path_parity(self):
        from minihou import accel
        geo=rounded_face()
        fast=run('inset',geo,amount=.06)
        with accel.disabled():reference=run('inset',geo,amount=.06)
        np.testing.assert_array_equal(fast.P,reference.P)
        np.testing.assert_array_equal(fast.topology.vertex_point,reference.topology.vertex_point)
