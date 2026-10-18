"""Pair-keyed Blender edge transfer with small bpy collection mocks."""
import importlib
import sys
import types
import unittest
from unittest.mock import patch

import numpy as np

from tests import test_rust_link  # loads the shipped bpy-free add-on package

PKG = test_rust_link.PKG
reader = importlib.import_module(PKG + ".mesh_reader")


class Collection(list):
    def foreach_get(self, prop, out):
        out[:] = np.asarray([getattr(row, prop) for row in self]).reshape(-1)


class Attributes(list):
    def __getitem__(self, key):
        if isinstance(key, str):
            raise KeyError(key)
        return super().__getitem__(key)


def attribute(name, data_type, domain, values):
    return types.SimpleNamespace(name=name, data_type=data_type, domain=domain,
                                 data=Collection(types.SimpleNamespace(value=v) for v in values))


class EdgeLinkTests(unittest.TestCase):
    def test_reader_sends_pairs_and_edge_values_in_blender_edge_order(self):
        me = types.SimpleNamespace(
            vertices=Collection(types.SimpleNamespace(co=p) for p in [(0, 0, 0)] * 4),
            loops=Collection(types.SimpleNamespace(vertex_index=i, edge_index=i) for i in range(4)),
            polygons=Collection([types.SimpleNamespace(loop_start=0)]),
            edges=Collection(types.SimpleNamespace(vertices=p) for p in [(2, 3), (1, 0), (3, 0), (1, 2)]),
            attributes=Attributes([attribute("crease_edge", "FLOAT", "EDGE", [.3, .1, .4, .2]),
                                   attribute("sharp_edge", "BOOLEAN", "EDGE", [0, 1, 0, 1]),
                                   attribute("uv_seam", "BOOLEAN", "EDGE", [1, 0, 0, 1]),
                                   attribute("custom_int", "INT", "EDGE", [3, 1, 4, 2])]),
            uv_layers=[], color_attributes=types.SimpleNamespace(render_color_index=-1),
            has_custom_normals=False)
        obj = types.SimpleNamespace(vertex_groups=[])
        arrays, _ = reader._read_mesh(obj, me)
        np.testing.assert_array_equal(arrays["topo/edge_pairs"], [[2, 3], [0, 1], [0, 3], [1, 2]])
        np.testing.assert_allclose(arrays["edge/crease_edge"], [.3, .1, .4, .2])
        np.testing.assert_array_equal(arrays["edge/sharp_edge"], [0, 1, 0, 1])
        np.testing.assert_array_equal(arrays["edge/seam"], [1, 0, 0, 1])
        np.testing.assert_array_equal(arrays["edge/custom_int"], [3, 1, 4, 2])

    def test_writer_matches_pairs_and_zeroes_unmatched_mesh_edges(self):
        with patch.dict(sys.modules, {"bpy": types.ModuleType("bpy")}):
            writer = importlib.import_module(PKG + ".mesh_writer")
        edges = Collection(types.SimpleNamespace(vertices=p) for p in [(1, 2), (3, 0), (0, 2), (1, 0)])
        mesh = types.SimpleNamespace(edges=edges)
        pairs = np.array([[0, 1], [1, 2], [0, 3]], np.int32)
        values = np.array([10, 20, 30], np.int32)
        np.testing.assert_array_equal(writer.edge_values(mesh, pairs, values), [20, 30, 0, 10])

    def test_combining_components_shifts_edge_pairs(self):
        parts = importlib.import_module(PKG + ".geometry_parts")
        cloud = parts.point_arrays(np.zeros((2, 3), np.float32))
        mesh = {"point/P": np.zeros((3, 3), np.float32),
                "topo/vertex_point": np.array([0, 1, 2], np.int32),
                "topo/prim_offsets": np.array([0, 3], np.int32),
                "topo/edge_pairs": np.array([[0, 1], [1, 2], [0, 2]], np.int32),
                "edge/crease_edge": np.array([.1, .2, .3], np.float32)}
        result = parts.combine([cloud, mesh])
        np.testing.assert_array_equal(result["topo/edge_pairs"], [[2, 3], [3, 4], [2, 4]])
        np.testing.assert_allclose(result["edge/crease_edge"], [.1, .2, .3])
        parts.check(result)


if __name__ == "__main__":
    unittest.main()
