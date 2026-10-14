"""Actual Blender mesh/protocol smoke tests, run by tools/blender_modeling_smoke.py."""
import pathlib
import os
import tempfile
import time
import struct
import sys
import unittest
import numpy as np

ROOT=pathlib.Path(__file__).resolve().parents[1]
sys.path[:0]=[str(ROOT),str(ROOT/'blender_addon')]
try:
    import bpy
except ImportError:
    bpy=None

from minihou import Evaluator,Graph
from minihou.link.diff import DiffState
from minihou.link.protocol import encode_message,decode_payload
from tests.test_edge_tools import profiles,run


@unittest.skipIf(bpy is None,'needs actual Blender')
class BlenderModelingTests(unittest.TestCase):
    def setUp(self):
        from minihou_link.protocol import Mirror
        self.mesh=bpy.data.meshes.new('ModelingSmoke')
        self.obj=bpy.data.objects.new('ModelingSmoke',self.mesh)
        bpy.context.scene.collection.objects.link(self.obj)
        self.mirror=Mirror()
        self.diff=DiffState()
        self.seq=0

    def tearDown(self):
        bpy.data.objects.remove(self.obj,do_unlink=True)
        bpy.data.meshes.remove(self.mesh)

    def send(self,geo):
        from minihou_link.protocol import GeoUpdate
        from minihou_link.mesh_writer import apply_update
        header,arrays=self.diff.build(geo,self.seq)
        self.seq+=1
        payload=b''.join(bytes(part) for part in encode_message(header,arrays))
        _,hlen,_=struct.unpack('<4sIQ',payload[:16])
        header,arrays=decode_payload(payload[16:],hlen)
        update=GeoUpdate.from_message(header,arrays)
        self.mirror.apply(update)
        status=apply_update(self.mesh,self.mirror,update)
        self.assertEqual(status['warnings'],[])
        bpy.context.view_layer.update()
        self.assertEqual(len(self.mesh.vertices),geo.npoints)
        self.assertEqual(len(self.mesh.polygons),geo.nprims)
        evaluated=self.obj.evaluated_get(bpy.context.evaluated_depsgraph_get()).data
        points=np.empty((len(evaluated.vertices),3),np.float32)
        evaluated.vertices.foreach_get('co',points.ravel())
        np.testing.assert_allclose(points,geo.P,atol=1e-5)
        return status

    def test_all_workflows_through_protocol_and_evaluated_mesh(self):
        for path in sorted((ROOT/'examples'/'graphs').glob('modeling_*.json')):
            with self.subTest(workflow=path.name):
                geo=Evaluator().cook(Graph.load(path))
                self.send(geo)
                self.assertFalse(self.mesh.validate(verbose=False),'Blender found invalid topology')

    def test_bridge_uv_material_edge_groups_and_position_update(self):
        g=profiles()
        g.vertex['uv']=g.P[g.topology.vertex_point,:2]
        g.prim['material_index']=[2,3]
        g.point['group_keep']=np.ones(g.npoints,np.int32)
        g=run('edge_group',g,name='a',method='primitives',pattern='0')
        g=run('edge_group',g,name='b',method='primitives',pattern='1')
        geo=run('polybridge',g,source='a',destination='b',segments=3)
        self.assertTrue(self.send(geo)['rebuilt'])
        uv=np.empty((geo.nvertices,2),np.float32)
        self.mesh.uv_layers['UVMap'].data.foreach_get('uv',uv.ravel())
        np.testing.assert_allclose(uv,geo.vertex['uv'])
        material=np.empty(geo.nprims,np.int32)
        self.mesh.polygons.foreach_get('material_index',material)
        np.testing.assert_array_equal(material,geo.prim['material_index'])
        attr=self.mesh.attributes['edge_group_a']
        self.assertEqual((attr.domain,attr.data_type),('EDGE','BOOLEAN'))
        from minihou import edges,topo
        source_values=edges.values(geo,'edge_group_a')!=0
        by_pair={tuple(pair):value for pair,value in zip(topo.edges(geo).points,source_values)}
        expected=[by_pair[tuple(sorted(edge.vertices))] for edge in self.mesh.edges]
        data=np.empty(len(self.mesh.edges),bool)
        attr.data.foreach_get('value',data)
        np.testing.assert_array_equal(data,expected)
        self.assertEqual(self.mesh.attributes['group_keep'].data_type,'BOOLEAN')
        moved=run('transform',geo,translate=[3,4,5])
        self.assertFalse(self.send(moved)['rebuilt'])


@unittest.skipIf(bpy is None or not os.environ.get('MINIHOU_ENGINE_PYTHON'),
                 'needs Blender and MINIHOU_ENGINE_PYTHON')
class BridgeLiveLinkTests(unittest.TestCase):
    def test_bridge_engine_socket_proxy_and_modifier_recook(self):
        import minihou_link
        from minihou_link import proxy,runtime
        from tests.test_blender_addon import Engine,evaluated_positions
        with tempfile.TemporaryDirectory() as directory:
            path=pathlib.Path(directory)/'bridge.json'
            path.write_text((ROOT/'examples'/'graphs'/'modeling_bridge.json').read_text(encoding='utf-8'),encoding='utf-8')
            engine=Engine(path)
            self.addCleanup(engine.stop)
            minihou_link.register()
            self.addCleanup(minihou_link.unregister)
            rt=runtime.get_runtime()
            rt.connect('127.0.0.1',engine.port)
            def pump(pred):
                until=time.monotonic()+30
                while time.monotonic()<until:
                    rt.tick()
                    bpy.context.view_layer.update()
                    if pred():
                        return
                    time.sleep(.01)
                self.fail(f'live bridge timeout: {rt.last_error}')
            pump(lambda: rt.applied_count>=1)
            graph=Graph.load(path)
            expected=Evaluator().cook(graph)
            obj=bpy.data.objects[proxy.PROXY_OBJECT]
            np.testing.assert_allclose(evaluated_positions(obj),expected.P,atol=1e-5)
            modifier=obj.modifiers[proxy.MODIFIER]
            previous=rt.applied_count
            proxy.set_input(modifier,proxy.mapping(modifier.node_group)['Segments'],2)
            bpy.context.view_layer.update()
            pump(lambda: rt.applied_count>previous and len(evaluated_positions(obj))==36)
            graph.set_exposed('Segments',2)
            expected=Evaluator().cook(graph)
            np.testing.assert_allclose(evaluated_positions(obj),expected.P,atol=1e-5)
            self.assertTrue(rt.last_timings['rebuilt'])
            output=bpy.data.objects[proxy.OUTPUT_OBJECT].data
            self.assertIn('edge_group_bottom',output.attributes)


if __name__=='__main__':
    unittest.main()
