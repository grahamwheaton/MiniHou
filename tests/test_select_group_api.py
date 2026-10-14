"""Small HTTP group-resolution tests; never realise large copy geometry."""
import http.client
import json
import tempfile
import unittest
from unittest.mock import patch

from minihou import Graph
from minihou.link.session import Session
from minihou.web import WebServer
from minihou.web.server import ApiError


class SelectGroupApiTests(unittest.TestCase):
    def setUp(self):
        self.root = tempfile.TemporaryDirectory()
        self.addCleanup(self.root.cleanup)
        self.session = Session(Graph.from_dict({'nodes': {
            'box': {'type': 'box'},
            'selected': {'type': 'group_create', 'inputs': {'geo': 'box'},
                         'params': {'name': 'face', 'domain': 'primitives', 'pattern': '0'}},
            'move': {'type': 'transform', 'inputs': {'geo': 'selected'}, 'params': {'group': 'face'}},
            'delete': {'type': 'blast', 'inputs': {'geo': 'selected'}, 'params': {'group': 'face'}},
            'edges': {'type': 'edge_collapse', 'inputs': {'geo': 'box'}, 'params': {'group': '0-2 ^1'}},
            'vertex': {'type': 'attribute_create', 'inputs': {'geo': 'box'}, 'params': {'domain': 'vertices'}},
        }, 'display': 'move'}))
        self.addCleanup(self.session.stop)
        self.web = WebServer(self.session, port=0, root=self.root.name)
        self.web.start()
        self.addCleanup(self.web.stop)

    def request(self, body):
        connection = http.client.HTTPConnection('127.0.0.1', self.web.port, timeout=5)
        connection.request('POST', '/api/select_group', json.dumps(body), {'Content-Type': 'application/json'})
        response = connection.getresponse()
        status, result = response.status, json.loads(response.read())
        connection.close()
        return status, result

    def test_point_conversion_and_graph_preserved(self):
        before = self.session.snapshot()
        status, data = self.request({'node': 'move'})
        self.assertEqual(status, 200)
        self.assertEqual(data['node'], 'selected')
        self.assertEqual(data['domain'], 'point')
        self.assertEqual(len(data['ids']), 4)
        self.assertEqual(self.session.snapshot(), before)

    def test_primitive_input_selection_survives_deleted_output(self):
        status, data = self.request({'node': 'delete'})
        self.assertEqual(status, 200)
        self.assertEqual(data, {'node': 'selected', 'domain': 'prim', 'ids': [0]})

    def test_edge_ranges_exclusions(self):
        status, data = self.request({'node': 'edges'})
        self.assertEqual(status, 200)
        self.assertEqual(data, {'node': 'box', 'domain': 'edge', 'ids': [0, 2]})

    def test_invalid_requests_and_unsupported_domain(self):
        for body in ({}, {'node': 'missing'}, {'node': 'box'}, {'node': 'vertex'}):
            self.assertEqual(self.request(body)[0], 400)

    def test_expressions_unknown_groups_and_unconnected_inputs(self):
        with self.session.lock:
            self.session.graph.nodes['move'].params['group'] = '@P.x > 0'
        status, data = self.request({'node': 'move'})
        self.assertEqual(status, 200)
        self.assertEqual(len(data['ids']), 4)
        with self.session.lock:
            self.session.graph.nodes['move'].params['group'] = 'missing'
        self.assertEqual(self.request({'node': 'move'})[0], 400)
        with self.session.lock:
            self.session.graph.nodes['move'].inputs.clear()
        self.assertEqual(self.request({'node': 'move'})[0], 400)

    def test_selection_limit_checked_before_ids_materialized(self):
        # Mock scalar size/count, rather than allocating or realising 100001 components.
        with patch('numpy.count_nonzero', return_value=100001):
            with self.assertRaises(ApiError) as caught:
                self.web.select_group('move')
        self.assertEqual(caught.exception.status, 400)


if __name__ == '__main__':
    unittest.main()
