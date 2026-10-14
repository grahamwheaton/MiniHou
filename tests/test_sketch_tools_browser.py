"""Real browser sketch pointer/key lifecycle with a deterministic orthographic preview fixture."""
from pathlib import Path
import unittest
try:
    from playwright.sync_api import sync_playwright
except ImportError:
    sync_playwright=None
@unittest.skipIf(sync_playwright is None,'playwright unavailable')
class SketchBrowserTests(unittest.TestCase):
    def test_draw_measure_cancel_and_numeric(self):
        with sync_playwright() as pw:
            browser=pw.chromium.launch()
            try:
                page=browser.new_page()
                page.set_content('<div style="position:relative;width:500px;height:400px"><canvas width="500" height="400" tabindex="0"></canvas></div><input id="text">')
                page.add_script_tag(path=str(Path(__file__).resolve().parents[1]/'minihou/web/static/sketch-tool.js'))
                page.evaluate('''()=>{window.commits=[];const canvas=document.querySelector('canvas');window.tool=MiniSketchTool({showFeedback:t=>window.hint=t,project:p=>[p[0]*50+100,p[1]*50+100],geometry:()=>null,planePoint:(x,y)=>[(x-100)/50,(y-100)/50,0]},canvas,s=>commits.push(s));tool.start('rectangle',{grid:0});}''')
                c=page.locator('canvas')
                c.click(position={'x':100,'y':100});c.click(position={'x':200,'y':200})
                self.assertEqual(page.evaluate('commits.length'),1)
                self.assertTrue(page.evaluate('commits[0].params.closed'))
                page.evaluate("tool.start('line',{grid:0,snap:false})")
                c.click(position={'x':100,'y':100});page.mouse.move(158,108)
                page.keyboard.type('3');page.keyboard.press('Enter')
                self.assertEqual(page.evaluate('commits[1].params.points'),'0,0,0 3,0,0')
                page.evaluate("tool.start('tape',{grid:0,snap:false})")
                c.click(position={'x':100,'y':100});c.click(position={'x':250,'y':300})
                self.assertEqual(page.evaluate('tool.dimensions().length'),1)
                self.assertEqual(page.locator('svg text').text_content(),'5.0000')
                page.evaluate("tool.start('pen')")
                c.click(position={'x':100,'y':100});page.keyboard.press('Escape')
                self.assertFalse(page.evaluate('tool.active()'))
                self.assertEqual(page.evaluate('commits.length'),2)
                page.evaluate("tool.start('pen')")
                page.locator('#text').fill('xyz123')
                self.assertTrue(page.evaluate('tool.active()'))
                self.assertEqual(page.locator('#text').input_value(),'xyz123')
            finally:browser.close()
    def test_path_snaps_to_objects_outside_grid_plane(self):
        with sync_playwright() as pw:
            browser=pw.chromium.launch()
            try:
                page=browser.new_page()
                page.set_content('<div style="position:relative;width:500px;height:400px"><canvas width="500" height="400" tabindex="0"></canvas></div>')
                page.add_script_tag(path=str(Path(__file__).resolve().parents[1]/'minihou/web/static/sketch-tool.js'))
                page.evaluate('''()=>{window.commits=[];window.tool=MiniSketchTool({showFeedback:t=>window.hint=t,project:p=>[p[0]*50+100,p[1]*50+100],sketchGeometry:()=>[{P:[0,0,2,2,0,2,2,2,2],edge:[0,1,1,2]}],planePoint:(x,y)=>[(x-100)/50,(y-100)/50,0]},document.querySelector('canvas'),s=>commits.push(s));tool.start('line');}''')
                c=page.locator('canvas')
                for x,y in [(101,101),(201,101),(201,201)]:c.click(position={'x':x,'y':y})
                self.assertEqual(page.evaluate('commits.length'),0)
                page.keyboard.press('Enter')
                self.assertEqual(page.evaluate('commits[0].params.points'),'0,0,2 2,0,2 2,2,2')
                page.evaluate("tool.start('tape')")
                c.click(position={'x':101,'y':101});c.click(position={'x':201,'y':201})
                self.assertEqual(page.evaluate('tool.dimensions()[0]'),[[0,0,2],[2,2,2]])
                page.evaluate("tool.start('rectangle')")
                c.click(position={'x':101,'y':101});c.click(position={'x':201,'y':201})
                self.assertEqual(page.evaluate('commits[1].params.points'),'0,0,2 2,0,2 2,2,2 0,2,2')
            finally:browser.close()
