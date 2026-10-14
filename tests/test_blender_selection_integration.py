"""Integrated Blender-style selection and primitive placement through the real editor."""
import unittest
from tests import test_editor_e2e as fixture

@unittest.skipIf(fixture.sync_playwright is None, 'playwright unavailable')
class BlenderSelectionIntegrationTests(unittest.TestCase):
    # Compose only the fixture and helpers; do not inherit its large test suite.
    @classmethod
    def setUpClass(cls):
        fixture.EditorE2E.setUpClass.__func__(cls)
    @classmethod
    def tearDownClass(cls):
        fixture.EditorE2E.tearDownClass.__func__(cls)
    setUp = fixture.EditorE2E.setUp
    tearDown = fixture.EditorE2E.tearDown
    node = fixture.EditorE2E.node
    box_scene = fixture.EditorE2E.box_scene
    view_xy = fixture.EditorE2E.view_xy
    selection = fixture.EditorE2E.selection

    def test_c_paints_circle_without_opening_pie_and_cancel_restores(self):
        self.box_scene()
        self.page.click('#preview [data-mode="point"]')
        point=self.top_points[0]
        self.page.mouse.move(*self.view_xy('point',point))
        self.page.keyboard.press('c')
        self.assertFalse(self.page.locator('#pie').is_visible())
        self.page.mouse.click(*self.view_xy('point',point))
        self.assertIn(point,self.selection()['points'])
        self.page.keyboard.press('Escape')
        self.assertEqual(self.selection()['points'],[])
        self.page.keyboard.press('c')
        self.page.mouse.click(*self.view_xy('point',point))
        self.page.keyboard.press('Enter')
        self.assertIn(point,self.selection()['points'])

    def test_sidebar_box_and_lasso_reach_viewport(self):
        self.box_scene()
        self.page.click('#preview [data-mode="point"]')
        point=self.top_points[0]
        x,y=self.view_xy('point',point)
        self.page.click('#pv-left-tools [data-select="box"]')
        self.page.mouse.move(x-16,y-16);self.page.mouse.down()
        self.page.mouse.move(x+16,y+16);self.page.mouse.up()
        self.assertIn(point,self.selection()['points'])
        self.page.evaluate('minihou.preview.clearSelection()')
        self.page.click('#pv-left-tools [data-select="lasso"]')
        self.page.mouse.move(x-16,y-16);self.page.mouse.down()
        for dx,dy in [(16,-16),(16,16),(-16,16),(-16,-16)]:
            self.page.mouse.move(x+dx,y+dy)
        self.page.mouse.up()
        self.assertIn(point,self.selection()['points'])

    def test_group_select_resolves_original_input_ids(self):
        self.box_scene()
        self.s.edit([{'op':'add','type':'transform','id':'group_transform','inputs':{'geo':'b'},'params':{'group':'0 2','translate':[4,0,0]},'pos':[250,100]}])
        self.page.wait_for_selector('.node[data-id="group_transform"]')
        self.node('group_transform').locator('.title').click()
        self.page.locator('.prow[data-param="group"]').get_by_role('button',name='Select',exact=True).click()
        self.page.wait_for_function("minihou.preview.selection().points.join(',')==='0,2'")
        self.assertEqual(self.selection()['mode'],'point')
        self.assertIn('b',self.page.inner_text('#pv-node'))
        self.assertEqual(set(self.s.graph.nodes),{'b','o','group_transform'})

    def test_primitive_drag_shows_wireframe_and_cancel_does_not_create_nodes(self):
        self.box_scene()
        self.page.locator('#shelf [data-tab="Create"]').click()
        shelf=self.page.locator('#shelf .sh-tool[data-type="box"]')
        transfer=self.page.evaluate_handle('new DataTransfer()')
        shelf.dispatch_event('dragstart',{'dataTransfer':transfer})
        rect=self.page.locator('#pv-canvas').bounding_box()
        payload={'dataTransfer':transfer,'clientX':rect['x']+rect['width']*.6,'clientY':rect['y']+rect['height']*.6}
        self.page.locator('#pv-canvas').dispatch_event('dragover',payload)
        ghost=self.page.locator('.pv-placement-preview')
        self.assertTrue(ghost.is_visible())
        path=ghost.locator('path').get_attribute('d')
        self.assertGreaterEqual(path.count('M'),12)
        self.assertEqual(set(self.s.graph.nodes),{'b','o'})
        self.page.keyboard.press('Escape')
        self.assertFalse(ghost.is_visible())
        self.assertEqual(set(self.s.graph.nodes),{'b','o'})
        self.page.locator('#pv-canvas').dispatch_event('dragover',payload)
        self.assertTrue(ghost.is_visible())
        self.page.locator('#pv-canvas').dispatch_event('dragleave',{'dataTransfer':transfer})
        self.assertFalse(ghost.is_visible())
        shelf.dispatch_event('dragend',{'dataTransfer':transfer})
        self.assertEqual(set(self.s.graph.nodes),{'b','o'})

