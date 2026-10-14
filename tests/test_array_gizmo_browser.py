"""A real editor drag must edit an exposed library-system parameter and its cooked copies."""
import json
import shutil
import unittest
from pathlib import Path
import numpy as np
from minihou import Evaluator
from tests import test_editor_e2e as fixture

@unittest.skipIf(fixture.sync_playwright is None,'playwright not installed')
class ArrayGizmoBrowserTests(unittest.TestCase):
    # Compose the shared browser/server fixture without inheriting its test suite.
    @classmethod
    def setUpClass(cls):
        fixture.EditorE2E.setUpClass.__func__(cls)

    @classmethod
    def tearDownClass(cls):
        fixture.EditorE2E.tearDownClass.__func__(cls)

    def setUp(self):
        fixture.EditorE2E.setUp(self)
        root=Path(fixture.ROOT)
        folder=Path(self.dir)/'nodes'
        staged=Path(self.dir)/'library-ready';staged.mkdir()
        for name in ('array_points.py','circle_array.json'):
            shutil.copyfile(root/'nodes'/name,staged/name)
        # Publish both complete files at once. Use the session's cook lock for
        # refresh, like its watcher does; raw lib.refresh() races Registry.batch.
        staged.rename(folder)
        self.s._poll_library()
        self.assertTrue(fixture.until(lambda:'circle_array' in self.s.evaluator.registry))
        self.assertFalse([f for f in self.lib.status()['files'] if f['error']])
        self.page.reload()
        self.page.wait_for_selector('.node[data-id="out"]')
        self.page.select_option('#pv-follow','selection')

    def tearDown(self):
        fixture.EditorE2E.tearDown(self)

    def test_radius_drag_updates_outer_group_and_cooked_geometry(self):
        if not self.page.evaluate("!!document.createElement('canvas').getContext('webgl2')"):
            self.skipTest('this Chromium has no WebGL2')
        graph={'version':1,'nodes':{
            'source':{'type':'box','params':{'size':[.2,.2,.2]},'pos':[0,0]},
            'array':{'type':'circle_array','inputs':{'source':'source'},'params':{'count':4,'radius':2.},'pos':[0,120]},
            'out':{'type':'output','inputs':{'geo':'array'},'pos':[0,240]}},'display':'out'}
        self.s.edit([{'op':'replace','graph':graph}])
        self.page.wait_for_selector('.node[data-id="array"]')
        self.page.locator('.node[data-id="array"] .title').click()
        self.page.wait_for_function("minihou.preview.handleTips().some(h=>h.id==='targets_radius')")
        self.assertTrue(fixture.until(lambda:self.s.status()['ok']))
        before=Evaluator().cook(self.s.graph.copy())
        fixture.EditorE2E.drag_handle(self,'targets_radius',35)
        self.assertTrue(fixture.until(lambda:self.s.graph.nodes['array'].params['radius']>2.))
        self.assertTrue(fixture.until(lambda:self.s.status()['ok']))
        radius=self.s.graph.nodes['array'].params['radius']
        after=Evaluator().cook(self.s.graph.copy())
        self.assertEqual(after.npoints,32)
        self.assertEqual(self.s.graph.nodes['array'].params['count'],4)
        self.assertGreater(float(np.ptp(after.P[:,0])),float(np.ptp(before.P[:,0])))
        self.assertAlmostEqual(float(np.ptp(after.P[:,0])),2*radius+.2,places=4)
        saved=json.loads((Path(self.dir)/'nodes/circle_array.json').read_text(encoding='utf-8'))
        self.assertNotIn('radius',saved['nodes']['targets'].get('params',{}))
        self.assertEqual(self.s.graph.nodes['source'].params['size'],(.2,.2,.2))
