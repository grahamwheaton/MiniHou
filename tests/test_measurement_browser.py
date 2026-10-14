"""Actual pointer editing and attachment tracking in Chromium, small geometry fixtures."""
from pathlib import Path
import unittest
try:
    from playwright.sync_api import sync_playwright
except ImportError:
    sync_playwright=None
ROOT=Path(__file__).resolve().parents[1]
@unittest.skipIf(sync_playwright is None,'playwright unavailable')
class MeasurementBrowserTests(unittest.TestCase):
    def setUp(self):
        self.pw=sync_playwright().start();self.browser=self.pw.chromium.launch();self.page=self.browser.new_page()
        self.addCleanup(self.pw.stop);self.addCleanup(self.browser.close)
        self.page.set_content('<div style="position:relative;width:500px;height:400px"><canvas width="500" height="400" tabindex="0"></canvas></div><aside></aside>')
        for name in ('units','sketch-tool'):self.page.add_script_tag(path=str(ROOT/f'minihou/web/static/{name}.js'))
        self.page.evaluate('''()=>{window.saved=[];window.commits=[];window.nodes={box:{measurement_identity:'id1'}};window.geos=[{node:'box',topology:'v1',P:[0,0,0,2,0,0,0,2,0],edge:[0,1,1,2,2,0],tri:[0,1,2]}];window.tool=MiniSketchTool({showFeedback:t=>window.hint=t,project:p=>[p[0]*50+100,p[1]*50+100],measurementGeometry:()=>geos,planePoint:(x,y)=>[(x-100)/50,(y-100)/50,0]},document.querySelector('canvas'),s=>commits.push(s),{nodes:()=>nodes,save:async values=>{if(window.delaySave)await new Promise(r=>(window.saveWaiters??=[]).push(r));values=structuredClone(values);for(const d of values)for(const r of d.refs||[])if(r)r.identity='id1';saved=values;tool.setMeasurements(values);}});document.querySelector('aside').append(tool.panel());}''')

    def test_measure_edit_deform_topology_and_replacement(self):
        p=self.page;c=p.locator('canvas');p.evaluate("tool.start('tape')")
        c.click(position={'x':100,'y':100});c.click(position={'x':200,'y':100})
        self.assertEqual(p.locator('svg text').text_content(),'2000 mm')
        self.assertEqual(p.evaluate('saved[0].refs[0].node'),'box')
        p.get_by_label('Measurement label',exact=True).fill('Width');p.get_by_label('Measurement label',exact=True).press('Tab')
        p.get_by_label('Label X offset (pixels)').fill('40');p.get_by_label('Label X offset (pixels)').press('Tab')
        self.assertEqual(p.evaluate('saved[0].label_offset[0]'),40)
        p.evaluate('geos[0].P[3]=3;tool.draw()')
        self.assertEqual(p.locator('svg text').text_content(),'Width: 3000 mm')
        p.evaluate("geos[0].topology='changed';tool.draw()")
        self.assertIn('[detached]',p.locator('svg text').text_content())
        p.evaluate("geos[0].topology='v1';nodes={};tool.draw()")
        self.assertIn('[detached]',p.locator('svg text').text_content())
        p.evaluate("nodes={box:{measurement_identity:'replacement'}};tool.draw()")
        self.assertIn('[detached]',p.locator('svg text').text_content())
        p.evaluate("nodes={box:{measurement_identity:'id1'}};tool.setMeasurements(JSON.parse(JSON.stringify(saved)))")
        self.assertEqual(p.locator('svg text').text_content(),'Width: 3000 mm')
        p.get_by_label('Endpoint 2 X (mm)').fill('500 mm');p.get_by_label('Endpoint 2 X (mm)').press('Tab')
        self.assertEqual(p.evaluate('saved[0].points[1][0]'),.5)
        self.assertIsNone(p.evaluate('saved[0].refs[1]'))
        p.get_by_role('button',name='Delete measurement',exact=True).click()
        self.assertEqual(p.evaluate('saved'),[])

    def test_edge_face_attachments_and_unit_scale(self):
        p=self.page
        results=p.evaluate('''()=>{const M=MiniSketchMath,g=geos[0],pr=p=>[p[0]*100,p[1]*100];return [M.infer([0,0,0],null,pr,[50,0],g,{grid:0}),M.infer([0,0,0],null,pr,[50,50],g,{grid:0})]}''')
        self.assertEqual(results[0]['ref']['weights'],[.75,.25])
        self.assertEqual(len(results[1]['ref']['indices']),3)
        self.assertAlmostEqual(sum(results[1]['ref']['weights']),1)
        self.assertEqual(p.evaluate("MiniUnits.parse('2 m')"),2)
        self.assertEqual(p.evaluate("MiniUnits.parse('25')"),.025)
        p.evaluate("MiniUnits.configure({length_unit:'mm',unit_scale:.001})")
        self.assertEqual(p.evaluate('MiniUnits.format(1)'),'1 mm')
        self.assertEqual(p.evaluate("MiniUnits.parse('25')"),25)

    def test_edits_queue_without_losing_later_values(self):
        p=self.page;c=p.locator('canvas');p.evaluate("tool.start('tape')")
        c.click(position={'x':100,'y':100});c.click(position={'x':200,'y':100})
        p.wait_for_function('saved.length===1');p.evaluate('window.delaySave=true')
        p.get_by_label('Measurement label',exact=True).fill('Queued');p.get_by_label('Measurement label',exact=True).press('Tab')
        p.wait_for_function('window.saveWaiters?.length===1')
        p.get_by_label('Label X offset (pixels)').fill('55');p.get_by_label('Label X offset (pixels)').press('Tab')
        p.evaluate('saveWaiters.shift()()');p.wait_for_function('saveWaiters.length===1');p.evaluate('saveWaiters.shift()()')
        p.wait_for_function("saved[0].label==='Queued'&&saved[0].label_offset[0]===55")

    def test_resolution_and_save_refresh_keep_inspector_controls_attached(self):
        p=self.page;c=p.locator('canvas');p.evaluate("tool.start('tape')")
        c.click(position={'x':100,'y':100});c.click(position={'x':200,'y':100})
        p.wait_for_function('saved.length===1')
        p.evaluate('''()=>{
            window.controls=[...document.querySelectorAll('aside input,aside select,aside button')];
            tool.setResolved({});tool.setMeasurements(structuredClone(saved));
        }''')
        self.assertTrue(p.evaluate("controls.every(e=>e.isConnected)&&controls.every((e,i)=>e===document.querySelectorAll('aside input,aside select,aside button')[i])"))
        label=p.get_by_label('Measurement label',exact=True)
        label.fill('Draft width')
        p.evaluate('geos[0].P[3]=3;tool.setResolved({})')
        self.assertEqual(label.input_value(),'Draft width')
        self.assertTrue(label.evaluate('(e)=>document.activeElement===e'))
        self.assertEqual(p.get_by_label('Endpoint 2 X (mm)').input_value(),p.evaluate('String(MiniUnits.display(3))'))
        label.press('Tab')
        p.wait_for_function("saved[0].label==='Draft width'")
        self.assertTrue(p.evaluate('controls.every(e=>e.isConnected)'))
        self.assertEqual(p.get_by_label('Measurement',exact=True).input_value(),p.evaluate('saved[0].id'))

    def test_focused_endpoint_edit_uses_latest_resolved_position(self):
        p=self.page;c=p.locator('canvas');p.evaluate("tool.start('tape')")
        c.click(position={'x':100,'y':100});c.click(position={'x':200,'y':100})
        p.wait_for_function('saved.length===1')
        endpoint=p.get_by_label('Endpoint 2 Y (mm)')
        endpoint.fill('500 mm')
        p.evaluate('geos[0].P[3]=3;tool.setResolved({})')
        self.assertEqual(endpoint.input_value(),'500 mm')
        endpoint.press('Tab')
        p.wait_for_function('saved[0].points[1][1]===.5')
        self.assertEqual(p.evaluate('saved[0].points[1]'),[3,.5,0])
        self.assertIsNone(p.evaluate('saved[0].refs[1]'))

    def test_inspector_switch_and_delete_target_selected_measurement(self):
        p=self.page
        p.evaluate("tool.setMeasurements([{id:'a',points:[[0,0,0],[1,0,0]],label:'First'},{id:'b',points:[[0,0,0],[2,0,0]],label:'Second'}])")
        p.get_by_label('Measurement',exact=True).select_option('a')
        p.get_by_label('Measurement label',exact=True).fill('Renamed')
        p.get_by_label('Measurement label',exact=True).press('Tab')
        p.wait_for_function("saved[0].label==='Renamed'")
        self.assertEqual(p.get_by_label('Measurement',exact=True).input_value(),'a')
        p.get_by_label('Measurement',exact=True).select_option('b')
        self.assertEqual(p.get_by_label('Measurement label',exact=True).input_value(),'Second')
        p.get_by_role('button',name='Delete measurement',exact=True).click()
        p.wait_for_function("saved.length===1&&saved[0].id==='a'")
        self.assertEqual(p.get_by_label('Measurement',exact=True).input_value(),'')
        self.assertEqual(p.get_by_label('Measurement label',exact=True).count(),0)

    def test_perspective_edge_face_and_nearest_overlap(self):
        p=self.page
        result=p.evaluate("""()=>{const M=MiniSketchMath,project=p=>[p[0]/p[2]*100,p[1]/p[2]*100,p[2]],g={node:'oblique',P:[0,0,1,4,0,2,0,4,2],edge:[0,1],tri:[0,1,2]};return [M.infer([0,0,0],null,project,[100,0],g,{grid:0}),M.infer([0,0,0],null,project,[50,50],g,{grid:0}),M.infer([0,0,0],null,project,[50,50],[{...g,node:'far',P:g.P.map(x=>x*2)},g],{grid:0})]}""")
        self.assertAlmostEqual(result[0]['ref']['weights'][1],1/3)
        self.assertAlmostEqual(result[1]['ref']['weights'][0],2/3)
        self.assertEqual(result[2]['ref']['node'],'oblique')

    def test_right_click_chain_commits_and_escape_only_cancels_pending(self):
        p=self.page;c=p.locator('canvas');p.evaluate("tool.start('line',{snap:false,grid:0})")
        c.click(position={'x':100,'y':100});c.click(position={'x':150,'y':100});c.click(position={'x':170,'y':120},button='right')
        self.assertTrue(p.evaluate('tool.active()'));self.assertEqual(p.evaluate('commits.length'),1)
        self.assertEqual(p.evaluate('commits[0].params.points'),'0,0,0 1,0,0')
        c.click(position={'x':200,'y':200});c.click(position={'x':250,'y':200});c.click(position={'x':270,'y':220},button='right')
        self.assertEqual(p.evaluate('commits.length'),2)
        self.assertEqual(p.evaluate('commits[1].params.points'),'2,2,0 3,2,0')
        c.click(position={'x':150,'y':150});c.click(position={'x':170,'y':170},button='right')
        self.assertEqual(p.evaluate('commits.length'),2);self.assertTrue(p.evaluate('tool.active()'))
        c.click(position={'x':150,'y':150});p.keyboard.press('Escape')
        self.assertFalse(p.evaluate('tool.active()'));self.assertEqual(p.evaluate('commits.length'),2)

