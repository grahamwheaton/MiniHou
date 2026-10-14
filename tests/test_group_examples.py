"""Shipped group workflows stay loadable, cookable, and useful."""
import unittest
from pathlib import Path
from minihou import Graph,Evaluator
from minihou.nodes import group_rules as R
ROOT=Path(__file__).resolve().parents[1]
class GroupExamples(unittest.TestCase):
 def test_examples_load_cook_and_create_named_groups(self):
  expected={'core':('combine','prim','result'),'helpers':('near','point','near'),'copy-transfer':('transfer','prim','near_topology_face'),'path-boundary':('promote','point','route_points'),'names':('delete','prim','part_piece0')}
  for slug,(node,dom,name) in expected.items():
   with self.subTest(example=slug):
    graph=Graph.load(ROOT/'examples'/'graphs'/('group-parity-'+slug+'.json'))
    ev=Evaluator();self.addCleanup(ev.close);out=ev.cook(graph,node);out.validate()
    self.assertIsNotNone(R.membership(out,dom,name));self.assertTrue(R.membership(out,dom,name).any())
