"""Browser RAF and live reduced-motion contract for the dynamic feedback module."""
from pathlib import Path
import unittest
try:
    from playwright.sync_api import sync_playwright
except ImportError:
    sync_playwright = None

@unittest.skipIf(sync_playwright is None, 'playwright unavailable')
class DynamicBrowserTests(unittest.TestCase):
    def test_real_animation_and_live_reduced_motion(self):
        with sync_playwright() as pw:
            browser = pw.chromium.launch()
            try:
                page = browser.new_page(reduced_motion='no-preference')
                page.set_content('<div id="node">node</div>')
                page.add_script_tag(path=str(Path(__file__).resolve().parents[1] / 'minihou/web/static/dynamic-nodes.js'))
                page.evaluate("""()=>{
                  window.d=MiniDynamicNodes({onChange:offsets=>{
                    const [x,y]=offsets.get('a')||[0,0];
                    document.querySelector('#node').style.transform=`translate(${x}px,${y}px)`;
                  }});
                  d.update({nodes:[{id:'a',x:10,y:0}],dragging:['other'],pointer:[0,0]});
                }""")
                page.wait_for_function("d.offset('a')[0]>2")
                page.emulate_media(reduced_motion='reduce')
                page.wait_for_function("d.offset('a')[0]===0")
                self.assertEqual(page.locator('#node').evaluate("e=>e.style.transform"), 'translate(0px, 0px)')
                page.emulate_media(reduced_motion='no-preference')
                page.wait_for_function("d.offset('a')[0]>2")
                page.evaluate('d.update({dragging:[]})')
                page.wait_for_function("d.offset('a')[0]===0")
                page.evaluate('d.dispose()')
            finally:
                browser.close()

if __name__ == '__main__':
    unittest.main()
