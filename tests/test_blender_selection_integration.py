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
        # Hold a stale preview in flight while Select requests the input geometry.
        self.page.evaluate("""() => {
            const original = window.fetch;
            window.fetch = async (...args) => {
                const response = await original(...args);
                if (args[0] === '/api/preview' && JSON.parse(args[1].body).node === 'group_transform')
                    await new Promise(resolve => window.releaseGroupPreview = resolve);
                return response;
            };
        }""")
        self.node('group_transform').locator('.title').click()
        self.page.wait_for_function("typeof window.releaseGroupPreview === 'function'")
        self.page.locator('.prow[data-param="group"]').get_by_role('button',name='Select',exact=True).click()
        self.page.wait_for_function("minihou.S.sel.has('b')")
        self.assertNotEqual(self.selection()['points'], [0, 2])
        self.page.evaluate("window.releaseGroupPreview()")
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

    def test_sidebar_draw_curve_commits_a_bezier(self):
        import json
        self.box_scene()
        self.page.locator('#pv-sketch summary').click()
        self.page.get_by_label('Sketch grid').fill('0')
        self.page.get_by_label('Sketch grid').press('Tab')
        self.page.click('#pv-left-tools [data-sketch="bezier"]')
        rect=self.page.locator('#pv-canvas').bounding_box()
        self.page.keyboard.down('Control')
        for x,y in [(.2,.65),(.5,.5),(.8,.65)]:
            self.page.mouse.click(rect['x']+rect['width']*x,rect['y']+rect['height']*y)
        self.page.keyboard.up('Control')
        self.page.keyboard.press('Enter')
        self.assertTrue(fixture.until(lambda:any(n.type=='bezier' for n in self.s.graph.nodes.values())))
        node=next(n for n in self.s.graph.nodes.values() if n.type=='bezier')
        controls=json.loads(node.params['controls'])
        self.assertEqual(len(controls),3)
        self.assertTrue(all(c['mode']=='AUTO' for c in controls))
        self.assertFalse(node.params['closed'])
        self.assertFalse(any(n.type=='curve' for n in self.s.graph.nodes.values()))
        self.assertTrue(fixture.until(lambda:self.s.status()['ok']))

    def exercise_modal_face_operation(self,shortcut,node_type,param):
        self.box_scene()
        self.page.click('#preview [data-mode="face"]')
        self.page.mouse.click(*self.view_xy('face',self.top))
        x,y=self.view_xy('face',self.top)
        self.page.mouse.move(x,y)
        self.page.keyboard.press(shortcut)
        self.assertTrue(fixture.until(lambda:any(n.type==node_type for n in self.s.graph.nodes.values())))
        nid=next(i for i,n in self.s.graph.nodes.items() if n.type==node_type)
        self.page.mouse.move(x+35,y)
        self.assertTrue(fixture.until(lambda:self.s.graph.nodes[nid].params.get(param,0)>0))
        self.page.keyboard.press('Escape')
        self.assertTrue(fixture.until(lambda:nid not in self.s.graph.nodes))
        self.page.wait_for_function("document.querySelector('.pv-drag-feedback').hidden")
        self.assertEqual(set(self.s.graph.nodes),{'b','o'})
        self.assertEqual(self.s.graph.nodes['o'].inputs,{'geo':'b'})
        self.page.wait_for_function("document.querySelector('#pv-node').innerText.includes('b')")
        # #pv-node names the node as soon as its preview is requested; a click before the box's own geometry is installed is dropped
        # with the cancelled tool's geometry (other point numbers). Wait for the box itself.
        from minihou import Evaluator
        npts=Evaluator().cook(self.s.graph,'b').npoints
        # the cancelled tool finishes in the browser after the server: it reloads and reselects its source node, which resets the 3-D selection
        self.page.wait_for_function("(n) => minihou.desktopCanClose() && minihou.S.sel.has('b') && minihou.preview.points === n && !minihou.preview.isDragging()", arg=npts)
        self.page.mouse.click(*self.view_xy('face',self.top))
        x,y=self.view_xy('face',self.top)
        self.page.mouse.move(x,y)
        self.page.keyboard.press(shortcut)
        self.assertTrue(fixture.until(lambda:any(n.type==node_type for n in self.s.graph.nodes.values())), self.editor_state())
        nid=next(i for i,n in self.s.graph.nodes.items() if n.type==node_type)
        self.page.mouse.move(x+25,y)
        self.assertTrue(fixture.until(lambda:self.s.graph.nodes[nid].params.get(param,0)>0))
        self.page.keyboard.press('Enter')
        self.page.wait_for_function("document.querySelector('.pv-drag-feedback').hidden")
        self.assertIn(nid,self.s.graph.nodes)
        self.assertEqual(self.s.graph.nodes[nid].inputs,{'geo':'b'})
        self.assertEqual(self.s.graph.nodes['o'].inputs,{'geo':nid})
        self.assertTrue(fixture.until(lambda:self.s.status()['ok']))

    def editor_state(self):
        """What the editor looked like when a modal tool did not start (for the failure message)."""
        return self.page.evaluate("""() => {
          const t = document.querySelector('#toast'), e = document.querySelector('#errorbar'), a = document.activeElement, c = document.querySelector('#pv-canvas').getBoundingClientRect();
          const st = minihou.S.status || {};
          return {toast: t ? [getComputedStyle(t).display, t.textContent] : null, faces: minihou.preview.selection ? (minihou.preview.selection().faces || []).length : null,
                  focus: a ? a.tagName + '#' + a.id : null, errorbar: [e.className, e.textContent], canvas: [c.left, c.top, c.width, c.height],
                  dragging: document.body.classList.contains('pv-dragging'), status: [st.ok, st.busy, st.error && st.error.message], node: minihou.preview.node};
        }""") | {"graph": sorted(self.s.graph.nodes)}

    def test_i_inset_modal_cancel_and_confirm(self):
        self.exercise_modal_face_operation('i','inset','amount')

    def test_alt_e_pushpull_modal_cancel_and_confirm(self):
        self.exercise_modal_face_operation('Alt+e','pushpull','distance')

