"""Blender edge attributes (bevel_weight_edge, crease_edge, ...) survive a MiniHou modifier round trip.

Needs Cargo (builds the headless link_selftest example). Run: python -m unittest tests.test_edge_attrs_link -v
"""
from __future__ import annotations
import unittest

import numpy as np

from tests.test_rust_link import Host, MODIFIER

PIPE = {"nodes": {"input": {"type": "modifier_input"},
                  "out": {"type": "output", "inputs": {"geo": "input"}}}, "display": "out"}


def quad():
    """A unit quad with its four edges in an order that is not the engine's own."""
    pairs = np.array([[1, 2], [0, 1], [2, 3], [0, 3]], np.int32)
    return {"point/P": np.array([[0, 0, 0], [1, 0, 0], [1, 1, 0], [0, 1, 0]], np.float32),
            "topo/vertex_point": np.array([0, 1, 2, 3], np.int32),
            "topo/prim_offsets": np.array([0, 4], np.int32),
            "topo/edge_pairs": pairs,
            "edge/bevel_weight_edge": np.array([0.25, 0.5, 0.75, 1.0], np.float32),
            "edge/crease_edge": np.array([1, 0, 1, 0], np.float32)}


class EdgeAttributeTests(unittest.TestCase):
    def host(self, graph):
        host = Host(graph)
        self.addCleanup(host.close)
        return host

    def check(self, graph):
        client = self.host(graph).instances()
        header, arrays = client.cook("E", "", {}, quad(), graph=graph)
        self.assertEqual(header["type"], "instance_result", header)
        self.assertIn("topo/edge_pairs", arrays)
        pairs = [tuple(p) for p in arrays["topo/edge_pairs"].tolist()]
        sent = {(1, 2): 0.25, (0, 1): 0.5, (2, 3): 0.75, (0, 3): 1.0}
        for name, expect in (("edge/bevel_weight_edge", sent), ("edge/crease_edge", {(1, 2): 1, (0, 1): 0, (2, 3): 1, (0, 3): 0})):
            self.assertIn(name, arrays, sorted(arrays))
            got = dict(zip(pairs, arrays[name].tolist()))
            self.assertEqual(set(got), set(expect))
            for pair, value in expect.items():
                self.assertAlmostEqual(got[pair], value, places=5, msg=f"{name} {pair}")

    def test_passthrough_graph_keeps_edge_attributes(self):
        self.check(PIPE)

    def test_transform_graph_keeps_edge_attributes(self):
        self.check(MODIFIER)


if __name__ == "__main__":
    unittest.main()
