import unittest
import numpy as np
from minihou import Geometry, topo, edges
from minihou.geometry import Topology
from minihou.sdk import REGISTRY


def run(node_type, *inputs, **params):
    nt = REGISTRY.get(node_type)
    out = nt.fn(*inputs, **nt.coerce_params(params))
    out.validate()
    return out


def mesh(points, faces, closed=None):
    t = Topology.from_polys(faces, closed)
    return Geometry.from_arrays(np.array(points, np.float32), t.vertex_point, t.prim_offsets, closed=t.closed)


def profiles():
    return mesh([[0,0,0],[1,0,0],[1,1,0],[0,1,0],
                 [0,0,2],[1,0,2],[1,1,2],[0,1,2]], [[3,2,1,0],[4,5,6,7]])


class EdgeToolsTests(unittest.TestCase):
    def test_groups_open_curves_and_patterns(self):
        g = mesh([[0,0,0],[1,0,0],[2,0,0]], [[0,1,2]], [False])
        g.freeze()
        out = run('edge_group', g, name='one', pattern='p0-1')
        np.testing.assert_array_equal(edges.mask(out, 'one'), [True,False])
        np.testing.assert_array_equal(edges.mask(out, '^one'), [False,True])
        self.assertEqual(out.vertex['edge_group_one'][-1], 0)
        self.assertNotIn('edge_group_one', g.vertex)
        pts = run('edge_group_promote', out, pattern='one')
        np.testing.assert_array_equal(pts.point['group_edge_points'], [1,1,0])
        for pattern in ['0-2:0','missing','p0-2']:
            with self.assertRaises(ValueError):
                edges.mask(g, pattern)

    def test_bridge_winding_segments_and_attributes(self):
        g = profiles()
        g.point['weight'] = np.arange(8, dtype=np.float32)
        g.prim['material_index'] = [2,3]
        g.vertex['uv'] = g.P[g.topology.vertex_point,:2]
        g.detail['materials'] = ['a','b','c','d']
        g = run('edge_group', g, name='bottom', method='primitives', pattern='0')
        g = run('edge_group', g, name='top', method='primitives', pattern='1')
        before = g.P.copy()
        g.freeze()
        out = run('polybridge', g, source='bottom', destination='top', segments=3)
        self.assertEqual((out.npoints,out.nprims), (16,14))
        self.assertTrue(np.all(topo.edges(out).count == 2))
        vp, nxt, e = out.topology.vertex_point, topo.next_vertex(out), topo.edges(out)
        for i in range(len(e.points)):
            vs = np.flatnonzero(e.of_vertex == i)
            self.assertEqual(vp[vs[0]], vp[nxt[vs[1]]])
        np.testing.assert_array_equal(g.P, before)
        np.testing.assert_array_equal(out.vertex['uv'][:g.nvertices], g.vertex['uv'])
        self.assertEqual(edges.mask(out,'bottom').sum(),4)
        self.assertEqual(out.detail, g.detail)
        self.assertTrue(np.isfinite(out.P).all())

    def test_bridge_invalid(self):
        g = profiles()
        for a,b in [('', ''), ('p0-1','p0-3'), ('p0-1 p1-2','p4-5'), ('p0-1 p2-3','p4-5 p6-7')]:
            with self.assertRaises(ValueError):
                run('polybridge', g, source=a, destination=b)

    def test_dissolve_area_groups_seams_and_immutability(self):
        g = mesh([[0,0,0],[1,0,0],[1,1,0],[0,1,0]], [[0,1,2],[0,2,3]])
        g.vertex['uv'] = g.P[g.topology.vertex_point,:2]
        g.prim['material_index'] = [2,2]
        g.prim['group_side'] = [0,1]
        g = run('edge_group',g,name='border',method='boundary')
        g.freeze()
        out = run('dissolve', g, group='p0-2')
        self.assertEqual(out.topology.polys(),[[0,1,2,3]])
        self.assertEqual(out.prim['group_side'][0],1)
        self.assertEqual(edges.mask(out,'border').sum(),4)
        self.assertEqual(g.nprims,2)
        for attr in ['uv','material_index']:
            bad = g.copy()
            table = bad.vertex if attr == 'uv' else bad.prim
            a = table[attr].copy()
            a[-1] += 4
            if attr == 'uv':
                a[3] += 1
            table[attr] = a
            with self.assertRaisesRegex(ValueError,'seam'):
                run('dissolve',bad,group='p0-2')
        with self.assertRaisesRegex(ValueError,'interior'):
            run('dissolve',g,group='p0-1')

    def test_empty_and_group_combine(self):
        g = Geometry()
        g.point['P'] = np.empty((0,3),np.float32)
        out = run('edge_group',g)
        self.assertEqual(len(edges.mask(out,'*')),0)
        self.assertIs(run('dissolve',g),g)
        g = profiles()
        a = run('edge_group',g,name='sel',pattern='0')
        b = run('edge_group',a,name='sel',pattern='1',combine='union')
        self.assertEqual(edges.mask(b,'sel').sum(),2)

    def test_dissolve_rejects_closed_shell_holes_and_nonmanifold(self):
        # Dissolving every edge of a closed shell cannot make one polygon.
        with self.assertRaisesRegex(ValueError,'simple closed boundary'):
            run('dissolve',run('box'),group='*',angle=180,preserve_seams=False)
        ring=mesh([[x,y,0] for x,y in [(0,0),(3,0),(3,3),(0,3),(1,1),(2,1),(2,2),(1,2)]],
                  [[0,1,5,4],[1,2,6,5],[2,3,7,6],[3,0,4,7]])
        with self.assertRaisesRegex(ValueError,'simple closed boundary'):
            run('dissolve',ring,group='p0-4 p1-5 p2-6 p3-7')
        bad=mesh([[0,0,0],[1,0,0],[0,1,0],[0,-1,0],[0,0,1]],[[0,1,2],[1,0,3],[0,1,4]])
        with self.assertRaisesRegex(ValueError,'manifold interior'):
            run('dissolve',bad,group='p0-1',angle=180)

    def test_bridge_open_chains_and_rigid_equivariance(self):
        g=mesh([[0,0,0],[1,0,0],[2,0,0],[0,0,2],[1,0,2],[2,0,2]],
               [[0,1,2],[3,4,5]],[False,False])
        g=run('edge_group',g,name='a',method='primitives',pattern='0')
        g=run('edge_group',g,name='b',method='primitives',pattern='1')
        a=run('polybridge',g,source='a',destination='b',segments=2)
        self.assertEqual(a.nprims,6)
        self.assertEqual(a.topology.closed_mask().tolist(),[False,False,True,True,True,True])
        moved=run('transform',g,translate=[3,2,4],rotate=[20,30,40])
        b=run('polybridge',moved,source='a',destination='b',segments=2)
        expected=run('transform',a,translate=[3,2,4],rotate=[20,30,40])
        np.testing.assert_allclose(b.P,expected.P,atol=1e-6)
        flat=g.copy(); flat.point['P']=g.P.copy(); flat.point['P'][3:]=g.P[:3]
        with self.assertRaisesRegex(ValueError,'collapsed'):
            run('polybridge',flat,source='a',destination='b')


if __name__ == '__main__':
    unittest.main()
