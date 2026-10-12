"""Several outputs: extra `output` nodes with an `object` name publish to their own Blender objects."""
import queue
import unittest

import numpy as np

from minihou import Evaluator, Graph
from minihou.edit import apply_ops
from minihou.link.protocol import GeoUpdate, Mirror
from minihou.link.session import Session
from tests.helpers import graph_from
from tests.test_link import Harness, wait_for

TWO = {
    "g": {"type": "grid", "params": {"columns": 6, "rows": 6}},
    "b": {"type": "box"},
    "main": {"type": "output", "inputs": {"geo": "g"}},
    "trees": {"type": "output", "params": {"object": "Trees"}, "inputs": {"geo": "b"}},
}


class GraphRulesTests(unittest.TestCase):
    def test_extra_outputs_and_the_main_one(self):
        g = graph_from(TWO, display="main")
        self.assertEqual(g.extra_outputs(), {"Trees": "trees"})
        self.assertEqual(g.output_node(), "main")
        g2 = graph_from(TWO)                                        # no display flag: the first output without an object
        self.assertEqual(g2.output_node(), "main")
        self.assertEqual(g2.extra_outputs(), {"Trees": "trees"})
        flipped = {k: v for k, v in TWO.items() if k != "main"}
        g3 = graph_from(flipped)                                    # only named outputs: the first stays the main one
        self.assertEqual((g3.output_node(), g3.extra_outputs()), ("trees", {}))

    def test_the_display_node_is_never_an_extra_and_duplicates_collapse(self):
        g = graph_from(TWO, display="trees")
        self.assertEqual((g.output_node(), g.extra_outputs()), ("trees", {}))
        nodes = dict(TWO, dup={"type": "output", "params": {"object": "Trees"}, "inputs": {"geo": "g"}},
                     blank={"type": "output", "params": {"object": "  "}, "inputs": {"geo": "g"}})
        self.assertEqual(graph_from(nodes, display="main").extra_outputs(), {"Trees": "trees"})

    def test_extra_outputs_do_not_change_the_main_cook(self):
        g = graph_from(TWO, display="main")
        a = Evaluator().cook(g)
        h, _ = apply_ops(g, [{"op": "set", "id": "trees", "params": {"object": ""}}])
        self.assertEqual(Evaluator().cook(h).npoints, a.npoints)


class LiveOutputTests(unittest.TestCase):
    def setUp(self):
        self.h = Harness(graph_from(TWO, display="main"))
        self.client, self.q = self.h.client()
        self.addCleanup(self.h.close)

    def extra(self, name="Trees", **kw):
        return wait_for(self.q, "geo", pred=lambda h: h.get("output") == name, **kw)

    def test_the_named_output_arrives_as_its_own_stream(self):
        wait_for(self.q, "welcome")
        main, ma = wait_for(self.q, "geo", pred=lambda h: h.get("output") is None)
        mm = Mirror()
        mm.apply(GeoUpdate.from_message(main, ma))
        self.assertEqual(mm.counts["point"], 36)                    # the main stream is the grid, untouched
        names, _ = wait_for(self.q, "outputs")
        self.assertEqual(names["names"], ["Trees"])
        h, a = self.extra()
        u = GeoUpdate.from_message(h, a)
        self.assertEqual((u.output, u.full), ("Trees", True))
        m = Mirror()
        m.apply(u)
        box = Evaluator().cook(self.h.session.graph, "trees")
        np.testing.assert_array_equal(m.P, box.P)

    def test_only_the_changed_output_resends_and_only_changes_are_diffed(self):
        h, a = self.extra()
        m = Mirror()
        m.apply(GeoUpdate.from_message(h, a))
        self.h.session.edit([{"op": "set", "id": "b", "params": {"size": [2, 2, 2]}}], origin="test")
        h2, a2 = self.extra()
        u = GeoUpdate.from_message(h2, a2)
        self.assertFalse(u.full or u.topology_changed)               # same topology: positions only
        self.assertEqual(sorted(u.arrays), ["point/P"])
        m.apply(u)
        np.testing.assert_array_equal(m.P, Evaluator().cook(self.h.session.graph, "trees").P)
        # editing the main branch does not resend the extra output
        while True:
            try:
                self.q.get_nowait()
            except queue.Empty:
                break
        self.h.session.edit([{"op": "set", "id": "g", "params": {"columns": 8}}], origin="test")
        wait_for(self.q, "geo", pred=lambda h: h.get("output") is None)
        with self.assertRaises(AssertionError):
            self.extra(timeout=0.5)

    def test_removing_the_object_name_announces_the_new_list(self):
        wait_for(self.q, "outputs")
        self.h.session.edit([{"op": "set", "id": "trees", "params": {"object": ""}}], origin="test")
        names, _ = wait_for(self.q, "outputs", pred=lambda h: h["names"] == [])
        self.assertEqual(names["names"], [])

    def test_a_failing_extra_output_is_reported_and_leaves_the_main_output_alone(self):
        wait_for(self.q, "outputs")
        self.h.session.edit([{"op": "add", "type": "wrangle", "id": "bad", "inputs": {"geo": "b"},
                              "params": {"snippet": "@P.z = @nope"}},
                             {"op": "connect", "from": "bad", "to": "trees", "port": "geo"}], origin="test")
        err, _ = wait_for(self.q, "error", pred=lambda h: "Trees" in h.get("message", ""))
        self.assertIn("nope", err["message"])
        self.assertTrue(self.h.session.status()["ok"])               # the main output is fine

    def test_resync_resends_extra_outputs_in_full(self):
        self.extra()
        self.client.resync()
        h, _ = self.extra()
        self.assertTrue(h["full"])


if __name__ == "__main__":
    unittest.main()
