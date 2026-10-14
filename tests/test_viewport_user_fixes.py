"""Real browser keyboard and multi-object preview regressions."""
import unittest
from tests import test_blender_selection_integration as fixture

class ViewportFixesTests(unittest.TestCase):
    setUpClass=classmethod(fixture.BlenderSelectionIntegrationTests.setUpClass.__func__)
    tearDownClass=classmethod(fixture.BlenderSelectionIntegrationTests.tearDownClass.__func__)
    setUp=fixture.BlenderSelectionIntegrationTests.setUp
    tearDown=fixture.BlenderSelectionIntegrationTests.tearDown
    node=fixture.BlenderSelectionIntegrationTests.node
    box_scene=fixture.BlenderSelectionIntegrationTests.box_scene
    view_xy=fixture.BlenderSelectionIntegrationTests.view_xy
    selection=fixture.BlenderSelectionIntegrationTests.selection

    def hover(self):
        b=self.page.locator('#pv-canvas').bounding_box()
        self.page.mouse.move(b['x']+b['width']/2,b['y']+b['height']/2)

    def test_opposite_views_and_input_focus(self):
        self.box_scene();self.hover()
        for name in ['front','back','left','right','top','bottom']:
            self.page.evaluate('(name)=>minihou.preview.viewAxis(name,false,true)',name)
            before=self.page.evaluate('({...minihou.preview.view})')
            self.page.keyboard.press('Numpad9')
            after=self.page.evaluate('({...minihou.preview.view})')
            self.assertTrue(after['ortho'])
            self.assertNotEqual((before['yaw'],before['pitch']),(after['yaw'],after['pitch']))
            self.page.keyboard.press('Numpad9')
            again=self.page.evaluate('({...minihou.preview.view})')
            import math
            self.assertAlmostEqual(math.sin(before['yaw']),math.sin(again['yaw']),places=5)
            self.assertAlmostEqual(before['pitch'],again['pitch'],places=5)
        self.page.evaluate("document.querySelector('#pv-canvas').parentElement.appendChild(Object.assign(document.createElement('input'),{id:'focus-test'}))")
        self.page.locator('#focus-test').focus();before=self.page.evaluate('JSON.stringify(minihou.preview.view)')
        self.page.keyboard.press('Numpad9')
        self.assertEqual(before,self.page.evaluate('JSON.stringify(minihou.preview.view)'))

    def test_a_modes_only_active_mesh_and_dim_preserves_geometry(self):
        self.box_scene()
        self.page.evaluate("minihou.edit([{op:'add',id:'other',type:'box',params:{center:[4,0,0]}},{op:'flag',id:'other',flag:'display',value:true,additive:true}])")
        self.page.wait_for_function('minihou.preview.points===16')
        for mode,key,count in [('point','points',8),('edge','edges',12),('face','faces',6),('piece','faces',12)]:
            self.page.click(f'#preview [data-mode="{mode}"]');self.hover();self.page.keyboard.press('a')
            self.assertEqual(len(self.selection()[key]),count)
        before=self.page.evaluate('minihou.preview.geometry().P.length')
        self.page.click('[data-pv-menu="pv-menu-view"]')
        self.page.check('#pv-isolated')
        self.assertTrue(self.page.evaluate('minihou.preview.opt.isolated'))
        self.assertEqual(before,self.page.evaluate('minihou.preview.geometry().P.length'))
        self.assertEqual(self.page.evaluate('minihou.preview.measurementGeometry().map(o=>o.P.length)'),[24,24])

    def test_flip_selected_faces_keeps_selection_and_undo(self):
        self.box_scene();self.page.evaluate('minihou.preview.selectElements("prim",[0,2])');self.hover()
        self.page.keyboard.press('Alt+n')
        self.page.wait_for_function("Object.values(minihou.S.graph.nodes).some(n=>n.type==='reverse')")
        self.page.wait_for_function('minihou.preview.selection().faces.length===2')
        self.assertEqual(self.selection()['faces'],[0,2])
        self.page.keyboard.press('Control+z')
        self.page.wait_for_function("!Object.values(minihou.S.graph.nodes).some(n=>n.type==='reverse')")
