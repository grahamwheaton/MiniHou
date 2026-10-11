"""End-to-end: Session + LinkServer + LinkClient over real loopback sockets."""
import json
import os
import queue
import tempfile
import time
import unittest

import numpy as np

from minihou import Evaluator, Graph
from minihou.link.client import LinkClient
from minihou.link.protocol import GeoUpdate, Mirror, TOPO_KEYS, TOPO_REQUIRED
from minihou.link.server import LinkServer
from minihou.link.session import Session
from tests.helpers import graph_from

SIMPLE = {
    "g": {"type": "grid", "params": {"columns": 20, "rows": 20}},
    "n": {"type": "noise", "params": {"amplitude": [0, 0, 1], "frequency": 0.5}, "inputs": {"geo": "g"}},
    "o": {"type": "output", "inputs": {"geo": "n"}},
}
EXPOSED = [{"name": "Height", "node": "n", "param": "amplitude"},
           {"name": "Columns", "node": "g", "param": "columns"},
           {"name": "Freq", "node": "n", "param": "frequency"}]


class Harness:
    """Server + one or more collecting clients."""

    def __init__(self, graph, watch_path=None):
        self.session = Session(graph)
        self.server = LinkServer(self.session, "127.0.0.1", 0)
        self.port = self.server.start()
        self.session.start()
        if watch_path:
            self.session.watch_file(watch_path, interval=0.05)
        self.clients = []

    def client(self):
        q = queue.Queue()
        c = LinkClient("127.0.0.1", self.port, on_message=lambda h, a: q.put((h, a)), client_name="test")
        c.start()
        self.clients.append(c)
        return c, q

    def close(self):
        for c in self.clients:
            c.stop()
        self.server.stop()
        self.session.stop()


def wait_for(q, type_, timeout=5.0, pred=None):
    end = time.monotonic() + timeout
    skipped = []
    while time.monotonic() < end:
        try:
            h, a = q.get(timeout=max(0.01, end - time.monotonic()))
        except queue.Empty:
            break
        if h["type"] == type_ and (pred is None or pred(h)):
            return h, a
        skipped.append(h["type"])
    raise AssertionError(f"timed out waiting for {type_!r} (saw {skipped})")


