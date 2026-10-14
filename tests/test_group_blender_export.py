"""Real Blender BOOLEAN POINT/FACE/CORNER/EDGE exports and incremental removal."""
import sys
import unittest
from pathlib import Path
import numpy as np
ROOT=Path(__file__).resolve().parents[1]
sys.path[:0]=[str(ROOT),str(ROOT/'blender_addon')]
try:import bpy
except ImportError:bpy=None
from minihou import Evaluator,Graph,Geometry,Topology
from minihou.nodes import group_rules as R
from minihou.link.diff import DiffState

@unittest.skipIf(bpy is None,'needs actual Blender')
class GroupBlenderExport(unittest.TestCase):
    def setUp(self):
        from minihou_link.protocol import Mirror
        self.mesh=bpy.data.meshes.new('GroupParityTest');self.mirror=Mirror();self.diff=DiffState();self.seq=0
        self.addCleanup(lambda:bpy.data.meshes.remove(self.mesh))
    def send(self,geo):
        from minihou_link.protocol import GeoUpdate
        from minihou_link.mesh_writer import apply_update
        head,arrays=self.diff.build(geo,self.seq);self.seq+=1
        update=GeoUpdate.from_message(head,arrays);self.mirror.apply(update)
        result=apply_update(self.mesh,self.mirror,update)
        self.assertEqual(result['warnings'],[])
        return result
    def vals(self,name):
        a=self.mesh.attributes[name];out=np.empty(len(a.data),bool);a.data.foreach_get('value',out)
        return out
    def test_four_domains_boolean_named_attributes(self):
        g=Evaluator().cook(Graph.from_dict({'nodes':{'b':{'type':'box'}},'display':'b'}))
        for d,n,ids in [('point','points',[0]),('prim','faces',[1]),('vertex','corners',[2]),('edge','edges',[0,1])]:
            m=np.zeros(R.count(g,d),bool);m[ids]=True;g=R.write(g,d,n,m)
        self.send(g)
        for name,domain,count in [('group_points','POINT',1),('group_faces','FACE',1),('group_corners','CORNER',1),('edge_group_edges','EDGE',2)]:
            a=self.mesh.attributes[name];self.assertEqual((a.domain,a.data_type),(domain,'BOOLEAN'));self.assertEqual(self.vals(name).sum(),count)
        # Attribute-only update changes EDGE membership without replacing mesh topology.
        m=np.zeros(R.count(g,'edge'),bool);m[-1]=True;g=R.write(g,'edge','edges',m)
        self.assertFalse(self.send(g)['rebuilt']);self.assertEqual(self.vals('edge_group_edges').sum(),1)
        g=R.delete(g,'edge','edges');self.send(g);self.assertNotIn('edge_group_edges',self.mesh.attributes)
    def test_curve_edge_group_export_and_corner_limit(self):
        g=Geometry(3,Topology(np.array([0,1,2],np.int32),np.array([0,3],np.int32),closed=np.array([False])));g.point['P']=np.array([[0,0,0],[1,0,0],[2,0,0]],np.float32)
        g=R.write(g,'edge','line',np.array([True,False]));self.send(g)
        self.assertEqual(len(self.mesh.edges),2);self.assertEqual(self.mesh.attributes['edge_group_line'].domain,'EDGE')
        self.assertEqual(self.vals('edge_group_line').sum(),1)
    def test_nonmanifold_half_edge_or(self):
        g=Geometry(5,Topology(np.array([0,1,2,1,0,3,0,1,4],np.int32),np.array([0,3,6,9],np.int32)));g.point['P']=np.array([[0,0,0],[1,0,0],[0,1,0],[0,-1,0],[0,0,1]],np.float32)
        from minihou import topo
        mask=np.all(topo.edges(g).points==[0,1],axis=1);g=R.write(g,'edge','seam',mask)
        self.send(g);self.assertEqual(self.vals('edge_group_seam').sum(),1)
