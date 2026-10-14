"""Display units apply only to explicit length parameters and modal model values."""
import json
from pathlib import Path
import shutil
import subprocess
import unittest
from minihou import REGISTRY
from tests import test_editor_e2e as fixture

ROOT=Path(__file__).resolve().parents[1]

class LengthFieldSchemaTests(unittest.TestCase):
    @unittest.skipUnless(shutil.which('node'), 'node unavailable')
    def test_explicit_schema_matches_node_parameters(self):
        script="require('./minihou/web/static/length-params.js');console.log(JSON.stringify(MiniLengthParams.fields))"
        data=json.loads(subprocess.check_output(['node','-e',script],cwd=ROOT,text=True,encoding='utf-8'))
        for name,fields in data.items():
            self.assertLessEqual(set(fields),{p.name for p in REGISTRY.get(name).params},name)
        self.assertNotIn('scale',data['transform'])
        self.assertNotIn('rotate',data['transform'])
        self.assertNotIn('divisions',data['box'])

@unittest.skipIf(fixture.sync_playwright is None,'playwright unavailable')
class LengthFieldBrowserTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):fixture.EditorE2E.setUpClass.__func__(cls)
    @classmethod
    def tearDownClass(cls):fixture.EditorE2E.tearDownClass.__func__(cls)
    setUp=fixture.EditorE2E.setUp
    tearDown=fixture.EditorE2E.tearDown
    box_scene=fixture.EditorE2E.box_scene
    node=fixture.EditorE2E.node

    def test_mm_fields_convert_without_rescaling_and_refresh_labels(self):
        self.box_scene()
        self.node('b').locator('.title').click()
        before=self.s.graph.to_dict()
        self.page.evaluate("MiniUnits.configure({length_unit:'mm',unit_scale:1})")
        size=self.page.locator('#params .prow[data-param="size"] input').first
        self.assertEqual(float(size.input_value()),2000)
        self.assertIn('(mm)',self.page.locator('#params .prow[data-param="size"] .pl').inner_text())
        self.assertEqual(self.s.graph.to_dict(),before)
        size.fill('2500');size.press('Tab')
        self.assertTrue(fixture.until(lambda:self.s.graph.nodes['b'].params.get('size',[1])[0]==2.5))
        self.assertEqual(float(self.page.locator('#params .prow[data-param="uniform_scale"] input[type="number"]').input_value()),1)
        self.assertEqual(float(self.page.locator('#params .prow[data-param="divisions"] input[type="number"]').input_value()),1)
        self.page.wait_for_function("minihou.S.graph.nodes.b.params.size[0]===2.5 && minihou.desktopCanClose()")
        self.page.evaluate("MiniUnits.configure({length_unit:'cm',unit_scale:.01})")
        self.assertEqual(float(size.input_value()),2.5)
        self.assertIn('(cm)',self.page.locator('#params .prow[data-param="size"] .pl').inner_text())
        self.assertEqual(self.s.graph.nodes['b'].params['size'][0],2.5)

    def test_modal_explicit_length_and_exact_typed_value(self):
        self.page.evaluate("MiniUnits.configure({length_unit:'mm',unit_scale:1})")
        self.page.evaluate("""()=>{window.unitUpdates=[];window.unitTool=MiniModalModelTool({showFeedback:t=>window.unitHint=t},document.querySelector('#pv-canvas'),{start:async()=>({value:0,scale:.01}),update:async(c,v)=>unitUpdates.push(v),finish:async()=>{}});unitTool.start('extrude',100,100);}""")
        self.page.keyboard.type('12.345mm')
        self.page.keyboard.press('Enter')
        self.page.wait_for_function('unitTool.active()===null')
        self.assertAlmostEqual(self.page.evaluate('unitUpdates.at(-1)'),.012345,places=8)
        self.page.evaluate('unitTool.destroy()')

