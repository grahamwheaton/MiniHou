"""Real browser tests for scalar modeling modal lifecycle."""
from pathlib import Path
import unittest
try:
    from playwright.sync_api import sync_playwright
except ImportError:
    sync_playwright=None
@unittest.skipIf(sync_playwright is None,'playwright unavailable')
class ModalModelBrowserTests(unittest.TestCase):
    def fixture(self,page):
        page.set_content('<canvas width="500" height="400" tabindex="0"></canvas><input id="text">')
        page.add_script_tag(path=str(Path(__file__).resolve().parents[1]/'minihou/web/static/modal-model-tool.js'))
        page.evaluate('''()=>{window.updates=[];window.finishes=[];window.tool=MiniModalModelTool({showFeedback:t=>window.hint=t},document.querySelector('canvas'),{start:async type=>({type,value:0,divisions:1,scale:.01}),update:async(c,v,d)=>updates.push([v,d]),finish:async(c,cancel)=>finishes.push(cancel)});}''')
    def test_precision_snap_wheel_confirm_and_text(self):
        with sync_playwright() as pw:
            browser=pw.chromium.launch()
            try:
                page=browser.new_page();self.fixture(page)
                page.mouse.move(100,100);page.evaluate("tool.start('bevel',100,100)")
                page.mouse.move(120,100)
                page.keyboard.down('Shift');page.mouse.move(140,100);page.keyboard.up('Shift')
                self.assertAlmostEqual(page.evaluate('updates.at(-1)[0]'),.22)
                page.keyboard.down('Control');page.mouse.move(142,100)
                self.assertAlmostEqual(page.evaluate('updates.at(-1)[0]'),.2)
                page.keyboard.up('Control')
                page.mouse.wheel(0,-100)
                self.assertEqual(page.evaluate('updates.at(-1)[1]'),2)
                page.locator('#text').fill('bevel text')
                page.keyboard.press('Escape')
                self.assertEqual(page.evaluate('tool.active()'),'bevel')
                page.locator('canvas').focus();page.keyboard.press('Enter')
                page.wait_for_function('tool.active()===null')
                self.assertEqual(page.evaluate('finishes'),[False])
                page.evaluate("tool.start('extrude',142,100)")
                page.mouse.click(142,100,button='right')
                page.wait_for_function('tool.active()===null')
                self.assertEqual(page.evaluate('finishes'),[False,True])
            finally:browser.close()
    def test_async_start_cancel_and_latest_update_flush(self):
        with sync_playwright() as pw:
            browser=pw.chromium.launch()
            try:
                page=browser.new_page();self.fixture(page)
                page.evaluate('''()=>{tool.destroy();window.releaseStart=null;tool=MiniModalModelTool({},document.querySelector('canvas'),{start:()=>new Promise(r=>releaseStart=r),update:async(c,v,d)=>updates.push([v,d]),finish:async(c,x)=>finishes.push(x)});tool.start('inset',100,100);}''')
                page.mouse.move(120,100);page.keyboard.press('Escape')
                page.evaluate('releaseStart({})')
                page.wait_for_function('tool.active()===null')
                self.assertEqual(page.evaluate('finishes'),[True])
                self.assertEqual(page.evaluate('updates'),[])
                page.evaluate('''()=>{tool.destroy();window.releaseUpdate=null;tool=MiniModalModelTool({},document.querySelector('canvas'),{start:async()=>({}),update:(c,v)=>{updates.push(v);return new Promise(r=>releaseUpdate=r);},finish:async(c,x)=>finishes.push(x)});tool.start('pushpull',100,100);}''')
                page.mouse.move(130,100);page.keyboard.press('Enter')
                page.evaluate('releaseUpdate()')
                page.wait_for_function('updates.length===2')
                page.evaluate('releaseUpdate()')
                page.wait_for_function('tool.active()===null')
                self.assertAlmostEqual(page.evaluate('updates.at(-1)'),.3)
                self.assertEqual(page.evaluate('finishes'),[True,False])
            finally:browser.close()

