"""Browser contracts for route geometry and region gesture cancellation."""
from pathlib import Path
import unittest
try:
    from playwright.sync_api import sync_playwright
except ImportError:
    sync_playwright = None

@unittest.skipIf(sync_playwright is None, 'playwright unavailable')
class SelectionToolsTests(unittest.TestCase):
    def test_routes_and_regions(self):
        with sync_playwright() as pw:
            browser = pw.chromium.launch()
            try:
                page = browser.new_page()
                page.add_script_tag(path=str(Path(__file__).resolve().parents[1] / 'minihou/web/static/selection-tools.js'))
                result = page.evaluate('''()=>{
                  const positions=[[0,0,0],[1,0,0],[2,0,0],[3,0,0],[1.5,1,0]];
                  const adjacency=new Map([[0,[1,4]],[1,[0,2]],[2,[1,3]],[3,[2,4]],[4,[0,3]]]);
                  const options={adjacency,key:x=>x,position:x=>positions[x],start:0,target:3};
                  const route=MiniSelectionMath.directPath(options);
                  const disconnected=MiniSelectionMath.directPath({...options,target:5,position:x=>positions[x]||[4,0,0]});
                  const inside=MiniSelectionMath.contains({type:'lasso',points:[[0,0],[10,0],[10,10],[0,10]]},5,5);
                  const boundary=MiniSelectionMath.contains({type:'lasso',points:[[0,0],[10,0],[10,10],[0,10]]},0,5);
                  const circle=MiniSelectionMath.contains({type:'circle',center:[0,0],radius:5},3,4);
                  return {route,disconnected,inside,boundary,circle};
                }''')
                self.assertEqual(result['route'], [0,1,2,3])
                self.assertEqual(result['disconnected'], [])
                self.assertTrue(result['inside'] and result['boundary'] and result['circle'])
            finally:
                browser.close()

    def test_gestures_wheel_cancel_and_text(self):
        with sync_playwright() as pw:
            browser = pw.chromium.launch()
            try:
                page = browser.new_page()
                page.set_content('<div style="position:relative;width:500px;height:400px"><canvas width="500" height="400" tabindex="0"></canvas></div><input id="text">')
                page.add_script_tag(path=str(Path(__file__).resolve().parents[1] / 'minihou/web/static/selection-tools.js'))
                page.evaluate('''()=>{window.calls=[];window.restore=[];window.tool=MiniSelectionTool({mode:'point',selection:()=>({points:[7]}),restoreSelection:s=>restore.push(s),boxSelect:(...a)=>calls.push(a),regionSelect:(...a)=>calls.push(a)},document.querySelector('canvas'));tool.start('box');}''')
                page.mouse.move(30,40);page.mouse.down();page.mouse.move(90,100);page.mouse.up()
                self.assertEqual(page.evaluate('calls[0].at(-1)'), 'replace')
                self.assertFalse(page.evaluate('tool.active()'))
                page.evaluate("tool.start('circle')")
                page.mouse.move(100,100);page.mouse.wheel(0,-50)
                self.assertEqual(page.evaluate('tool.radius()'),35)
                page.mouse.down();page.mouse.move(150,100);page.mouse.up()
                self.assertEqual(page.evaluate('calls[1][1]'),'add')
                page.locator('#text').fill('C B text')
                page.keyboard.press('Escape')
                self.assertEqual(page.evaluate('tool.active()'),'circle')
                page.locator('canvas').focus();page.keyboard.press('Escape')
                self.assertFalse(page.evaluate('tool.active()'))
                self.assertEqual(page.evaluate('restore[0].points'),[7])
                page.evaluate("tool.start('lasso')")
                page.mouse.move(30,30);page.mouse.down();page.mouse.move(90,30);page.mouse.move(90,90);page.mouse.up()
                self.assertEqual(page.evaluate('calls.at(-1)[0].type'),'lasso')
                page.evaluate('tool.destroy()')
            finally:
                browser.close()
