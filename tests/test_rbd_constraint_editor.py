"""Constraint creation and parameter edits through the actual browser editor."""
import unittest

from tests import test_editor_e2e as editor


@unittest.skipIf(editor.sync_playwright is None, "playwright not installed")
class ConstraintEditorTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        editor.EditorE2E.setUpClass()

    @classmethod
    def tearDownClass(cls):
        editor.EditorE2E.tearDownClass()

    def setUp(self):
        self.fixture = editor.EditorE2E()
        self.addCleanup(self.fixture.doCleanups)
        self.fixture.setUp()
        self.page, self.session = self.fixture.page, self.fixture.s

    def tearDown(self):
        self.assertEqual(self.fixture.errors, [])

    def create_constraint(self):
        self.page.click('#shelf .sh-tab[data-tab="Constraints"]')
        self.page.click('#shelf .sh-tool[data-label="RBD Constraint"]')
        self.assertTrue(editor.until(lambda: any(n.type == "rbd_constraint" for n in self.session.graph.nodes.values())))
        node = next(n for n in self.session.graph.nodes.values() if n.type == "rbd_constraint")
        self.page.wait_for_selector(f'.node[data-id="{node.id}"].selected')
        self.assertEqual(node.inputs, {})
        return node.id

    def number(self, name, value):
        field = self.page.locator(f'#params [data-param="{name}"] [data-role="num"]')
        field.fill(str(value))
        field.press("Tab")

    def test_constraint_shelf_kind_limits_and_break_controls(self):
        node_id = self.create_constraint()
        kind = self.page.locator('#params [data-param="kind"] select')
        choices = kind.locator('option').evaluate_all('es=>es.map(e=>e.value)')
        self.assertEqual(set(choices), {"glue", "pin", "hinge", "spring"})
        kind.select_option("hinge")
        self.number("body_a", 1)
        self.number("body_b", 2)
        self.page.check('#params [data-param="limited"] input[type="checkbox"]')
        self.number("lower", -25)
        self.number("upper", 40)
        self.number("strength", 500)
        self.number("break_torque", 20)
        self.page.wait_for_function('minihou.desktopCanClose()')
        params = self.session.graph.nodes[node_id].params
        for key, value in dict(kind="hinge", body_a=1, body_b=2, limited=True,
                               lower=-25, upper=40, strength=500, break_torque=20).items():
            self.assertEqual(params[key], value)
        self.assertTrue(self.session.wait_idle())
        self.assertTrue(self.session.preview(node_id)["ok"])

    def test_spring_parameters_and_search_discoverability(self):
        node_id = self.create_constraint()
        self.page.select_option('#params [data-param="kind"] select', "spring")
        self.page.check('#params [data-param="world"] input[type="checkbox"]')
        self.number("stiffness", 120)
        self.number("damping", 4)
        self.number("rest_length", 0.75)
        for i, value in enumerate([0.25, 0, 1.5]):
            field = self.page.locator(f'#params [data-param="anchor_b"] [data-role="v{i}"]')
            field.fill(str(value))
            field.press("Tab")
            self.page.wait_for_function('minihou.desktopCanClose()')
        self.page.wait_for_function('minihou.desktopCanClose()')
        params = self.session.graph.nodes[node_id].params
        self.assertEqual(params["kind"], "spring")
        self.assertTrue(params["world"])
        self.assertEqual(params["stiffness"], 120)
        self.assertEqual(params["damping"], 4)
        self.assertEqual(params["rest_length"], 0.75)
        self.assertEqual(list(params["anchor_b"]), [0.25, 0, 1.5])
        self.assertTrue(self.session.wait_idle())
        self.assertTrue(self.session.preview(node_id)["ok"])
        from minihou import menu
        for term in ("hinge", "pin", "spring"):
            self.assertIn(term, menu.ALIASES["rbd_constraint"])
