"""Native modelling tests run real Blender when installed; standalone transforms always run."""
import json
import shutil
import unittest
from unittest.mock import patch

import numpy as np

from minihou import Geometry, Graph, Evaluator, edges, topo
from minihou.blender_mesh import encode, execute
from minihou.blender_mesh_catalog import OPERATORS
from minihou.blender_setup import find_blender
from tests.helpers import quad_geo
from tests.test_nodes import run


def geometry(points, *args, **kwargs):
    return Geometry.from_arrays(np.asarray(points, np.float32), *args, **kwargs)


class StandaloneTransforms(unittest.TestCase):
    def test_to_sphere_selected_points_and_center(self):
        g = geometry([[2,0,0],[0,1,0],[0,0,0],[-3,0,0]])
        g.point['group_sel'] = np.array([1,1,1,0],np.int32)
        out = run('to_sphere',g,group='sel',radius=2,use_centroid=False)
        np.testing.assert_allclose(out.P,[[2,0,0],[0,2,0],[0,0,0],[-3,0,0]])
        np.testing.assert_allclose(g.P,[[2,0,0],[0,1,0],[0,0,0],[-3,0,0]])

    def test_shear_pivot_group_and_normals(self):
        g=quad_geo();g.point['N']=np.tile([0.,0.,1.],(4,1))
        out=run('shear',g,group='2',factor=2,center=[0,.5,0])
        self.assertNotIn('N',out.point)
        np.testing.assert_allclose(out.P,[[0,0,0],[1,0,0],[2,1,0],[0,1,0]])
        self.assertIn('N',g.point)
        with self.assertRaises(ValueError): run('shear',g,axis='x',direction='x')

    def test_backend_rejects_arbitrary_operator_json_and_missing_runtime(self):
        with self.assertRaisesRegex(ValueError,'Unsupported'): execute(quad_geo(),'wm.open_mainfile')
        with self.assertRaisesRegex(ValueError,'JSON'): run('blender_mesh',quad_geo(),options='__import__("os")')
        with patch('minihou.blender_mesh.find_blender',return_value=None):
            with self.assertRaisesRegex(ValueError,'requires Blender'): execute(quad_geo(),'poke')
        self.assertEqual(len(OPERATORS),len(set(OPERATORS)))

    def test_nonexecutable_runtime_has_an_actionable_error(self):
        with patch('minihou.blender_mesh.find_blender', return_value='/missing/blender'):
            with self.assertRaisesRegex(ValueError,'Cannot launch Blender'): execute(quad_geo(), 'poke')

    def test_cancel_terminates_the_disposable_process(self):
        from minihou.cancellation import CookCancelled
        from unittest.mock import MagicMock
        process = MagicMock()
        process.poll.return_value = None
        with patch('minihou.blender_mesh.find_blender', return_value='/mock/blender'), \
             patch('minihou.blender_mesh.subprocess.Popen', return_value=process), \
             patch('minihou.blender_mesh.time.sleep', side_effect=CookCancelled):
            with self.assertRaises(CookCancelled): execute(quad_geo(), 'poke')
        process.kill.assert_called_once()
        process.wait.assert_called_once()

    def test_input_selection_and_open_attribute_loss_rejected(self):
        q=quad_geo();q.vertex['edge_group_pick']=np.array([1,0,0,0],np.int32)
        request=encode(q,'edge','pick')
        self.assertEqual(len(request['selected']),1)
        curve=geometry([[0,0,0],[1,0,0]],[0,1],[0,2],closed=[False])
        curve.prim['label']=np.array([1],np.int32)
        with self.assertRaisesRegex(ValueError,'open curves'): encode(curve,'edge','')


