"""Real Chromium pointer/key workflows for Knife; no direct calls to its handlers."""
import json
import unittest
from tests import test_editor_e2e as f

@unittest.skipIf(f.sync_playwright is None,'playwright unavailable')
class KnifeUI(unittest.TestCase):
 @classmethod
 def setUpClass(cls):f.EditorE2E.setUpClass.__func__(cls)
 @classmethod
 def tearDownClass(cls):f.EditorE2E.tearDownClass.__func__(cls)
 setUp=f.EditorE2E.setUp
 tearDown=f.EditorE2E.tearDown
 node=f.EditorE2E.node
 box_scene=f.EditorE2E.box_scene
 view_xy=f.EditorE2E.view_xy
 def xy(self,p):
  r=self.page.locator('#pv-canvas').bounding_box();q=self.page.evaluate('(p)=>minihou.preview.project(p)',p)
  return r['x']+q[0],r['y']+q[1]
 def cutnode(self):
  self.assertTrue(f.until(lambda:any(n.type=='knife' for n in self.s.graph.nodes.values())))
  self.assertTrue(f.until(lambda:self.s.status()['ok']))
  return next(n for n in self.s.graph.nodes.values() if n.type=='knife')
 def test_paths_cutthrough_undo_and_single_graph_undo(self):
  self.box_scene();self.page.click('#pv-knife')
  for p in ([-.6,-.4,1],[.6,-.4,1]):self.page.mouse.click(*self.xy(p))
  self.page.mouse.click(*self.xy([.6,-.4,1]),button='right')
  for p in ([-.5,.3,1],[.5,.3,1]):self.page.mouse.click(*self.xy(p))
  self.page.keyboard.press('Control+z')
  self.page.mouse.click(*self.xy([.5,.5,1]))
  self.page.keyboard.press('c');self.assertTrue(self.page.locator('[data-setting=through]').is_checked())
  self.page.keyboard.press('Space');n=self.cutnode();cuts=json.loads(n.params['cuts'])
  self.assertEqual(len(cuts),2);self.assertTrue(all(s['through'] for s in cuts))
  self.page.click('#undo-btn');self.assertTrue(f.until(lambda:set(self.s.graph.nodes)=={'b','o'}))
 def test_selected_only_constraints_and_cancellation(self):
  self.box_scene();self.page.click('#preview [data-mode=face]');self.page.mouse.click(*self.view_xy('face',self.top));self.page.keyboard.press('Shift+k')
  self.assertTrue(self.page.locator('[data-setting=selected]').is_checked())
  self.page.mouse.click(*self.xy([-.4,-.4,1]));self.page.keyboard.press('a');self.page.keyboard.type('45')
  self.assertEqual(self.page.locator('[aria-label="Knife angle increment"]').input_value(),'45')
  self.page.mouse.move(*self.xy([.4,.4,1]));self.page.keyboard.press('a');self.page.keyboard.press('r')
  self.page.mouse.move(*self.xy([.3,.2,1]));self.page.keyboard.press('x');self.page.keyboard.press('x');self.page.keyboard.press('x')
  self.page.keyboard.press('s');self.page.keyboard.press('v');self.assertFalse(self.page.locator('[data-setting=xray]').is_checked())
  self.page.keyboard.press('Escape');self.assertEqual(set(self.s.graph.nodes),{'b','o'})
  self.page.keyboard.press('Shift+k')
  for p in ([-.4,-.4,1],[.4,.4,1]):self.page.mouse.click(*self.xy(p))
  self.page.keyboard.press('Enter');cuts=json.loads(self.cutnode().params['cuts']);self.assertEqual(cuts[0]['faces'],[self.top])
 def test_drag_stroke_and_midpoint_modifier(self):
  self.box_scene();self.page.click('#pv-knife')
  self.page.keyboard.down('Shift');self.page.mouse.click(*self.view_xy('edge',list(self.top_edges[0])));self.page.keyboard.up('Shift')
  self.page.mouse.move(*self.xy([.3,.2,1]));self.page.mouse.down();self.page.mouse.move(*self.xy([-.3,.5,1]),steps=10);self.page.mouse.up()
  self.page.keyboard.press('Enter');cuts=json.loads(self.cutnode().params['cuts']);self.assertGreater(len(cuts),1)

 def test_double_click_closes_loop_and_selects_cut_edges(self):
  self.box_scene();self.page.click('#pv-knife')
  for p in ([-.4,-.4,1],[.4,-.4,1],[.4,.4,1]):self.page.mouse.click(*self.xy(p))
  self.page.mouse.dblclick(*self.xy([-.4,.4,1]));self.page.keyboard.press('Enter')
  cuts=json.loads(self.cutnode().params['cuts']);self.assertEqual(len(cuts),4)
  self.page.wait_for_function('minihou.preview.selection().edges.length>0')
  self.assertEqual(self.page.evaluate('minihou.preview.selection().mode'),'edge')

 def test_changed_source_is_rejected_without_a_knife_node(self):
  self.box_scene();self.page.click('#pv-knife')
  for p in ([-.4,-.4,1],[.4,.4,1]):self.page.mouse.click(*self.xy(p))
  self.s.edit([{'op':'set','id':'b','params':{'size':[3,2,2]}}])
  self.page.wait_for_function('minihou.S.graph.nodes.b.params.size[0]===3')
  self.page.keyboard.press('Enter')
  self.page.wait_for_function("document.querySelector('.pv-drag-feedback').textContent.includes('Geometry changed')")
  self.assertEqual(set(self.s.graph.nodes),{'b','o'});self.page.keyboard.press('Escape')
