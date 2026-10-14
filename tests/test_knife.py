import json,unittest
import numpy as np
from minihou import Graph,Evaluator
from tests.test_bevel import closed_shell,volume
class KnifeTests(unittest.TestCase):
 def run_cut(self,segments):
  return Evaluator().cook(Graph.from_dict({'nodes':{'b':{'type':'box'},'k':{'type':'knife','inputs':{'geo':'b'},'params':{'cuts':json.dumps(segments)}}},'display':'k'}))
 def segment(self,a,b,through=True):return {'a':a,'b':b,'matrix':np.eye(4).flatten(order='F').tolist(),'through':through}
 def test_cut_through_closed_box_preserves_volume(self):
  g=self.run_cut([self.segment([-.75,0],[.75,0])]);g.validate();closed_shell(self,g);self.assertAlmostEqual(volume(g),1.,places=6);self.assertGreater(g.npoints,8)
 def test_visible_only_leaves_back_face_uncut(self):
  g=self.run_cut([self.segment([-.75,0],[.75,0],False)]);closed_shell(self,g)
  new=g.P[8:];self.assertTrue(np.all(new[:,2]<0))
 def test_interior_loop_and_crossing_segments(self):
  p=[[-.2,-.2],[.2,-.2],[.2,.2],[-.2,.2],[-.2,-.2]]
  seg=[self.segment(a,b) for a,b in zip(p,p[1:])]
  seg.append(self.segment([-.4,0],[.4,0]))
  g=self.run_cut(seg);g.validate();closed_shell(self,g);self.assertAlmostEqual(volume(g),1.,places=6)
 def test_interior_dangling_cut(self):
  g=self.run_cut([self.segment([-.13,.11],[.21,.17])]);g.validate();closed_shell(self,g);self.assertGreater(g.npoints,8)

 def test_random_crossings_remain_watertight(self):
  rng=np.random.default_rng(4)
  for k in range(40):
   cuts=[self.segment(*rng.uniform(-.7,.7,(2,2)).tolist(),through=bool(k%2)) for _ in range(3)]
   g=self.run_cut(cuts);g.validate();closed_shell(self,g);self.assertAlmostEqual(volume(g),1.,places=5)
 def test_existing_diagonal_materialises_as_edge(self):
  from minihou import topo
  g=self.run_cut([self.segment([-.5,-.5],[.5,.5])]);closed_shell(self,g)
  # The cut must survive even if it exactly coincides with the internal triangulation.
  found=False
  for a,b in topo.edges(g).points:
   p,q=g.P[[a,b]]
   if abs(p[2]-q[2])<1e-6 and abs(p[0]-q[0])>.9 and abs(p[1]-q[1])>.9:found=True
  self.assertTrue(found)
 def test_selected_only_and_no_hit(self):
  seg=self.segment([-.75,0],[.75,0]);seg['faces']=[]
  self.assertEqual(self.run_cut([seg]).npoints,8)
  self.assertEqual(self.run_cut([self.segment([2,2],[3,3])]).npoints,8)
 def test_perspective_and_rotated_view(self):
  a=.37;c,s=np.cos(a),np.sin(a);m=np.array([[c,0,s,0],[0,1,0,0],[-s,0,c,0],[0,0,.25,1]])
  cut=self.segment([-.8,-.12],[.8,.21]);cut['matrix']=m.flatten(order='F').tolist()
  g=self.run_cut([cut]);g.validate();closed_shell(self,g);self.assertAlmostEqual(volume(g),1.,places=5)
 def test_attributes_preserve_affine_fields_and_discrete_materials(self):
  from minihou.nodes.knife import knife
  g=Evaluator().cook(Graph.from_dict({'nodes':{'b':{'type':'box'}},'display':'b'}))
  g=g.copy();g.point['field']=g.P.copy();g.vertex['uv']=g.P[g.topology.vertex_point,:2].copy();g.prim['material']=np.arange(g.nprims,dtype=np.int32)
  out=knife(g,json.dumps([self.segment([-.7,.13],[.7,.13])]))
  np.testing.assert_allclose(out.point['field'],out.P,atol=1e-6)
  np.testing.assert_allclose(out.vertex['uv'],out.P[out.topology.vertex_point,:2],atol=1e-6)
  self.assertEqual(set(out.prim['material']),set(g.prim['material']))
 def test_invalid_cut_transaction_preserves_graph(self):
  from minihou.edit import apply_ops,EditError
  g=Graph.from_dict({'nodes':{'b':{'type':'box'}},'display':'b'});before=g.to_dict()
  with self.assertRaises(EditError):apply_ops(g,[{'op':'knife','after':'b','params':{'cuts':'invalid'}}])
  self.assertEqual(g.to_dict(),before)
 def test_multi_object_cut_is_one_atomic_transaction(self):
  from minihou.edit import apply_ops
  g=Graph.from_dict({'nodes':{'b':{'type':'box'},'b2':{'type':'box','params':{'center':[2,0,0]},'display_extra':True}},'display':'b'})
  out,result=apply_ops(g,[{'op':'knife','after':'b','merge_visible':True,'params':{'cuts':json.dumps([self.segment([-1,0],[3,0])])}}])
  mesh=Evaluator().cook(out,result[0]['id']);closed_shell(self,mesh);self.assertAlmostEqual(volume(mesh),2.,places=5);self.assertGreater(mesh.npoints,16)
  self.assertEqual(len(g.nodes),2)

 def test_concave_polygon_area_and_cut_group(self):
  from minihou.nodes import _build as B
  from minihou.nodes.knife import knife
  from minihou import edges
  points=np.array([[0,0,0],[2,0,0],[2,1,0],[1,1,0],[1,2,0],[0,2,0]],float)
  g=B.make(points,np.arange(6),np.array([0,6]))
  out=knife(g,json.dumps([self.segment([-.1,.5],[2.1,.5])]))
  area=0
  for f in range(out.nprims):
   p=out.P[out.topology.vertex_point[out.topology.prim_offsets[f]:out.topology.prim_offsets[f+1]]]
   area+=np.sum(np.cross(p,np.roll(p,-1,axis=0)),axis=0)[2]/2
  self.assertAlmostEqual(area,3.,places=6);self.assertGreater(np.count_nonzero(edges.values(out,'edge_group_knife')),0)
 def test_split_source_edges_keep_crease_weights(self):
  from minihou.nodes.knife import knife
  from minihou import edges,topo
  g=Evaluator().cook(Graph.from_dict({'nodes':{'b':{'type':'box'}},'display':'b'}))
  g=edges.write(g,edges.CREASE,np.ones(len(topo.edges(g).points),np.float32))
  out=knife(g,json.dumps([self.segment([-.75,0],[.75,0])]))
  weights=edges.values(out,edges.CREASE)
  self.assertGreater(np.count_nonzero(weights),12)
  self.assertTrue(np.all(weights[edges.values(out,'edge_group_knife')>0]==0))

 def test_seven_segment_snapped_bevel_path_degenerate_tessellation(self):
  # Reproduced None / float: segment six retraces a tessellation diagonal and
  # leaves a zero-area ear. Subsequent cuts must preserve all earlier endpoints.
  from tests.test_nodes import run
  from minihou.nodes.knife import knife
  from minihou import edges
  source=run('polybevel',run('box'),edges='0-1 0-2 1-3 2-3',divisions=8,distance=.2)
  path=[[-.49666106700897217,-.49666106700897217],[.3736189305782318,.3736189305782318],[.48595765233039856,-.48595765233039856],[-.29412065636450413,.1783291491516698],[-.46739909052848816,.46739909052848816],[-.30000001192092896,-.30000001192092896],[.30000001192092896,.30000001192092896],[.4094419479370117,.4094419479370117]]
  for through in (False,True):
   out=knife(source,json.dumps([self.segment(a,b,through) for a,b in zip(path,path[1:])]))
   out.validate();closed_shell(self,out)
   self.assertAlmostEqual(volume(out),volume(source),places=6)
   self.assertGreater(np.count_nonzero(edges.values(out,'edge_group_knife')),7)
   self.assertTrue(np.isfinite(out.P).all())

 def test_home_rounded_outline_exact_snapped_path(self):
  from pathlib import Path
  from minihou.nodes import _build as B
  from minihou.nodes.knife import knife
  from minihou import edges,topo
  data=json.loads((Path(__file__).parent/'fixtures/knife_rounded_home_repro.json').read_text(encoding='utf-8'))
  points=np.asarray(data['outline']);source=B.make(points,np.arange(len(points)),np.array([0,len(points)]))
  source.point['field']=source.P.copy();source.vertex['uv']=source.P[:,:2].copy()
  source.prim['material']=np.array([7],np.int32);source.prim['group_face']=np.array([1],np.int32)
  out=knife(source,json.dumps(data['cuts']));out.validate()
  def area(g):
   total=0.
   for start,end in zip(g.topology.prim_offsets,g.topology.prim_offsets[1:]):
    q=g.P[g.topology.vertex_point[start:end]].astype(float)
    signed=np.sum(np.cross(q,np.roll(q,-1,axis=0)),axis=0)[2]/2
    self.assertGreater(signed,0);total+=signed
   return total
  self.assertAlmostEqual(area(out),area(source),places=6)
  np.testing.assert_allclose(out.point['field'],out.P,atol=1e-7)
  np.testing.assert_allclose(out.vertex['uv'],out.P[out.topology.vertex_point,:2],atol=1e-7)
  self.assertTrue(np.all(out.prim['material']==7));self.assertTrue(np.all(out.prim['group_face']==1))
  cut_edges=topo.edges(out).points[edges.values(out,'edge_group_knife')>0]
  # Every intended stroke is represented continuously by tagged result edges,
  # even if tessellation subdivides it into several shorter segments.
  for cut in data['cuts']:
   a,b=np.asarray(cut['a']),np.asarray(cut['b']);delta=b-a;length=np.linalg.norm(delta)
   intervals=[]
   for pair in cut_edges:
    q=out.P[pair,:2].astype(float);t=(q-a)@delta/(length*length)
    if np.max(np.linalg.norm(q-a-t[:,None]*delta,axis=1))<2e-7:
     lo,hi=sorted(t);lo=max(0.,lo);hi=min(1.,hi)
     if hi>lo:intervals.append((lo,hi))
   covered=0.
   for lo,hi in sorted(intervals):
    self.assertLessEqual(lo,covered+2e-6);covered=max(covered,hi)
   self.assertGreaterEqual(covered,1-2e-6)
