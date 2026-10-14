"""Real browser fixture for projected shelf-drag ghost primitives."""
from pathlib import Path
import unittest
try:
    from playwright.sync_api import sync_playwright
except ImportError:
    sync_playwright = None

@unittest.skipIf(sync_playwright is None, 'playwright unavailable')
class PlacementPreviewBrowserTests(unittest.TestCase):
    def test_shapes_position_projection_update_and_clear(self):
        with sync_playwright() as pw:
            browser=pw.chromium.launch()
            try:
                page=browser.new_page()
                page.set_content('<div style="position:relative"><canvas style="width:500px;height:400px" width="500" height="400"></canvas></div>')
                page.add_script_tag(path=str(Path(__file__).resolve().parents[1]/'minihou/web/static/viewport-placement.js'))
                page.evaluate("()=>{window.zoom=10;window.pv={project:p=>[200+p[0]*zoom,200-p[1]*zoom]};window.ghost=MiniPlacementPreview(pv,document.querySelector('canvas'));}")
                self.assertTrue(page.evaluate("ghost.show('box',[1,2,3])"))
                self.assertEqual(page.locator('.pv-placement-preview path').get_attribute('d').count('M'),12)
                self.assertIn('M205,185',page.locator('.pv-placement-preview path').get_attribute('d'))
                before=page.locator('.pv-placement-preview path').get_attribute('d')
                page.evaluate('zoom=20;ghost.draw()')
                self.assertNotEqual(before,page.locator('.pv-placement-preview path').get_attribute('d'))
                self.assertEqual(page.locator('.pv-placement-preview').evaluate('(e)=>getComputedStyle(e).pointerEvents'),'none')
                for typ,count in [('sphere',96),('grid',20),('tube',72),('circle',32)]:
                    self.assertTrue(page.evaluate('(t)=>ghost.show(t,[0,0,0])',typ))
                    self.assertEqual(page.locator('.pv-placement-preview path').get_attribute('d').count('M'),count)
                self.assertEqual(page.evaluate("MiniPlacementPreview.wireframe('grid',{rows:10000,columns:10000}).length"),40)
                self.assertFalse(page.evaluate("ghost.show('noise',[0,0,0])"))
                self.assertFalse(page.evaluate('ghost.active()'))
                page.evaluate("ghost.show('box',[0,0,0]);ghost.hide()")
                self.assertEqual(page.locator('.pv-placement-preview').evaluate('(e)=>getComputedStyle(e).display'),'none')
                self.assertFalse(page.evaluate("ghost.show('box',[NaN,0,0])"))
                page.evaluate('pv.project=()=>null;ghost.show("box",[0,0,0])')
                self.assertEqual(page.locator('.pv-placement-preview').evaluate('(e)=>getComputedStyle(e).display'),'none')
                page.evaluate('ghost.destroy()')
                self.assertEqual(page.locator('.pv-placement-preview').count(),0)
            finally:browser.close()

    def test_wireframe_matches_parameter_bounds_and_transform(self):
        with sync_playwright() as pw:
            browser=pw.chromium.launch()
            try:
                page=browser.new_page();page.set_content('<div></div>')
                page.add_script_tag(path=str(Path(__file__).resolve().parents[1]/'minihou/web/static/viewport-placement.js'))
                bounds=page.evaluate("()=>{const a=MiniPlacementPreview.wireframe('box',{size:[2,4,6],rotate:[0,0,90],uniform_scale:2}).flat();return [0,1,2].map(k=>[Math.min(...a.map(p=>p[k])),Math.max(...a.map(p=>p[k]))]);}")
                for pair,expected in zip(bounds,[[-4,4],[-2,2],[-6,6]]):
                    for a,b in zip(pair,expected):self.assertAlmostEqual(a,b)
            finally:browser.close()
