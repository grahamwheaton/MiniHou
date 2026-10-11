"""Real socket tests for independent stack-aware graph instances."""
import copy
import time
import unittest
from unittest.mock import Mock, patch
from concurrent.futures import ThreadPoolExecutor

import numpy as np
from minihou import Graph
from minihou.link.instance_client import InstanceClient
from minihou.link.server import LinkServer
from minihou.link.session import Session

GRAPH = {"nodes": {
    "input": {"type": "modifier_input"},
    "move": {"type": "transform", "inputs": {"geo": "input"}},
    "out": {"type": "output", "inputs": {"geo": "move"}}}, "display": "out",
    "exposed": [{"name": "Move", "node": "move", "param": "translate"},
                {"name": "Scale", "node": "move", "param": "uniform_scale"}]}


def mesh(x=1):
    return {"point/P": np.array([[x, 0, 0], [x, 1, 0], [x, 0, 1]], np.float32),
            "topo/vertex_point": np.array([0, 1, 2], np.int32),
            "topo/prim_offsets": np.array([0, 3], np.int32)}


class ModifierInstances(unittest.TestCase):
    def setUp(self):
        self.session = Session(Graph.from_dict({"nodes": {"g": {"type": "grid"}}, "display": "g"}))
        self.server = LinkServer(self.session, port=0)
        self.server.start()
        self.clients = []

    def tearDown(self):
        for c in self.clients:
            c.close()
        self.server.stop()

    def client(self):
        c = InstanceClient(port=self.server.port)
        self.clients.append(c)
        return c

    def test_independent_parameters_defaults_and_chained_input(self):
        c = self.client()
        welcome = c.connect()
        self.assertIn("instances_v1", welcome["capabilities"])
        self.assertEqual(len(c.describe("", GRAPH)["interface"]), 2)
        _, a = c.cook("A", "", {"Move": [2, 0, 0]}, mesh(), graph=GRAPH)
        _, b = c.cook("B", "", {"Scale": 3}, a, graph=GRAPH)
        np.testing.assert_array_equal(a["point/P"][:, 0], [3, 3, 3])
        np.testing.assert_array_equal(b["point/P"][:, 0], [9, 9, 9])
        _, b = c.cook("B", "", {}, mesh(), graph=GRAPH)
        np.testing.assert_array_equal(b["point/P"][:, 0], [1, 1, 1])
        self.assertEqual(self.session.graph.nodes["g"].type, "grid")
        self.assertEqual(len(self.server.clients[0].instances.instances), 2)
        c.remove("A")
        self.assertEqual(len(self.server.clients[0].instances.instances), 1)

    def test_connection_ownership_concurrency_and_cleanup(self):
        a, b = self.client(), self.client()
        a.connect(); b.connect()
        with ThreadPoolExecutor(2) as pool:
            jobs = [pool.submit(c.cook, "same-id", "", {"Move": [i, 0, 0]}, mesh(), graph=GRAPH)
                    for c, i in [(a, 4), (b, 8)]]
            outputs = [j.result()[1]["point/P"][0, 0] for j in jobs]
        self.assertEqual(outputs, [5, 9])
        owners = self.server.clients
        a.close(); b.close()
        deadline = time.monotonic() + 2
        while self.server.clients and time.monotonic() < deadline:
            time.sleep(.01)
        self.assertFalse(self.server.clients)
        self.assertTrue(all(not owner.instances.instances for owner in owners))

    def test_bad_graph_and_params_do_not_poison_other_instances(self):
        c = self.client()
        bad = copy.deepcopy(GRAPH)
        bad["nodes"]["input"] = {"type": "blender_object", "params": {"object": "Cube"}}
        with self.assertRaisesRegex(RuntimeError, "feedback"):
            c.cook("bad", "", {}, mesh(), graph=bad)
        bad = copy.deepcopy(GRAPH)
        bad["nodes"]["move"]["inputs"]["geo"] = "grid"
        bad["nodes"]["grid"] = {"type": "grid"}
        with self.assertRaisesRegex(RuntimeError, "replacement-only"):
            c.describe("", bad)
        with self.assertRaisesRegex(RuntimeError, "exposed"):
            c.cook("bad", "", {"missing": 4}, mesh(), graph=GRAPH)
        _, output = c.cook("good", "", {}, mesh(7), graph=GRAPH)
        self.assertEqual(output["point/P"][0, 0], 7)

    def test_indirect_library_input_is_blocked_without_poisoning_legacy_store(self):
        from minihou.inputs import INPUTS
        from minihou.sdk import REGISTRY, node
        from minihou.geometry import Geometry
        @node("test_hidden_input", inputs=["geo"])
        def hidden(geo):
            return INPUTS.get("Other").geo
        INPUTS.put("Other", Geometry.from_arrays(np.array([[42, 0, 0]], np.float32)))
        try:
            graph = copy.deepcopy(GRAPH)
            graph["nodes"]["hidden"] = {"type": "test_hidden_input", "inputs": {"geo": "move"}}
            graph["nodes"]["out"]["inputs"]["geo"] = "hidden"
            # A nested system evaluator may already have warm legacy cache entries.
            from minihou import Evaluator
            from minihou.inputs import without_object_inputs
            inner = Graph.from_dict({"nodes": {"g": {"type": "grid"},
                "hidden": {"type": "test_hidden_input", "inputs": {"geo": "g"}}}, "display": "hidden"})
            evaluator = Evaluator()
            self.assertEqual(evaluator.cook(inner).P[0, 0], 42)
            with without_object_inputs(), self.assertRaisesRegex(RuntimeError, "feedback"):
                evaluator.cook(inner)
            c = self.client()
            c.connect()
            sock = c.sock
            with self.assertRaisesRegex(RuntimeError, "feedback"):
                c.cook("bad", "", {}, mesh(), graph=graph)
            self.assertIs(c.sock, sock)
            _, result = c.cook("good", "", {}, mesh(), graph=GRAPH)
            self.assertEqual(result["point/P"][0, 0], 1)
            self.assertEqual(INPUTS.get("Other").geo.P[0, 0], 42)
        finally:
            REGISTRY.unregister("test_hidden_input")
            INPUTS.clear()

    def test_invalid_input_and_time_report_errors_then_recover(self):
        c = self.client()
        data = mesh()
        data["point/P"][0, 0] = np.inf
        with self.assertRaisesRegex(RuntimeError, "finite"):
            c.cook("A", "", {}, data, graph=GRAPH)
        data = mesh()
        data["topo/vertex_point"][0] = 99
        with self.assertRaisesRegex(RuntimeError, "outside"):
            c.cook("A", "", {}, data, graph=GRAPH)
        with self.assertRaisesRegex(RuntimeError, "fps"):
            c.cook("A", "", {}, mesh(), fps=0, graph=GRAPH)
        _, result = c.cook("A", "", {}, mesh(), graph=GRAPH)
        self.assertEqual(result["point/P"][0, 0], 1)

    def test_input_attributes_and_topology_invalidate(self):
        c = self.client()
        data = mesh()
        data["point/weight"] = np.ones(3, np.float32)
        _, first = c.cook("A", "", {}, data, graph=GRAPH)
        data["point/weight"] = np.full(3, 2, np.float32)
        data["topo/vertex_point"] = np.array([2, 1, 0], np.int32)
        _, second = c.cook("A", "", {}, data, graph=GRAPH)
        np.testing.assert_array_equal(first["point/weight"], [1, 1, 1])
        np.testing.assert_array_equal(second["point/weight"], [2, 2, 2])
        np.testing.assert_array_equal(second["topo/vertex_point"], [2, 1, 0])