@unittest.skipUnless(find_blender(),'requires a native Blender executable')
class NativeBlenderModelling(unittest.TestCase):
    def test_poke_attributes_and_input_immutability(self):
        g=quad_geo();g.point['weight']=np.arange(4,dtype=np.float32)
        g.point['group_keep']=np.ones(4,np.int32);g.vertex['uv']=g.P[:,:2].copy()
        g.prim['material_index']=np.array([2],np.int32);g.detail['materials']=['a','b','c']
        out=run('poke_faces',g)
        self.assertEqual((out.npoints,out.nprims),(5,4))
        self.assertEqual(out.vertex['uv'].shape,(12,2))
        np.testing.assert_allclose(out.point['weight'][-1],1.5)
        np.testing.assert_array_equal(out.prim['material_index'],[2]*4)
        self.assertEqual(out.detail['materials'],g.detail['materials'])
        self.assertEqual((g.npoints,g.nprims),(4,1))
        self.assertIn('native_blender_version',out.detail)

    def test_solidify_closed_manifold_and_thickness(self):
        out=run('solidify',quad_geo(),thickness=.25)
        self.assertEqual((out.npoints,out.nprims),(8,6))
        np.testing.assert_allclose(np.ptp(out.P[:,2]),.25,atol=1e-6)
        self.assertTrue((topo.edges(out).count==2).all())

    def test_inset_and_extrude_are_selection_aware(self):
        g=run('merge',[quad_geo(),quad_geo(x0=3)])
        inset=run('native_inset',g,group='0',amount=.2)
        self.assertEqual(inset.nprims,6)
        self.assertTrue(any(np.allclose(p,[3,0,0]) for p in inset.P))
        extrude=run('native_extrude',g,group='0',offset=[0,0,1])
        self.assertEqual(extrude.nprims,7)
        self.assertTrue((extrude.P[extrude.P[:,0]>=3,2]==0).all())
        self.assertAlmostEqual(float(extrude.P[:,2].max()),1)

    def test_triangles_to_quads_respects_uv_seams(self):
        g=geometry([[0,0,0],[1,0,0],[1,1,0],[0,1,0]],[0,1,2,0,2,3],[0,3,6])
        g.vertex['uv']=np.array([[0,0],[1,0],[1,1],[2,0],[2,1],[0,1]],np.float32)
        self.assertEqual(run('triangles_to_quads',g,compare_uvs=False).nprims,1)
        self.assertEqual(run('triangles_to_quads',g,compare_uvs=True).nprims,2)

    def test_merge_split_and_make_faces(self):
        points=geometry([[0,0,0],[1,0,0],[1,1,0],[0,1,0],[0,0,0]])
        welded=run('merge_by_distance',points,distance=.01)
        self.assertEqual(welded.npoints,4)
        face=run('make_faces',welded)
        self.assertEqual(face.nprims,1)
        tris=run('divide',face)
        split=run('split_geometry',tris,group='0')
        self.assertGreater(split.npoints,tris.npoints)
        merge=run('merge_vertices',quad_geo(),group='0 1')
        self.assertEqual(merge.npoints,3)

    def test_native_edge_flags_and_edge_group_preservation(self):
        g=quad_geo();g=edges.write(g,'edge_group_keep',np.array([1,0,0,0],np.int32))
        out=run('blender_mesh',g,operation='mark_seam',domain='edge',group='keep')
        np.testing.assert_array_equal(edges.values(out,'edge_group_keep'),edges.values(g,'edge_group_keep'))
        np.testing.assert_array_equal(edges.values(out,'edge_group_seam'),edges.values(g,'edge_group_keep'))

    def test_connect_edge_split_dissolve_wireframe_and_spin(self):
        connected=run('connect_vertices',quad_geo(),group='0 2')
        self.assertEqual(connected.nprims,2)
        split=run('split_edges',connected,group='p0-2')
        self.assertEqual(split.npoints,6)
        self.assertTrue((topo.edges(split).count==1).all())
        dissolved=run('limited_dissolve',connected)
        self.assertEqual(dissolved.nprims,1)
        wire=run('native_wireframe',run('box'))
        self.assertGreater(wire.nprims,6)
        wire.validate()
        profile=geometry([[1,0,0],[1,0,1]],[0,1],[0,2],closed=[False])
        spun=run('spin',profile,steps=4,angle=90)
        self.assertEqual(spun.nprims,4)
        np.testing.assert_allclose(np.linalg.norm(spun.P[:,:2],axis=1),1,atol=1e-6)

    def test_grid_fill_symmetrize_bevel_and_bisect(self):
        wire=geometry([[0,0,0],[1,0,0],[1,1,0],[0,1,0]],[0,1,2,3,0],[0,5],closed=[False])
        filled=run('grid_fill',wire)
        self.assertEqual(filled.nprims,1)
        bevel=run('native_bevel',run('box'),amount=.1,segments=2)
        self.assertGreater(bevel.nprims,6)
        bisect=run('native_bisect',run('box'),normal=[1,0,0],fill=True,clear_inner=True)
        self.assertTrue((topo.edges(bisect).count==2).all())
        self.assertLessEqual(float(np.ptp(bisect.P[:,0])),.50001)
        sym=run('symmetrize',run('box'),direction='NEGATIVE_X')
        self.assertTrue((topo.edges(sym).count==2).all())
        for p in sym.P:
            self.assertTrue(np.any(np.linalg.norm(sym.P-p*np.array([-1,1,1]),axis=1)<1e-6))

    def test_documented_native_example_cooks_and_caches(self):
        from pathlib import Path
        graph=Graph.load(Path(__file__).resolve().parents[1]/'examples'/'blender_modelling.json')
        self.assertTrue(graph.interface())
        ev=Evaluator()
        out=ev.cook(graph)
        out.validate()
        self.assertEqual(out.nprims,9)
        self.assertIn('N',out.point)
        ev.cook(graph)
        self.assertIn('normal',ev.last_stats.cached)

    def test_selected_result_groups_drive_the_next_native_operation(self):
        g=run('native_inset',quad_geo(),amount=.15)
        self.assertEqual(int(g.prim['group_native_selection'].sum()),1)
        extrude=run('native_extrude',g,group='native_selection',offset=[0,0,.4])
        self.assertAlmostEqual(float(extrude.P[:,2].max()),.4)
        self.assertEqual(extrude.nprims,9)
        unchanged=run('native_inset',quad_geo(),result_group='')
        self.assertNotIn('group_native_selection',unchanged.prim)

    def test_advanced_dispatch_and_bad_parameters(self):
        out=run('blender_mesh',quad_geo(),operation='subdivide',domain='face',options=json.dumps({'number_cuts':1}))
        self.assertEqual(out.nprims,4)
        with self.assertRaisesRegex(ValueError,'failed'): run('blender_mesh',quad_geo(),operation='poke',options='{"not_a_parameter":1}')
        graph=Graph.from_dict({'nodes':{'q':{'type':'grid','params':{'columns':2,'rows':2}},'p':{'type':'poke_faces','inputs':{'geo':'q'}}},'display':'p'})
        ev=Evaluator();ev.cook(graph);ev.cook(graph)
        self.assertIn('p',ev.last_stats.cached)
