"""Actual RPC + HTTP session contracts for live Blender modifiers."""
import json
from pathlib import Path
import tempfile
import unittest
import numpy as np
from minihou import Graph, GraphError
from minihou.link.session import Session
from minihou.link.server import LinkServer
from minihou.link.instance_client import InstanceClient
from minihou.web import WebServer
from tests.test_modifier_instances import GRAPH, mesh


class LiveModifierTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name).resolve()
        self.session = Session(Graph.from_dict({"nodes": {"box": {"type": "box"}}, "display": "box"}))
        self.web = WebServer(self.session, port=0, root=self.root)
        self.web.start()
        self.addCleanup(self.web.stop)
        self.server = LinkServer(self.session, port=0)
        self.server.editor_url = self.web.url
        self.server.start()
        self.session.start()
        self.addCleanup(self.session.stop)
        self.addCleanup(self.server.stop)
        self.client = self.connect()

    def connect(self):
        client = InstanceClient(port=self.server.port)
        self.addCleanup(client.close)
        return client

    def bind(self, ident="A", client=None, x=1):
        return (client or self.client).edit_live(ident, GRAPH, {"Move": [2, 0, 0]}, mesh(x), label="Cube / " + ident)

    def test_binding_preserves_blender_time(self):
        self.client.edit_live("timed", GRAPH, {}, mesh(), frame=42, fps=30)
        self.assertEqual((self.session.frame, self.session.fps), (42, 30))
        with self.assertRaisesRegex(RuntimeError, "finite"):
            self.client.edit_live("bad", GRAPH, {}, mesh(), fps=0)

    def test_input_preview_inspector_and_two_way_parameters(self):
        bound = self.bind()
        self.assertTrue(self.session.wait_idle())
        self.assertTrue(self.session.status()["ok"], self.session.status())
        self.assertTrue(self.session.inspect("move")["ok"])
        result = self.session.preview("move")
        self.assertTrue(result["ok"], result)
        np.testing.assert_array_equal(result["geo"].P, mesh()["point/P"] + [2, 0, 0])
        self.session.edit([{"op": "set", "id": "move", "params": {"translate": [5, 0, 0]}}])
        synced = self.client.sync_live("A", {"Move": [2, 0, 0], "Scale": 1})
        values = {p["name"]: p["value"] for p in synced["interface"]}
        self.assertEqual(values["Move"], [5, 0, 0])
        _, arrays = self.client.cook("A", "", values, mesh(4), graph=synced["graph"])
        np.testing.assert_array_equal(arrays["point/P"], mesh(4)["point/P"] + [5, 0, 0])
        self.assertTrue(self.session.wait_idle())
        np.testing.assert_array_equal(self.session.preview("move")["geo"].P, arrays["point/P"])
        self.client.sync_live("A", dict(values, Move=[8, 0, 0]))
        self.assertEqual(list(self.session.graph.get_param("move", "translate")), [8, 0, 0])
        self.assertTrue(self.session.flush())
        self.assertEqual(list(Graph.load(bound["path"]).get_param("move", "translate")), [8, 0, 0])

    def test_instance_and_connection_isolation_and_switch_fence(self):
        first = self.bind()
        old = self.session.snapshot()["document_id"]
        self.session.edit([{"op": "set", "id": "move", "params": {"translate": [9, 0, 0]}}])
        second = self.bind("B", x=10)
        with self.assertRaises(GraphError):
            self.session.edit([], document_id=old)
        self.assertNotEqual(first["path"], second["path"])
        self.assertEqual(self.client.sync_live("A", {"Move": [2, 0, 0]})["graph"]["nodes"]["move"]["params"]["translate"], [9, 0, 0])
        stranger = self.connect()
        self.assertFalse(stranger.sync_live("A", {})["live"])
        self.assertEqual(self.session.live_modifier.instance_id, "B")
        self.client.remove("A")
        self.assertTrue(self.session.live_modifier.connected)
        self.client.remove("B")
        self.assertFalse(self.session.live_modifier.connected)
        self.assertFalse(self.session.preview("move")["ok"])

    def test_switch_to_file_does_not_use_stale_modifier_geometry(self):
        self.bind()
        path = self.root / "template.json"
        Graph.from_dict(GRAPH).save(path)
        self.session.open_file(path)
        self.assertIsNone(self.session.live_modifier)
        self.assertFalse(self.session.preview("move")["ok"])
        # The old modifier still receives pending edits, but cannot switch the browser back.
        self.assertTrue(self.client.sync_live("A", {})["live"])
        self.assertEqual(self.session.path, path)

    def test_failure_to_save_current_file_does_not_switch_live_target(self):
        from unittest.mock import patch
        self.bind()
        self.session.edit([{"op": "set", "id": "move", "params": {"translate": [7, 0, 0]}}])
        old = self.session.snapshot()
        with patch("minihou.link.session._atomic_write", side_effect=PermissionError("denied")):
            with self.assertRaisesRegex(RuntimeError, "save"):
                self.bind("B")
        self.assertEqual(self.session.snapshot(), old)