from tests import test_editor_e2e as fixture
@unittest.skipIf(sync_playwright is None,'playwright unavailable')
class MeasurementEditorTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):fixture.EditorE2E.setUpClass.__func__(cls)
    @classmethod
    def tearDownClass(cls):fixture.EditorE2E.tearDownClass.__func__(cls)
    setUp=fixture.EditorE2E.setUp
    tearDown=fixture.EditorE2E.tearDown
    node=fixture.EditorE2E.node
    box_scene=fixture.EditorE2E.box_scene
    view_xy=fixture.EditorE2E.view_xy
    def test_attached_dimension_follows_a_gizmo_created_transform(self):
        self.box_scene(); p = self.page
        p.locator('#pv-sketch summary').click()
        p.locator('#pv-left-tools [data-sketch="tape"]').first.click()
        a, b = self.top_points[:2]
        p.mouse.click(*self.view_xy('point', a)); p.mouse.click(*self.view_xy('point', b))
        self.assertTrue(fixture.until(lambda: len(self.s.graph.measurements) == 1))
        original = self.s.graph.to_dict()
        from minihou.measurements import resolve_session
        before = resolve_session(self.s)['measurements'][self.s.graph.measurements[0]['id']]
        # Exercise the actual browser gizmo callbacks: insertion and parameter updates
        # run through the same gesture transaction as pointer/keyboard transforms.
        p.evaluate('''async index => {
          const pv = minihou.preview;
          pv.selectElements('point', [index], 'replace');
          const h = {param:'translate',kind:'plane',comp:null,tool:'move',pending:true,axis_rotation:[0,0,0]};
          await pv.onHandleStart(h);
          pv.onHandle(h, [0.5,0,0], true);
        }''', a)
        self.assertTrue(fixture.until(lambda: self.s._gesture is None and any(
            n.type == 'transform' and list(n.params.get('translate', [])) == [.5, 0, 0]
            for n in self.s.graph.nodes.values())))
        item = self.s.graph.measurements[0]
        self.assertEqual(self.s.graph.nodes[item['refs'][0]['node']].type, 'transform')
        after = resolve_session(self.s)['measurements'][item['id']]
        self.assertAlmostEqual(after[0]['point'][0], before[0]['point'][0] + .5)
        self.assertEqual(after[1]['point'], before[1]['point'])
        self.assertTrue(all(v['status'].startswith('Attached') for v in after))
        p.locator('#pv-canvas').focus(); p.keyboard.press('Control+z')
        self.assertTrue(fixture.until(lambda: self.s.graph.to_dict() == original))
        p.keyboard.press('Control+Shift+z')
        self.assertTrue(fixture.until(lambda: self.s.graph.measurements[0]['refs'][0]['node'] != 'b'))
        p.reload(); p.wait_for_selector('[data-measurement]')
        self.assertEqual(resolve_session(self.s)['measurements'][item['id']], after)

    def test_measurement_saved_edited_undone_and_reloaded(self):
        self.box_scene();p=self.page
        p.locator('#pv-sketch summary').click()
        p.locator('#pv-left-tools [data-sketch="tape"]').first.click()
        p.mouse.click(*self.view_xy('point',self.top_points[0]));p.mouse.click(*self.view_xy('point',self.top_points[1]))
        self.assertTrue(fixture.until(lambda:len(self.s.graph.measurements)==1))
        self.assertIsNotNone(self.s.graph.measurements[0]['refs'][0])
        ref=self.s.graph.measurements[0]['refs'][0]
        self.assertEqual(ref['node'],'b')
        self.assertIn('identity',ref)
        original_label=p.locator('[data-measurement]').text_content()
        self.s.edit([{'op':'set','id':'b','params':{'size':[4,4,4]}}])
        p.wait_for_function("minihou.preview.geometry().P.some(v=>Math.abs(v)>1.5)")
        p.wait_for_function("document.querySelector('[data-measurement]').textContent!=="+repr(original_label))
        self.assertNotIn('[detached]',p.locator('[data-measurement]').text_content())
        self.s.edit([{'op':'set','id':'b','params':{'axis_divisions':[2,2,2]}}])
        p.wait_for_function("document.querySelector('[data-measurement]').textContent.includes('[detached]')")
        self.s.undo()
        p.wait_for_function("!document.querySelector('[data-measurement]').textContent.includes('[detached]')")
        self.s.undo()
        p.wait_for_function("document.querySelector('[data-measurement]').textContent==="+repr(original_label))
        p.get_by_label('Measurement label',exact=True).fill('Roof');p.get_by_label('Measurement label',exact=True).press('Tab')
        self.assertTrue(fixture.until(lambda:self.s.graph.measurements[0]['label']=='Roof'))
        p.get_by_role('button',name='Delete measurement',exact=True).click()
        self.assertTrue(fixture.until(lambda:not self.s.graph.measurements))
        p.locator('#pv-canvas').focus();p.keyboard.press('Control+z')
        self.assertTrue(fixture.until(lambda:len(self.s.graph.measurements)==1))
        p.reload();p.wait_for_selector('[data-measurement]')
        self.assertIn('Roof',p.locator('[data-measurement]').text_content())
        p.click('#preferences-btn');p.select_option('#pref-length-unit','mm')
        p.locator('#pref-unit-scale').fill('0.001');p.locator('#pref-unit-scale').press('Tab')
        p.wait_for_function('MiniUnits.scale===.001')
        self.assertEqual(p.evaluate("MiniUnits.format(1)"),'1 mm')
        p.reload();p.wait_for_function('MiniUnits.scale===.001')
        self.assertEqual(list(self.s.graph.nodes['b'].params['size']),[2,2,2])
    def test_multi_object_attachment_moves_renames_deletes_and_undoes(self):
        self.box_scene();p=self.page
        p.evaluate("minihou.edit([{op:'add',id:'other',type:'box',params:{center:[4,0,0]}},{op:'flag',id:'other',flag:'display',value:true,additive:true}])")
        p.wait_for_function('minihou.preview.points===16')
        p.evaluate("minihou.preview.frameAll();minihou.preview.viewAxis('top',false,true)")
        p.locator('#pv-left-tools [data-sketch="tape"]').first.click()
        p.mouse.click(*self.view_xy('point',self.top_points[0]));p.mouse.click(*self.view_xy('point',self.top_points[0]+8))
        self.assertTrue(fixture.until(lambda:len(self.s.graph.measurements)==1))
        refs=self.s.graph.measurements[0]['refs']
        self.assertEqual([r['node'] for r in refs],['b','other'])
        before=p.locator('[data-measurement]').text_content()
        self.s.edit([{'op':'set','id':'other','params':{'center':[5,0,0]}}])
        p.wait_for_function("document.querySelector('[data-measurement]').textContent!=="+repr(before))
        self.assertNotIn('[detached]',p.locator('[data-measurement]').text_content())
        self.s.edit([{'op':'rename','id':'other','to':'renamed'}])
        self.assertEqual(self.s.graph.measurements[0]['refs'][1]['node'],'renamed')
        p.wait_for_function("minihou.preview.measurementGeometry().some(g=>g.node==='renamed')")
        self.assertNotIn('[detached]',p.locator('[data-measurement]').text_content())
        self.s.edit([{'op':'remove','id':'renamed'}])
        p.wait_for_function("document.querySelector('[data-measurement]').textContent.includes('[detached]')")
        self.s.undo()
        p.wait_for_function("!document.querySelector('[data-measurement]').textContent.includes('[detached]')")
        p.reload();p.wait_for_selector('[data-measurement]')
        p.wait_for_function("!document.querySelector('[data-measurement]').textContent.includes('[detached]')")
