"""User-only copy guard and proxy persistence, with no large realised geometry."""
import http.client
import json
import os
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

from minihou import Graph
from minihou import user_preferences
from minihou.geometry import Geometry
from minihou.link.session import Session
from minihou.nodes import python_node
from minihou.nodes.copy_to_points import copy_to_points
from minihou.sdk import REGISTRY
from minihou.web import WebServer


class PersistentPreferencesTests(unittest.TestCase):
    def setUp(self):
        self.root = tempfile.TemporaryDirectory()
        self.addCleanup(self.root.cleanup)
        self.file = os.path.join(self.root.name, 'preferences.json')
        self.environment = patch.dict(os.environ, {'MINIHOU_PREFERENCES_FILE': self.file})
        self.environment.start()
        self.addCleanup(self.environment.stop)
        enabled = python_node._ENABLED
        self.addCleanup(python_node.set_allowed, enabled)

    def session(self):
        result = Session(Graph.from_dict({'nodes': {'box': {'type': 'box'}}, 'display': 'box'}))
        self.addCleanup(result.stop)
        return result

    def server(self):
        session = self.session()
        server = WebServer(session, port=0, root=self.root.name)
        server.start()
        self.addCleanup(server.stop)
        return session, server

    def request(self, server, path, body=None, headers=None):
        connection = http.client.HTTPConnection('127.0.0.1', server.port, timeout=5)
        connection.request('GET' if body is None else 'POST', '/api/'+path,
                           None if body is None else json.dumps(body), headers or {'Content-Type': 'application/json'})
        response = connection.getresponse()
        out = response.status, json.loads(response.read())
        connection.close()
        return out

    def test_defaults_legacy_and_malformed_settings(self):
        self.assertEqual(user_preferences.read(), user_preferences.defaults())
        user_preferences.path().write_text('{"python_nodes":true}')
        self.assertTrue(user_preferences.read()['python_nodes'])
        self.assertEqual(user_preferences.copy_limit_bytes(), 256*1024**2)
        user_preferences.path().write_text('{"copy_memory_limit_mib":false,"proxy":{"level":"wrong","auto":true}}')
        saved = user_preferences.read()
        self.assertEqual(saved['copy_memory_limit_mib'], 256)
        self.assertEqual(saved['proxy']['level'], 1)
        self.assertTrue(saved['proxy']['auto'])

    def test_partial_updates_and_python_preserve_other_settings(self):
        user_preferences.update({'copy_memory_limit_mib': 512, 'proxy': {'level': .25, 'auto': True, 'link_tris': 2000}})
        user_preferences.set_python(True)
        user_preferences.update({'proxy': {'link_tris': 4000}})
        saved = user_preferences.read()
        self.assertTrue(saved['python_nodes'])
        self.assertEqual(saved['copy_memory_limit_mib'], 512)
        self.assertEqual(saved['proxy'], {'level': .25, 'auto': True, 'link_tris': 4000})
        self.assertEqual(self.session().proxy_level, .25)
        self.assertTrue(self.session().proxy_auto)
        self.assertEqual(self.session().link_tris, 4000)

    def test_copy_limit_validation_never_writes_invalid_settings(self):
        for value in (0, -1, 15, 16385, True, '512', 512.0):
            with self.assertRaises(ValueError):
                user_preferences.update({'copy_memory_limit_mib': value})
        self.assertFalse(user_preferences.path().exists())

    def test_copy_preflight_changes_before_allocation_and_cache_key_changes(self):
        source = Geometry.points_only([[0, 0, 0]])
        tiny = Geometry.points_only([[0, 0, 0]])
        # Only the count is synthetic. Position/attribute arrays contain one point.
        points = SimpleNamespace(npoints=200000, point=tiny.point, P=tiny.P)
        user_preferences.update({'copy_memory_limit_mib': 16})
        old_key = REGISTRY.get('copy_to_points').key_extra({})
        with patch('minihou.nodes.copy_to_points._copy_positions_numba', side_effect=RuntimeError('preflight passed')) as allocate:
            with self.assertRaisesRegex(ValueError, '16 MiB'):
                copy_to_points(source, points, False, False)
            allocate.assert_not_called()
            user_preferences.update({'copy_memory_limit_mib': 32})
            with self.assertRaisesRegex(RuntimeError, 'preflight passed'):
                copy_to_points(source, points, False, False)
            allocate.assert_called_once()
        self.assertNotEqual(old_key, REGISTRY.get('copy_to_points').key_extra({}))
        with self.assertRaises(ValueError):
            Graph.from_dict({'nodes': {'copy': {'type': 'copy_to_points', 'params': {'copy_memory_limit_mib': 16384}}}})

    def test_http_settings_invalidate_cook_and_preserve_python_override(self):
        python_node.set_allowed(True)
        session, server = self.server()
        before = session.rev
        with patch.object(session.evaluator.cache, 'clear') as clear:
            status, data = self.request(server, 'preferences', {'copy_memory_limit_mib': 512})
            self.assertEqual(status, 200)
            clear.assert_called_once()
        self.assertGreater(session.rev, before)
        self.assertTrue(data['python_nodes'])
        self.assertEqual(data['copy_memory_limit_mib'], 512)
        self.assertEqual(data['limits']['copy_memory_limit_mib'], {'min': 16, 'max': 16384, 'default': 256})
        self.assertEqual(self.request(server, 'preferences')[1]['copy_memory_limit_mib'], 512)
        self.assertEqual(self.request(server, 'preferences', {'copy_memory_limit_mib': True})[0], 400)
        self.assertEqual(self.request(server, 'preferences', {'copy_memory_limit_mib': 1024},
                                      {'Content-Type': 'application/json', 'Origin': 'https://example.com'})[0], 403)

    def test_proxy_http_persists_but_direct_override_is_transient(self):
        session, server = self.server()
        self.assertEqual(self.request(server, 'proxy', {'level': .2, 'auto': True, 'link_tris': 1234})[0], 200)
        session.set_proxy(level=.8, auto=False)
        restarted = self.session()
        self.assertEqual(restarted.proxy_level, .2)
        self.assertTrue(restarted.proxy_auto)
        self.assertEqual(restarted.link_tris, 1234)
        user_preferences.set_python(True)
        self.assertEqual(user_preferences.read()['proxy']['level'], .2)


if __name__ == '__main__':
    unittest.main()
