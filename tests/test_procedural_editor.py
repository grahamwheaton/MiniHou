"""The procedural examples and their controls through the actual browser editor."""
from pathlib import Path
import unittest

from minihou import Graph
from tests import test_editor_e2e as editor


@unittest.skipIf(editor.sync_playwright is None, "playwright not installed")
class ProceduralEditorTests(unittest.TestCase):
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

    def load_example(self, name, node_type):
        path = Path(editor.ROOT) / "examples" / name
        self.session.replace_graph(Graph.load(str(path)))
        node = next(n for n in self.session.graph.nodes.values() if n.type == node_type)
        self.page.locator(f'.node[data-id="{node.id}"] .title').click()
        self.page.wait_for_selector(f'.node[data-id="{node.id}"].selected')
        self.assertTrue(self.session.wait_idle())
        self.page.wait_for_function('window.minihou.preview?.ok && window.minihou.preview.points > 0')
        return node

    def test_building_controls_and_render(self):
        node = self.load_example("procedural_building.json", "building_rules")
        field = self.page.locator('#params [data-param="storeys"] [data-role="num"]')
        field.fill("5")
        field.press("Tab")
        self.page.wait_for_function('minihou.desktopCanClose()')
        self.assertEqual(self.session.graph.nodes[node.id].params["storeys"], 5)
        self.assertTrue(self.session.wait_idle())
        self.assertTrue(self.session.preview(node.id)["ok"])
        self.page.click("#pv-frame")
        self.assertTrue(editor.until(lambda: self.fixture.pixels_vary()))
        path = Path(editor.ROOT) / "build" / "validation" / "procedural-building.png"
        path.parent.mkdir(parents=True, exist_ok=True)
        self.page.screenshot(path=str(path))

    def test_goal_and_region_examples_preview(self):
        for filename, node_type in [("procedural_cable.json", "goal_solve"),
                                    ("procedural_fabric.json", "goal_solve"),
                                    ("procedural_packing.json", "goal_solve"),
                                    ("procedural_regions.json", "region_transform")]:
            with self.subTest(example=filename):
                node = self.load_example(filename, node_type)
                self.assertTrue(self.session.preview(node.id)["ok"])

    def test_shelf_exposes_all_system_nodes(self):
        self.page.click('#shelf .sh-tab[data-tab="Procedural"]')
        shown = self.page.locator('#shelf .sh-tool').evaluate_all('es=>es.map(e=>e.dataset.type)')
        self.assertEqual(set(shown), {"building_rules", "region_identify", "region_transform", "goal_length",
                                     "goal_collision", "goal_surface", "goal_gravity", "goal_pin", "goal_solve"})