class LiveLinkTests(unittest.TestCase):
    def setUp(self):
        self.h = Harness(graph_from(SIMPLE, display="o", exposed=EXPOSED))
        self.client, self.q = self.h.client()

    def tearDown(self):
        self.h.close()

    def first_full(self):
        wait_for(self.q, "welcome")
        wait_for(self.q, "interface")
        h, a = wait_for(self.q, "geo")
        u = GeoUpdate.from_message(h, a)
        m = Mirror()
        m.apply(u)
        return m, u

    def test_handshake_interface_and_first_full_update(self):
        m, u = self.first_full()
        self.assertTrue(u.full and u.topology_changed)
        self.assertEqual(u.counts, {"point": 400, "vertex": 361 * 4, "prim": 361})
        self.assertTrue(set(TOPO_REQUIRED) <= set(u.arrays))
        expected = Evaluator().cook(self.h.session.graph)
        np.testing.assert_array_equal(m.P, expected.P)
        np.testing.assert_array_equal(m.vertex_point, expected.topology.vertex_point)

    def test_interface_describes_exposed_parameters(self):
        wait_for(self.q, "welcome")
        h, _ = wait_for(self.q, "interface")
        names = [p["name"] for p in h["params"]]
        self.assertEqual(names, ["Height", "Columns", "Freq"])
        cols = h["params"][1]
        self.assertEqual((cols["kind"], cols["min"], cols["value"]), ("int", 2, 20))

    def test_changing_a_noise_parameter_sends_only_positions(self):
        m, _ = self.first_full()
        self.client.set_param("Height", [0, 0, 3.0])
        h, a = wait_for(self.q, "geo")
        u = GeoUpdate.from_message(h, a)
        self.assertEqual(sorted(u.arrays), ["point/P"])            # not uv, not topology
        self.assertFalse(u.topology_changed or u.full)
        m.apply(u)
        np.testing.assert_array_equal(m.P, Evaluator().cook(self.h.session.graph).P)
        self.assertEqual(h["cook"]["cooked"], ["n", "o"])           # grid came from cache
        self.assertIn("g", h["cook"]["cached"])

    def test_changing_topology_sends_topology_and_dependent_arrays(self):
        m, _ = self.first_full()
        self.client.set_param("Columns", 10)
        h, a = wait_for(self.q, "geo")
        u = GeoUpdate.from_message(h, a)
        self.assertTrue(u.topology_changed)
        self.assertTrue(set(TOPO_REQUIRED) <= set(u.arrays))
        m.apply(u)
        self.assertEqual(m.counts["point"], 200)
        np.testing.assert_array_equal(m.P, Evaluator().cook(self.h.session.graph).P)

    def test_setting_a_value_that_changes_nothing_sends_nothing(self):
        self.first_full()
        self.client.set_param("Freq", 0.5)                         # same as current
        self.client.set_param("Freq", 0.9)
        h, _ = wait_for(self.q, "geo")
        self.assertEqual(h["seq"], 2)                              # exactly one new geo, not two

    def test_invalid_parameters_report_errors_and_keep_serving(self):
        self.first_full()
        self.client.set_param("Columns", 1)                        # below min
        h, _ = wait_for(self.q, "error")
        self.assertIn("minimum", h["message"])
        self.client.set_param("Nope", 1)
        h, _ = wait_for(self.q, "error")
        self.assertIn("Nope", h["message"])
        self.client.set_param("Columns", 8)
        wait_for(self.q, "geo")

    def test_burst_of_edits_is_coalesced_and_converges(self):
        m, _ = self.first_full()
        for i in range(60):
            self.client.set_param("Freq", 0.1 + i * 0.01)
        self.assertTrue(self.h.session.wait_idle(10))
        n_geo, deadline = 0, time.monotonic() + 5
        while time.monotonic() < deadline:
            try:
                h, a = self.q.get(timeout=0.3)
            except queue.Empty:
                break
            if h["type"] == "geo":
                m.apply(GeoUpdate.from_message(h, a))
                n_geo += 1
        self.assertGreaterEqual(n_geo, 1)
        self.assertLess(n_geo, 60)                                 # 60 edits, fewer cooks/sends
        np.testing.assert_array_equal(m.P, Evaluator().cook(self.h.session.graph).P)

    def test_resync_resends_everything(self):
        self.first_full()
        self.client.resync()
        h, a = wait_for(self.q, "geo")
        self.assertTrue(h["full"])
        self.assertTrue(set(TOPO_REQUIRED) <= set(a))

    def test_second_client_gets_a_full_update_of_current_state(self):
        self.first_full()
        self.client.set_param("Height", [0, 0, 2.0])
        wait_for(self.q, "geo")
        c2, q2 = self.h.client()
        wait_for(q2, "welcome")
        h, a = wait_for(q2, "geo")
        self.assertTrue(h["full"])
        m = Mirror()
        m.apply(GeoUpdate.from_message(h, a))
        np.testing.assert_array_equal(m.P, Evaluator().cook(self.h.session.graph).P)

    def test_both_clients_receive_updates(self):
        self.first_full()
        c2, q2 = self.h.client()
        wait_for(q2, "geo")
        self.client.set_param("Freq", 0.77)
        wait_for(self.q, "geo")
        wait_for(q2, "geo")

    def test_ping(self):
        self.first_full()
        self.client.send({"type": "ping", "t": 123})
        h, _ = wait_for(self.q, "pong")
        self.assertEqual(h["t"], 123)

    def test_protocol_version_mismatch_is_reported(self):
        import socket
        from minihou.link.protocol import recv_message, send_message
        s = socket.create_connection(("127.0.0.1", self.h.port))
        send_message(s, {"type": "hello", "protocol": 999})
        h, _ = recv_message(s)
        s.close()
        self.assertEqual(h["type"], "error")
        self.assertIn("protocol", h["message"])

    def test_server_survives_a_client_vanishing(self):
        self.first_full()
        c2, q2 = self.h.client()
        wait_for(q2, "geo")
        c2.stop()
        self.client.set_param("Freq", 0.31)
        wait_for(self.q, "geo")


