"""Tiny real-browser checks of editor transform undo and proportional controls."""
import tempfile
import unittest

from minihou import Graph
from minihou.link.session import Session
from minihou.web import WebServer

try:
    from playwright.sync_api import sync_playwright
except ImportError:
    sync_playwright = None


@unittest.skipIf(sync_playwright is None, 'playwright not installed')
class TransformGestureEditorTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.pw = sync_playwright().start()
        try:
            cls.browser = cls.pw.chromium.launch()
        except Exception as error:
            cls.pw.stop()
            raise unittest.SkipTest(str(error))

    @classmethod
    def tearDownClass(cls):
        cls.browser.close()
        cls.pw.stop()

    def setUp(self):
        self.root = tempfile.TemporaryDirectory()
        self.addCleanup(self.root.cleanup)
        self.s = Session(Graph.from_dict({'nodes': {'box': {'type': 'box'},
            'move': {'type': 'transform', 'inputs': {'geo': 'box'}, 'params': {'group': '0'}}}, 'display': 'move'}))
        self.s.start()
        self.addCleanup(self.s.stop)
        self.web = WebServer(self.s, port=0, root=self.root.name)
        self.web.start()
        self.addCleanup(self.web.stop)
        self.page = self.browser.new_page(viewport={'width': 1400, 'height': 960})
        self.addCleanup(self.page.close)
        self.page.goto(self.web.url)
        self.page.wait_for_function("window.minihou?.preview?.geometry()?.P?.length > 0")

    def test_cancel_does_not_restore_preview_through_undo(self):
        before = self.s.graph.to_dict()
        self.page.evaluate("""async()=>{const pv=minihou.preview,h={param:'translate',kind:'plane',comp:null,tool:'move'};
          await pv.onHandleStart(h);pv.onHandle(h,[1,0,0],false);pv.onHandle(h,[2,0,0],false);
          await pv.onHandleCancel(h,[0,0,0]);}""")
        self.assertEqual(self.s.graph.to_dict(), before)
        self.assertFalse(self.s.undo())
        self.assertFalse(self.s.redo())

    def test_existing_transform_proportional_controls_apply(self):
        self.page.evaluate("""()=>{const check=document.querySelector('#pv-proportional-enabled');check.checked=true;check.dispatchEvent(new Event('change'));
          const radius=document.querySelector('#pv-proportional-radius');radius.value='2.5';radius.dispatchEvent(new Event('change'));}""")
        self.page.wait_for_function("minihou.S.graph.nodes.move.params.proportional_radius===2.5")
        params = self.s.graph.nodes['move'].params
        self.assertTrue(params['proportional_enabled'])
        self.assertEqual(params['proportional_radius'], 2.5)

    def test_created_transform_cancel_has_no_undo_or_redo_artifacts(self):
        before = self.s.graph.to_dict()
        self.page.evaluate("""async()=>{const pv=minihou.preview;pv.selectElements('point',[0],'replace');
          const h={param:'translate',kind:'plane',comp:null,tool:'move',pending:true,axis_rotation:[0,0,0]};
          await pv.onHandleStart(h);pv.onHandle(h,[1,0,0],false);await pv.onHandleCancel(h,[0,0,0]);}""")
        self.assertEqual(self.s.graph.to_dict(), before)
        self.assertFalse(self.s.undo())
        self.assertFalse(self.s.redo())

    def test_cancel_restores_proportional_radius_edits_too(self):
        before = self.s.graph.to_dict()
        self.page.evaluate("""async()=>{const pv=minihou.preview,h={param:'translate',kind:'plane',comp:null,tool:'move'};
          await pv.onHandleStart(h);pv.onHandle(h,[1,0,0],false);
          pv.interaction.proportional_radius=3;pv.onProportional();await pv.onHandleCancel(h,[0,0,0]);}""")
        self.assertEqual(self.s.graph.to_dict(), before)
        self.assertFalse(self.s.undo())

    def test_no_movement_does_not_add_undo_entry(self):
        self.page.evaluate("""async()=>{const pv=minihou.preview,h={param:'translate',kind:'plane',comp:null,tool:'move'};
          await pv.onHandleStart(h);pv.onHandle(h,[0,0,0],true);await new Promise(r=>setTimeout(r,100));}""")
        self.assertFalse(self.s.undo())


if __name__ == '__main__':
    unittest.main()
