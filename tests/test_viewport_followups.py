import unittest
import numpy as np
from tests import test_editor_e2e as f

@unittest.skipIf(f.sync_playwright is None,'playwright unavailable')
class ViewportFollowups(unittest.TestCase):
    @classmethod
    def setUpClass(cls): f.EditorE2E.setUpClass.__func__(cls)
    @classmethod
    def tearDownClass(cls): f.EditorE2E.tearDownClass.__func__(cls)
    setUp=f.EditorE2E.setUp
    tearDown=f.EditorE2E.tearDown
    node=f.EditorE2E.node
    box_scene=f.EditorE2E.box_scene
    view_xy=f.EditorE2E.view_xy
    def test_inset_slides_left_and_typed_value_is_exact(self):
        self.box_scene();self.page.click('#preview [data-mode="face"]')
        x,y=self.view_xy('face',self.top);self.page.mouse.click(x,y);self.page.keyboard.press('i')
        self.page.mouse.move(x-17.3,y+3.1,steps=6)
        self.assertTrue(f.until(lambda:'inset1' in self.s.graph.nodes and self.s.graph.nodes['inset1'].params.get('amount',0)>0))
        value=self.s.graph.nodes['inset1'].params['amount'];self.assertAlmostEqual(value*1000,round(value*1000))
        self.page.keyboard.type('0.012345');self.page.keyboard.press('Enter')
        self.assertTrue(f.until(lambda:abs(self.s.graph.nodes['inset1'].params['amount']-.012345)<1e-10))
        self.assertEqual(self.s.graph.nodes['b'].meta['pos'][0],self.s.graph.nodes['inset1'].meta['pos'][0])
        self.assertGreater(self.s.graph.nodes['inset1'].meta['pos'][1],self.s.graph.nodes['b'].meta['pos'][1])
    def test_null_drop_inserts_pass_through(self):
        self.box_scene();before=self.s.graph.nodes['b'].params.copy()
        self.page.evaluate("() => {const c=document.querySelector('#pv-canvas'),r=c.getBoundingClientRect(),d=new DataTransfer();d.setData('application/x-minihou-tool',JSON.stringify({type:'null'}));c.dispatchEvent(new DragEvent('drop',{bubbles:true,cancelable:true,dataTransfer:d,clientX:r.x+r.width/2,clientY:r.y+r.height/2}));}")
        self.assertTrue(f.until(lambda:'null1' in self.s.graph.nodes))
        self.assertEqual(self.s.graph.nodes['null1'].inputs,{'geo':'b'})
        self.assertEqual(self.s.graph.nodes['b'].params,before)
    def test_new_lsystem_has_spatial_grammar(self):
        from minihou import Evaluator
        self.page.evaluate("minihou.edit([{op:'add',id:'plant',type:'lsystem'}])")
        self.assertTrue(f.until(lambda:'plant' in self.s.graph.nodes))
        g=Evaluator().cook(self.s.graph,'plant')
        self.assertEqual(np.linalg.matrix_rank(g.P-g.P.mean(0)),3)
    def test_wheel_keeps_pointer_anchor_in_both_views(self):
        self.box_scene();rect=self.page.locator('#pv-canvas').bounding_box()
        for ortho in [False,True]:
            self.page.evaluate('(v)=>{minihou.preview.view.ortho=v;minihou.preview.redraw()}',ortho)
            x,y=rect['width']*.8,rect['height']*.25
            anchor=self.page.evaluate('([x,y])=>minihou.preview.planePoint(x,y,minihou.preview.view.target)',[x,y])
            # Empty-space anchor is preserved by the zoom-about-pointer similarity.
            self.page.mouse.move(rect['x']+x,rect['y']+y);self.page.mouse.wheel(0,-100)
            self.page.wait_for_timeout(100)
            projected=self.page.evaluate('(p)=>minihou.preview.project(p)',anchor)
            self.assertAlmostEqual(projected[0],x,delta=1)
            self.assertAlmostEqual(projected[1],y,delta=1)
    def test_memory_and_proxy_preferences_persist_from_ui(self):
        import json, os
        self.page.locator('#preferences-btn').click()
        field=self.page.locator('#pref-copy-memory');field.fill('128');field.blur()
        path=os.path.join(self.dir,'user-preferences.json')
        def saved():
            try:
                with open(path,encoding='utf-8') as stream:return json.load(stream)
            except (OSError,ValueError):return {}
        self.assertTrue(f.until(lambda:saved().get('copy_memory_limit_mib')==128))
        field=self.page.locator('#pref-proxy-level');field.fill('0.5');field.blur()
        self.assertTrue(f.until(lambda:saved().get('proxy',{}).get('level')==.5))
        self.page.get_by_role('button',name='Done',exact=True).click();self.page.reload()
        self.page.locator('#preferences-btn').click()
        self.page.wait_for_function("document.querySelector('#pref-copy-memory').value==='128'")
        self.assertEqual(self.page.locator('#pref-proxy-level').input_value(),'0.5')
