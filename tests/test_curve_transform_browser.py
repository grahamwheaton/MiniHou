"""Actual curve SVG pointer lifecycle, using a bounded orthographic preview fixture."""
from pathlib import Path
import unittest
from playwright.sync_api import sync_playwright

class CurveTransformBrowserTests(unittest.TestCase):
    def test_plane_tangent_rotation_precision_and_cancel(self):
        with sync_playwright() as pw:
            browser=pw.chromium.launch()
            try:
                page=browser.new_page()
                page.set_content('<style>.pv-curve{position:absolute;inset:0;pointer-events:none}.curve-grab{pointer-events:all}</style><div style="position:relative;width:500px;height:400px"><canvas width="500" height="400"></canvas></div>')
                root=Path(__file__).resolve().parents[1]/'minihou/web/static'
                for name in ['interaction.js','curve-tool.js']:page.add_script_tag(path=str(root/name))
                page.evaluate("""()=>{window.commits=[];window.cs=[{p:[0,0,0],left:[-1,0,0],right:[1,0,0],mode:'FREE'},{p:[3,0,0],left:[2,0,0],right:[4,0,0],mode:'FREE'}];window.pv={tool:'move',interaction:{snap:false,scaleStep:.1,angleStep:15},project:p=>[200+p[0]*50,180-p[1]*50],orientationFrame:()=>MiniInteraction.identity(),planePoint:(x,y,a,n)=>n&&Math.abs(n[2])<.5?null:[(x-200)/50,(180-y)/50,a[2]],snapPosition:p=>p,showFeedback:t=>window.hint=t};window.tool=MiniCurveTool(pv,document.querySelector('canvas'),(id,c)=>commits.push(JSON.parse(c)));tool.update('curve',JSON.stringify(cs),false);} """)
                # XY square locks Z and commits one edit.
                plane=page.locator('[data-plane="2"]');box=plane.bounding_box()
                page.mouse.move(box['x']+box['width']/2,box['y']+box['height']/2);page.mouse.down();page.mouse.move(box['x']+box['width']/2+25,box['y']+box['height']/2-25);page.mouse.up()
                self.assertAlmostEqual(page.evaluate('commits[0][0].p[0]'),.5)
                self.assertAlmostEqual(page.evaluate('commits[0][0].p[1]'),.5)
                self.assertEqual(page.evaluate('commits[0][0].p[2]'),0)
                # Select tangent, then its plane gizmo. Escape rolls back without save.
                page.locator('[data-part="right"]').click()
                before=page.evaluate('commits.length')
                box=page.locator('[data-plane="2"]').bounding_box();x=box['x']+box['width']/2;y=box['y']+box['height']/2
                page.mouse.move(x,y);page.mouse.down();page.mouse.move(x+20,y+10);page.keyboard.press('Escape');page.mouse.up()
                self.assertEqual(page.evaluate('commits.length'),before)
                # Rotate selected tangent around its anchor using the actual Z ring.
                page.evaluate("pv.tool='rotate';pv.onOverlayDraw()")
                ring=page.locator('[data-axis="2"]');ring.dispatch_event('pointerdown',{'button':0,'clientX':288,'clientY':163})
                page.mouse.move(233,108);page.mouse.up()
                self.assertGreater(page.evaluate('commits.length'),before)
                self.assertAlmostEqual(page.evaluate('commits.at(-1)[0].right[0]'),.5,places=4)
                self.assertAlmostEqual(page.evaluate('commits.at(-1)[0].right[1]'),1.5,places=4)
                # Scale precision toggles without a discontinuity, then right-click cancels.
                page.evaluate("pv.tool='scale';pv.onOverlayDraw()")
                before=page.evaluate('commits.length')
                tip=page.locator('[data-axis="1"]');tip.dispatch_event('pointerdown',{'button':0,'clientX':233,'clientY':99})
                page.mouse.move(253,99)
                prior=page.evaluate('hint')
                page.keyboard.down('Shift')
                self.assertEqual(page.evaluate('hint'),prior)
                page.mouse.move(263,99)
                self.assertNotEqual(page.evaluate('hint'),prior)
                page.mouse.down(button='right');page.mouse.up(button='right');page.mouse.up()
                self.assertFalse(page.evaluate('tool.active()'))
                self.assertEqual(page.evaluate('commits.length'),before)
                # Plane scaling leaves excluded coordinate unchanged.
                result=page.evaluate("MiniInteraction.transformControl([{p:[0,0,0],left:[-1,-2,-3],right:[1,2,3],mode:'FREE'}],0,'right','scale',2,2,MiniInteraction.identity(),true)[0].right")
                self.assertEqual(result,[2,4,3])
            finally:browser.close()
