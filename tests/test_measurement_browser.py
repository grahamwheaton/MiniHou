"""Actual pointer editing and attachment tracking in Chromium, small geometry fixtures."""
from pathlib import Path
import unittest
from playwright.sync_api import sync_playwright
ROOT=Path(__file__).resolve().parents[1]
class MeasurementBrowserTests(unittest.TestCase):
    def setUp(self):
        self.pw=sync_playwright().start();self.browser=self.pw.chromium.launch();self.page=self.browser.new_page()
        self.addCleanup(self.pw.stop);self.addCleanup(self.browser.close)
        self.page.set_content('<div style="position:relative;width:500px;height:400px"><canvas width="500" height="400" tabindex="0"></canvas></div><aside></aside>')
        for name in ('units','sketch-tool'):self.page.add_script_tag(path=str(ROOT/f'minihou/web/static/{name}.js'))
        self.page.evaluate('''()=>{window.saved=[];window.commits=[];window.nodes={box:{measurement_identity:'id1'}};window.geos=[{node:'box',topology:'v1',P:[0,0,0,2,0,0,0,2,0],edge:[0,1,1,2,2,0],tri:[0,1,2]}];window.tool=MiniSketchTool({showFeedback:t=>window.hint=t,project:p=>[p[0]*50+100,p[1]*50+100],measurementGeometry:()=>geos,planePoint:(x,y)=>[(x-100)/50,(y-100)/50,0]},document.querySelector('canvas'),s=>commits.push(s),{nodes:()=>nodes,save:values=>{values=structuredClone(values);for(const d of values)for(const r of d.refs||[])if(r)r.identity='id1';saved=values;tool.setMeasurements(values);}});document.querySelector('aside').append(tool.panel());}''')

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
    def test_measurement_saved_edited_undone_and_reloaded(self):
        self.box_scene();p=self.page
        p.locator('#pv-sketch summary').click()
        p.locator('#pv-left-tools [data-sketch="tape"]').first.click()
        p.mouse.click(*self.view_xy('point',self.top_points[0]));p.mouse.click(*self.view_xy('point',self.top_points[1]))
        self.assertTrue(fixture.until(lambda:len(self.s.graph.measurements)==1))
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