class PortTests(unittest.TestCase):
    def test_busy_port_gives_a_readable_error_quickly(self):
        g = graph_from(SIMPLE, display="o")
        a = LinkServer(Session(g), "127.0.0.1", 0)
        port = a.start()
        try:
            b = LinkServer(Session(g), "127.0.0.1", port, bind_retry_seconds=0.3)
            t0 = time.monotonic()
            with self.assertRaises(OSError) as cm:
                b.start()
            self.assertIn("already in use", str(cm.exception))
            self.assertLess(time.monotonic() - t0, 3.0)
        finally:
            a.stop()

    def test_port_can_be_rebound_right_after_stop(self):
        g = graph_from(SIMPLE, display="o")
        a = LinkServer(Session(g), "127.0.0.1", 0)
        port = a.start()
        s = Session(g)
        c = LinkClient("127.0.0.1", port, reconnect=False)
        c.start()
        time.sleep(0.3)
        a.stop()
        c.stop()
        b = LinkServer(s, "127.0.0.1", port)
        self.assertEqual(b.start(), port)
        b.stop()


class ErrorAndReloadTests(unittest.TestCase):
    def test_cook_error_is_reported_then_recovers_with_consistent_state(self):
        from minihou.sdk import Float, node
        from minihou import Geometry, REGISTRY
        state = {"fail": False}

        @node("sometimes_fails", inputs=["geo"], params=[Float("k", 1.0)], replace=True)
        def sometimes_fails(geo, k):
            if state["fail"]:
                raise RuntimeError("kaboom")
            out = geo.copy()
            out.point["P"] = geo.P * np.float32(k)
            return out

        try:
            g = graph_from({"g": {"type": "grid", "params": {"columns": 5, "rows": 5}},
                            "s": {"type": "sometimes_fails", "inputs": {"geo": "g"}},
                            "o": {"type": "output", "inputs": {"geo": "s"}}}, display="o",
                           exposed=[{"name": "K", "node": "s", "param": "k"}])
            h = Harness(g)
            try:
                c, q = h.client()
                hdr, arr = wait_for(q, "geo")
                m = Mirror()
                m.apply(GeoUpdate.from_message(hdr, arr))
                state["fail"] = True
                c.set_param("K", 2.0)
                err, _ = wait_for(q, "error", pred=lambda x: x.get("where") == "cook")
                self.assertEqual(err["node"], "s")
                self.assertIn("kaboom", err["message"])
                state["fail"] = False
                c.set_param("K", 3.0)
                hdr, arr = wait_for(q, "geo")
                u = GeoUpdate.from_message(hdr, arr)
                self.assertEqual(sorted(u.arrays), ["point/P"])      # diffed against the last good state
                m.apply(u)
                np.testing.assert_allclose(m.P, Evaluator().cook(h.session.graph).P)
            finally:
                h.close()
        finally:
            del REGISTRY._types["sometimes_fails"]

    def test_hot_reload_of_graph_file_updates_interface_and_geometry(self):
        with tempfile.TemporaryDirectory() as d:
            path = os.path.join(d, "g.json")
            data = {"version": 1, "nodes": json.loads(json.dumps(SIMPLE)), "display": "o", "exposed": EXPOSED[:1]}
            with open(path, "w", encoding="utf-8") as f:
                json.dump(data, f)
            h = Harness(Graph.load(path), watch_path=path)
            try:
                c, q = h.client()
                wait_for(q, "geo")
                data["nodes"]["g"]["params"]["columns"] = 7
                data["exposed"] = EXPOSED
                time.sleep(0.1)
                with open(path, "w", encoding="utf-8") as f:
                    json.dump(data, f)
                iface, _ = wait_for(q, "interface", pred=lambda x: len(x["params"]) == 3)
                self.assertEqual([p["name"] for p in iface["params"]], ["Height", "Columns", "Freq"])
                hdr, arr = wait_for(q, "geo")
                self.assertEqual(hdr["counts"]["point"], 7 * 20)
                # a broken edit is reported, and the last good geometry keeps being served
                time.sleep(0.1)
                with open(path, "w", encoding="utf-8") as f:
                    f.write("{ not json")
                err, _ = wait_for(q, "error")
                self.assertIn("g.json", err["message"])
                time.sleep(0.1)
                data["nodes"]["g"]["params"]["columns"] = 9
                with open(path, "w", encoding="utf-8") as f:
                    json.dump(data, f)
                hdr, arr = wait_for(q, "geo")
                self.assertEqual(hdr["counts"]["point"], 9 * 20)
            finally:
                h.close()


if __name__ == "__main__":
    unittest.main()
