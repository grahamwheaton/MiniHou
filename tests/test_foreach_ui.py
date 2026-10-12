import json
import unittest
from minihou import Graph
from tests import test_editor_e2e as editor


@unittest.skipIf(editor.sync_playwright is None, "requires playwright")
class ForeachUITests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        editor.EditorE2E.setUpClass()

    @classmethod
    def tearDownClass(cls):
        editor.EditorE2E.tearDownClass()

    def test_body_controls_and_incoming_attribute_tags(self):
        fixture = editor.EditorE2E()
        self.addCleanup(fixture.doCleanups)
        fixture.setUp()
        page, session = fixture.page, fixture.s
        session.replace_graph(Graph.from_dict({"nodes": {
            "box": {"type": "box"},
            "connected": {"type": "connectivity", "inputs": {"geo": "box"}},
            "each": {"type": "for_each", "inputs": {"geo": "connected"}, "params": {"body": "convex_hull"}},
        }, "display": "each"}))
        page.locator('.node[data-id="each"] .title').click()
        self.assertEqual(fixture.errors, [])
        picker = page.get_by_label("Body node", exact=True)
        self.assertEqual(picker.input_value(), "convex_hull")
        self.assertGreater(page.locator('#foreach-body-each option').count(), 20)
        page.locator('.stream-attributes summary').click()
        page.get_by_role('button', name='class · prim', exact=True).click()
        page.wait_for_function('minihou.S.graph.nodes.each.params.piece === "attribute"')
        self.assertEqual(session.graph.nodes['each'].params['attribute'], 'class')
        picker.fill('polyextrude')
        picker.press('Tab')
        control = page.locator('[data-body-param="distance"] input')
        control.fill('0.25')
        control.press('Tab')
        page.wait_for_function('JSON.parse(minihou.S.graph.nodes.each.params.params || "{}").distance === 0.25')
        self.assertEqual(json.loads(session.graph.nodes['each'].params['params'])['distance'], .25)
        self.assertEqual(fixture.errors, [])
