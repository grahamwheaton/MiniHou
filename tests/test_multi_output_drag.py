"""Multiple selected output wires driven through real Chromium."""
import unittest
from tests import test_editor_e2e as f

@unittest.skipIf(f.sync_playwright is None, 'playwright unavailable')
class MultiOutputDragTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls): f.EditorE2E.setUpClass.__func__(cls)
    @classmethod
    def tearDownClass(cls): f.EditorE2E.tearDownClass.__func__(cls)
    tearDown=f.EditorE2E.tearDown
    node=f.EditorE2E.node
    center=f.EditorE2E.center
    empty_spot=f.EditorE2E.empty_spot
    def setUp(self):
        f.EditorE2E.setUp(self)
        graph={'version':1,'nodes':{'a':{'type':'box','pos':[0,0]},'b':{'type':'box','pos':[260,0]},'m':{'type':'merge','inputs':{'geos':['a']},'pos':[0,220]},'t':{'type':'transform','pos':[260,220]}},'display':'a'}
        self.s.edit([{'op':'replace','graph':graph}])
        self.page.wait_for_selector('.node[data-id="a"]')
        self.page.evaluate('minihou.frame()')
        self.node('a').locator('.title').click()
        self.node('b').locator('.title').click(modifiers=['Shift'])
        self.page.wait_for_function('minihou.S.sel.size===2')
    def start(self, target=None):
        self.page.mouse.move(*self.center(self.node('a').locator('.port.out')))
        self.page.mouse.down()
        self.page.mouse.move(*(target or self.empty_spot()),steps=6)
    def test_empty_drop_converges_and_creates_one_undoable_merge(self):
        before=self.s.graph.to_dict()
        self.start()
        self.assertEqual(self.page.locator('#overlay path').get_attribute('d').count('M'),2)
        self.assertEqual(self.page.locator('#overlay circle').count(),1)
        self.page.mouse.up()
        self.assertTrue(f.until(lambda:'merge1' in self.s.graph.nodes))
        self.assertEqual(self.s.graph.nodes['merge1'].inputs,{'geos':['a','b']})
        self.assertEqual(self.s.graph.display,'a')
        self.page.keyboard.press('Control+z')
        self.assertTrue(f.until(lambda:self.s.graph.to_dict()==before))
    def test_escape_does_not_change_wires(self):
        before=self.s.graph.to_dict();self.start();self.page.keyboard.press('Escape');self.page.mouse.up()
        self.assertEqual(self.s.graph.to_dict(),before)
        self.assertEqual(self.page.locator('#overlay path').count(),0)
    def test_existing_multi_input_appends_without_duplicate(self):
        self.start(self.center(self.node('m').locator('.port.in')));self.page.mouse.up()
        self.assertTrue(f.until(lambda:self.s.graph.nodes['m'].inputs['geos']==['a','b']))
        self.assertEqual(len(self.s.graph.nodes),4)
    def test_empty_single_input_gets_merged_geometry(self):
        self.start(self.center(self.node('t').locator('.port.in')));self.page.mouse.up()
        self.assertTrue(f.until(lambda:self.s.graph.nodes['t'].inputs.get('geo')=='merge1'))
        self.assertEqual(self.s.graph.nodes['merge1'].inputs['geos'],['a','b'])
    def test_occupied_single_input_is_preserved(self):
        self.s.edit([{'op':'connect','from':'a','to':'t','port':'geo'}])
        self.page.wait_for_function("minihou.S.graph.nodes.t.inputs?.geo==='a'")
        before=self.s.graph.to_dict()
        self.start(self.center(self.node('t').locator('.port.in')));self.page.mouse.up()
        self.assertEqual(self.s.graph.to_dict(),before)

    def test_right_click_cancels(self):
        before=self.s.graph.to_dict();self.start();self.page.mouse.click(*self.empty_spot(),button='right');self.page.mouse.up()
        self.assertEqual(self.s.graph.to_dict(),before)
        self.assertEqual(self.page.locator('#overlay path').count(),0)
