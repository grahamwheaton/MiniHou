import http.client
import json
import tempfile
import unittest

from minihou import Graph
from minihou.graph import GraphError
from minihou.link.session import Session
from minihou.web import WebServer


def parameter(value):
    return [{'op': 'set', 'id': 'move', 'params': {'translate': [value, 0, 0]}}]


class GestureUndoTests(unittest.TestCase):
    def setUp(self):
        self.s = Session(Graph.from_dict({'nodes': {'box': {'type': 'box'},
            'move': {'type': 'transform', 'inputs': {'geo': 'box'}}}, 'display': 'move'}))
        self.addCleanup(self.s.stop)

    def test_multiple_different_operations_are_one_undo(self):
        before = self.s.graph.to_dict()
        self.s.edit(parameter(1), gesture='drag')
        self.s.edit([{'op': 'set', 'id': 'move', 'params': {'rotate': [0, 0, 45]}}], gesture='drag')
        self.s.edit(parameter(4), gesture='drag')
        self.assertEqual(len(self.s._undo), 1)
        self.s.gesture_finish('drag')
        after = self.s.graph.to_dict()
        self.s.undo()
        self.assertEqual(self.s.graph.to_dict(), before)
        self.s.redo()
        self.assertEqual(self.s.graph.to_dict(), after)

    def test_cancel_restores_original_without_artifacts(self):
        before = self.s.graph.to_dict()
        self.s.edit(parameter(1), gesture='drag')
        self.s.edit(parameter(2), gesture='drag')
        self.s.gesture_finish('drag', cancel=True)
        self.assertEqual(self.s.graph.to_dict(), before)
        self.assertFalse(self.s.undo())
        self.assertFalse(self.s.redo())
        with self.assertRaises(GraphError):
            self.s.edit(parameter(3), gesture='drag')

    def test_cancel_restores_preexisting_redo(self):
        self.s.edit(parameter(5))
        expected = self.s.graph.to_dict()
        self.s.undo()
        self.s.edit(parameter(2), gesture='drag')
        self.s.gesture_finish('drag', cancel=True)
        self.assertTrue(self.s.redo())
        self.assertEqual(self.s.graph.to_dict(), expected)

    def test_created_model_node_and_updates_cancel_together(self):
        before = self.s.graph.to_dict()
        self.s.edit([{'op': 'add', 'id': 'extrude', 'type': 'polyextrude',
                      'inputs': {'geo': 'box'}, 'params': {'distance': .1}}], gesture='extrude')
        self.s.edit([{'op': 'set', 'id': 'extrude', 'params': {'distance': .4}}], gesture='extrude')
        self.assertEqual(len(self.s._undo), 1)
        self.s.gesture_finish('extrude', cancel=True)
        self.assertEqual(self.s.graph.to_dict(), before)
        self.assertFalse(self.s.redo())

    def test_foreign_edit_cannot_be_undone_by_cancel(self):
        self.s.edit(parameter(1), gesture='drag')
        self.s.edit([{'op': 'set', 'id': 'move', 'params': {'rotate': [0, 0, 90]}}])
        expected = self.s.graph.to_dict()
        for action in (lambda: self.s.gesture_finish('drag', cancel=True),
                       lambda: self.s.edit(parameter(2), gesture='drag')):
            with self.assertRaises(GraphError):
                action()
        self.assertEqual(self.s.graph.to_dict(), expected)

    def test_direct_mutation_and_wrong_document_are_fenced(self):
        self.s.edit(parameter(1), gesture='drag')
        with self.assertRaises(GraphError):
            self.s.gesture_finish('drag', cancel=True, document_id=99)
        self.s.set_node_param('move', 'rotate', [0, 0, 90])
        expected = self.s.graph.to_dict()
        with self.assertRaises(GraphError):
            self.s.gesture_finish('drag', cancel=True)
        self.assertEqual(self.s.graph.to_dict(), expected)

    def test_http_cancel_and_finish(self):
        with tempfile.TemporaryDirectory() as root:
            web = WebServer(self.s, port=0, root=root)
            web.start()
            try:
                def request(path, body):
                    c = http.client.HTTPConnection('127.0.0.1', web.port, timeout=5)
                    c.request('POST', '/api/'+path, json.dumps(body), {'Content-Type': 'application/json'})
                    r = c.getresponse()
                    result = r.status, json.loads(r.read())
                    c.close()
                    return result
                for token, end in [('cancel', 'gesture_cancel'), ('finish', 'gesture_finish')]:
                    self.assertEqual(request('edit', {'ops': parameter(3), 'gesture': token, 'document_id': 0})[0], 200)
                    status, data = request(end, {'gesture': token, 'document_id': 0})
                    self.assertEqual(status, 200)
                    self.assertEqual(data['cancelled'], end == 'gesture_cancel')
                self.assertEqual(request('gesture_cancel', {'gesture': 'cancel'})[0], 400)
                self.assertEqual(request('edit', {'ops': parameter(4), 'gesture': ''})[0], 400)
            finally:
                web.stop()


if __name__ == '__main__':
    unittest.main()
