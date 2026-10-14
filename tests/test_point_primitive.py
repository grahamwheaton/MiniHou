"""One-point sources remain point-only through editing and graph serialization."""
import json
import unittest
import numpy as np
from minihou import Evaluator, Graph, REGISTRY
from tests.test_nodes import run
from tests import test_editor_e2e as fixture


class PointPrimitiveTests(unittest.TestCase):
    def test_default_and_position_have_no_connectivity(self):
        for position in [(0, 0, 0), (1.25, -2, 3.75)]:
            geo = run('point', center=position)
            self.assertEqual((geo.npoints, geo.nvertices, geo.nprims), (1, 0, 0))
            np.testing.assert_allclose(geo.P, [position])
            self.assertEqual(geo.P.dtype, np.float32)
            geo.validate()

    def test_serialization_transform_and_copy_target(self):
        graph = Graph.from_dict({'nodes': {
            'p': {'type': 'point', 'params': {'center': [2, 3, 4]}},
            't': {'type': 'transform', 'inputs': {'geo': 'p'}, 'params': {'translate': [1, 0, 0]}},
            'b': {'type': 'box'},
            'c': {'type': 'copy_to_points', 'inputs': {'source': 'b', 'points': 't'}},
        }, 'display': 'c'})
        restored = Graph.from_dict(json.loads(json.dumps(graph.to_dict())))
        result = Evaluator().cook(restored, 'c')
        self.assertEqual((result.npoints, result.nprims), (8, 6))
        np.testing.assert_allclose(result.P.mean(0), [3, 3, 4])
        self.assertEqual(len(REGISTRY.get('point').handles), 3)


@unittest.skipIf(fixture.sync_playwright is None, 'playwright unavailable')
class PointPrimitiveBrowserTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls): fixture.EditorE2E.setUpClass.__func__(cls)
    @classmethod
    def tearDownClass(cls): fixture.EditorE2E.tearDownClass.__func__(cls)
    setUp = fixture.EditorE2E.setUp
    tearDown = fixture.EditorE2E.tearDown
    box_scene = fixture.EditorE2E.box_scene
    node = fixture.EditorE2E.node

    def test_shelf_drop_edit_position_undo_reload(self):
        self.box_scene()
        self.page.locator('#shelf [data-tab="Create"]').click()
        tool = self.page.locator('#shelf .sh-tool[data-type="point"]')
        self.assertTrue(tool.is_visible())
        data = self.page.evaluate_handle('new DataTransfer()')
        tool.dispatch_event('dragstart', {'dataTransfer': data})
        canvas = self.page.locator('#pv-canvas')
        rect = canvas.bounding_box()
        args = {'dataTransfer': data, 'clientX': rect['x'] + rect['width'] * .6,
                'clientY': rect['y'] + rect['height'] * .6}
        canvas.dispatch_event('dragover', args)
        self.assertTrue(self.page.locator('.pv-placement-preview').is_visible())
        canvas.dispatch_event('drop', args)
        tool.dispatch_event('dragend', {'dataTransfer': data})
        self.assertTrue(fixture.until(lambda: 'point1' in self.s.graph.nodes))
        self.assertFalse(any(n.type == 'transform' for n in self.s.graph.nodes.values()))
        self.node('point1').locator('.title').click()
        field = self.page.locator('.prow[data-param="center"] input[type="number"]').first
        field.fill('2.5'); field.press('Tab')
        self.assertTrue(fixture.until(lambda: self.s.graph.nodes['point1'].params['center'][0] == 2.5))
        geo = Evaluator().cook(self.s.graph, 'point1')
        self.assertEqual((geo.npoints, geo.nprims, geo.nvertices), (1, 0, 0))
        self.s.undo()
        self.assertTrue(fixture.until(lambda: self.s.graph.nodes['point1'].params['center'][0] != 2.5))
        self.s.redo()
        self.page.reload()
        self.page.wait_for_selector('.node[data-id="point1"]')
        self.assertEqual(self.s.graph.nodes['point1'].params['center'][0], 2.5)
