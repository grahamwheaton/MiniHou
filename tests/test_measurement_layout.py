"""Preferences and long measurement inspectors remain readable and scrollable."""
from pathlib import Path
import unittest
from tests import test_editor_e2e as fixture

@unittest.skipIf(fixture.sync_playwright is None,'playwright unavailable')
class MeasurementLayoutTests(unittest.TestCase):
    setUpClass=classmethod(fixture.EditorE2E.setUpClass.__func__)
    tearDownClass=classmethod(fixture.EditorE2E.tearDownClass.__func__)
    setUp=fixture.EditorE2E.setUp
    tearDown=fixture.EditorE2E.tearDown
    node=fixture.EditorE2E.node
    box_scene=fixture.EditorE2E.box_scene

    def test_preferences_groups_never_overlap_and_all_controls_reachable(self):
        self.box_scene();p=self.page;p.click('#preferences-btn')
        p.select_option('#pref-length-unit','mm');p.wait_for_function("MiniUnits.unit==='mm'")
        for width,height in [(1400,960),(1000,700)]:
            p.set_viewport_size({'width':width,'height':height})
            groups=p.evaluate("""()=>[...document.querySelectorAll('#preferences fieldset:not([hidden])')].map(f=>{const r=f.getBoundingClientRect();return {top:r.top,bottom:r.bottom,content:[...f.querySelectorAll('label,p')].map(e=>e.getBoundingClientRect().bottom)}})""")
            for a,b in zip(groups,groups[1:]):self.assertLessEqual(a['bottom'],b['top']+1)
            for group in groups:self.assertLessEqual(max(group['content']),group['bottom']+1)
            for selector in ['#pref-length-unit','#pref-copy-memory','#pref-python-nodes','#preferences-reset']:
                p.locator(selector).scroll_into_view_if_needed()
                bounds=p.locator(selector).bounding_box()
                self.assertGreaterEqual(bounds['x'],0);self.assertLessEqual(bounds['x']+bounds['width'],width)
                self.assertGreaterEqual(bounds['y'],0);self.assertLessEqual(bounds['y']+bounds['height'],height)
        p.set_viewport_size({'width':1400,'height':960});p.locator('#pref-length-unit').scroll_into_view_if_needed()
        destination=Path(__file__).resolve().parents[2]/'user-fixes-20261014'
        if destination.is_dir():p.screenshot(path=str(destination/'unit-preferences-layout-mm.png'))

    def test_measurement_fields_fit_sidebar_and_delete_reachable(self):
        self.box_scene();p=self.page
        with p.expect_response('**/api/measurement_positions'):
            self.s.edit([{'op':'measurements','values':[{'id':'width','points':[[-1,-1,1],[1,-1,1]],'refs':[None,None],'label':'Box width','label_offset':[8,-8]}]}])
        p.wait_for_selector('[data-measurement="width"]');p.locator('[data-measurement="width"]').click()
        for width,height in [(1400,960),(1000,700)]:
            p.set_viewport_size({'width':width,'height':height})
            for selector in ['[aria-label="Measurement label"]','[aria-label^="Endpoint 1 X"]','[aria-label^="Endpoint 2 Z"]','[aria-label="Label Y offset (pixels)"]']:
                input=p.locator(selector);input.scroll_into_view_if_needed()
                a=input.bounding_box();b=p.locator('.pv-display-strip').bounding_box()
                self.assertGreaterEqual(a['x'],b['x']);self.assertLessEqual(a['x']+a['width'],b['x']+b['width'])
                self.assertGreaterEqual(a['y'],b['y']);self.assertLessEqual(a['y']+a['height'],b['y']+b['height'])
            self.assertTrue(p.evaluate("document.querySelector('.pv-display-strip').scrollWidth<=document.querySelector('.pv-display-strip').clientWidth+1"))
        p.set_viewport_size({'width':1400,'height':960});p.get_by_label('Measurement label',exact=True).scroll_into_view_if_needed()
        destination=Path(__file__).resolve().parents[2]/'user-fixes-20261014'
        if destination.is_dir():p.screenshot(path=str(destination/'measurement-editor-layout.png'))
        p.get_by_role('button',name='Delete measurement',exact=True).click()
        self.assertTrue(fixture.until(lambda:not self.s.graph.measurements))
