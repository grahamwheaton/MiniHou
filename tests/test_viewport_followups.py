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
    def test_first_primitive_after_clear_gets_output_and_undo(self):
        self.box_scene()
        self.s.edit([{'op':'remove','id':i} for i in list(self.s.graph.nodes)])
        self.page.wait_for_function('Object.keys(minihou.S.graph.nodes).length===0')
        self.page.evaluate("() => {const c=document.querySelector('#pv-canvas'),r=c.getBoundingClientRect(),d=new DataTransfer();d.setData('application/x-minihou-tool',JSON.stringify({type:'box'}));c.dispatchEvent(new DragEvent('drop',{bubbles:true,cancelable:true,dataTransfer:d,clientX:r.x+r.width/2,clientY:r.y+r.height/2}));}")
        self.assertTrue(f.until(lambda:len(self.s.graph.nodes)==2))
        out=next(n for n in self.s.graph.nodes.values() if n.type=='output')
        box=next(n for n in self.s.graph.nodes.values() if n.type=='box')
        self.assertEqual(out.inputs,{'geo':box.id})
        self.assertEqual(self.s.graph.display,out.id)
        self.assertEqual(out.meta['pos'][0],box.meta['pos'][0])
        self.page.click('#undo-btn')
        self.assertTrue(f.until(lambda:not self.s.graph.nodes))
        self.page.wait_for_function("Object.keys(minihou.S.graph.nodes).length===0")
        self.page.evaluate("minihou.edit([{op:'add',id:'sphere1',type:'sphere',pos:[10,20]}])")
        self.assertTrue(f.until(lambda:len(self.s.graph.nodes)==2))
        self.page.evaluate("minihou.edit([{op:'add',id:'box2',type:'box'}])")
        self.assertTrue(f.until(lambda:len(self.s.graph.nodes)==3))
        self.assertEqual(sum(n.type=='output' for n in self.s.graph.nodes.values()),1)
    def test_orbit_surface_depth_has_no_mouse_down_camera_jump(self):
        self.box_scene()
        for ortho in (False,True):
            self.page.evaluate('(v)=>{minihou.preview.view.ortho=v;minihou.preview.redraw()}',ortho)
            x,y=self.view_xy('face',self.top)
            self.page.mouse.move(x,y)
            before=self.page.evaluate('minihou.preview.navigationState()')
            self.page.mouse.down(button='middle')
            after=self.page.evaluate('minihou.preview.navigationState()')
            self.assertAlmostEqual(before['yaw'],after['yaw'],places=10)
            self.assertAlmostEqual(before['pitch'],after['pitch'],places=10)
            np.testing.assert_allclose(before['eye'],after['eye'],atol=1e-8)
            target=after['target']
            self.page.mouse.move(x+24,y+10,steps=8)
            moved=self.page.evaluate('minihou.preview.navigationState()')
            self.assertAlmostEqual(moved['yaw'],after['yaw']-24*.008,places=5)
            self.assertAlmostEqual(moved['pitch'],after['pitch']+10*.008,places=5)
            np.testing.assert_allclose(moved['target'],target,atol=1e-10)
            self.page.mouse.up(button='middle')
    def test_new_source_adds_visibility_without_replacing_existing_output(self):
        self.box_scene()
        before=self.s.graph.display
        self.page.evaluate("minihou.edit([{op:'add',id:'sphere_new',type:'sphere',params:{center:[2,0,0]},display:true}])")
        self.assertTrue(f.until(lambda:'sphere_new' in self.s.graph.nodes))
        self.assertEqual(self.s.graph.display,before)
        self.assertTrue(self.s.graph.nodes['sphere_new'].meta.get('display_extra'))
        self.assertEqual(self.s.graph.nodes['o'].inputs,{'geo':'b'})
    def test_knife_preview_cancel_commit_and_undo(self):
        self.box_scene()
        self.page.click('#pv-knife')
        self.page.mouse.click(*self.view_xy('point',0))
        self.page.keyboard.press('Escape')
        self.assertEqual(set(self.s.graph.nodes),{'b','o'})
        self.page.click('#pv-knife')
        self.page.mouse.click(*self.view_xy('point',0))
        self.page.mouse.click(*self.view_xy('point',3))
        self.assertFalse(any(n.type=='polysplit' for n in self.s.graph.nodes.values()))
        self.page.keyboard.press('Enter')
        self.assertTrue(f.until(lambda:any(n.type=='polysplit' for n in self.s.graph.nodes.values())))
        node=next(n for n in self.s.graph.nodes.values() if n.type=='polysplit')
        self.assertEqual(node.params['split_points'],'0-3')
        self.assertTrue(f.until(lambda:self.s.status()['ok']))
        self.page.click('#undo-btn')
        self.assertTrue(f.until(lambda:set(self.s.graph.nodes)=={'b','o'}))
        self.assertEqual(self.page.locator('#pv-left-tools button').count(),self.page.locator('#pv-left-tools button svg').count())