class InstanceClientValidation(unittest.TestCase):
    def test_wrong_server_and_protocol_close_handshake_socket(self):
        good = {"type": "welcome", "server": "minihou", "protocol": 1,
                "capabilities": ["instances_v1"]}
        for override in ({"server": "other"}, {"protocol": 99}, {"type": "pong"}):
            sock = Mock()
            c = InstanceClient()
            with patch("minihou.link.instance_client.socket.create_connection", return_value=sock), \
                 patch("minihou.link.instance_client.send_message"), \
                 patch("minihou.link.instance_client.recv_message", return_value=({**good, **override}, {})):
                with self.assertRaises(RuntimeError):
                    c.connect()
            self.assertIsNone(c.sock)
            sock.close.assert_called_once()

    def test_bad_correlation_type_and_malformed_reply_close_socket(self):
        for reply in ({"request_id": 999, "type": "instance_description"},
                      {"request_id": 1, "type": "instance_result"},
                      {"request_id": 1, "type": "instance_description", "instance_id": "wrong"}, None):
            c = InstanceClient()
            sock = c.sock = Mock()
            with patch("minihou.link.instance_client.send_message"), \
                 patch("minihou.link.instance_client.recv_message", return_value=(reply, {})):
                with self.assertRaises((RuntimeError, AttributeError)):
                    c.describe("")
            self.assertIsNone(c.sock)
            sock.close.assert_called_once()


if __name__ == "__main__":
    unittest.main()
