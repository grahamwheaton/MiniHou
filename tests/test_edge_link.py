"""Pair-keyed Blender edge transfer with small bpy collection mocks."""
import ast
import importlib
from pathlib import Path
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


def custom_attribute_object():
    def make_mesh(attributes):
        return types.SimpleNamespace(
            vertices=Collection(types.SimpleNamespace(co=p) for p in [(0, 0, 0), (1, 0, 0), (0, 1, 0)]),
            loops=Collection(types.SimpleNamespace(vertex_index=i, edge_index=i) for i in range(3)),
            polygons=Collection([types.SimpleNamespace(loop_start=0)]), edges=Collection(),
            attributes=Attributes(attributes), uv_layers=[],
            color_attributes=types.SimpleNamespace(render_color_index=-1), has_custom_normals=False)

    custom = [attribute("temperature", "FLOAT", "POINT", [1.5, 2.5, 3.5]),
              attribute("region", "INT", "FACE", [7])]
    graph = object()
    # Model Blender's to_mesh default: generic data layers are omitted unless requested.
    evaluated = types.SimpleNamespace(
        to_mesh=lambda preserve_all_data_layers=False, depsgraph=None: make_mesh(custom if preserve_all_data_layers and depsgraph is graph else []),
        to_mesh_clear=lambda: None, material_slots=[])
    obj = types.SimpleNamespace(name="Cube", type="MESH", vertex_groups=[],
                                matrix_world=np.eye(4), evaluated_get=lambda depsgraph: evaluated)
    return obj, graph


class EdgeLinkTests(unittest.TestCase):
    def test_automatic_connection_tries_live_default_before_stale_records(self):
        source = Path(test_rust_link.ROOT / "blender_addon/minihou_link/modifiers.py")
        function = next(node for node in ast.parse(source.read_text(encoding="utf-8")).body
                        if isinstance(node, ast.FunctionDef) and node.name == "_connection")
        attempts = []

        class FakeClient:
            def __init__(self, host, port, timeout):
                self.address = (host, port)
                self.timeout = timeout
                self.sock = types.SimpleNamespace(settimeout=lambda value: None)

            def connect(self):
                attempts.append((self.address, self.timeout))
                if self.address != ("127.0.0.1", 8765):
                    raise OSError("dead endpoint")
                return {"capabilities": ["live_modifier_v1", "object_inputs_v1"]}

            def close(self):
                self.sock = None

        stale = [("127.0.0.1", port) for port in range(9000, 9015)]
        discovery = types.ModuleType(PKG + ".discovery")
        discovery.candidates = lambda: stale
        instance_client = types.ModuleType(PKG + ".instance_client")
        instance_client.InstanceClient = FakeClient
        state = {"__package__": PKG, "bpy": types.SimpleNamespace(context=types.SimpleNamespace(
            preferences=types.SimpleNamespace(addons={}))), "_client": None, "_endpoint": None,
            "_settings": lambda host, port: ("127.0.0.1", 8765)}

        def disconnect():
            state["_client"] = state["_endpoint"] = None

        state["disconnect"] = disconnect
        with patch.dict(sys.modules, {PKG + ".discovery": discovery,
                                      PKG + ".instance_client": instance_client}):
            exec(compile(ast.Module(body=[function], type_ignores=[]), str(source), "exec"), state)
            client = state["_connection"]()
        self.assertIs(client, state["_client"])
        self.assertEqual(attempts, [(("127.0.0.1", 8765), 0.4)])

    def test_read_object_preserves_custom_mesh_attributes(self):
        obj, graph = custom_attribute_object()
        _, arrays = reader.read_object(obj, graph)
        np.testing.assert_allclose(arrays["point/temperature"], [1.5, 2.5, 3.5])
        np.testing.assert_array_equal(arrays["prim/region"], [7])

    def test_custom_mesh_attributes_reach_rust_blender_object(self):
        graph = {"nodes": {"source": {"type": "blender_object", "params": {"object": "Cube", "space": "object"}}},
                 "display": "source"}
        host = test_rust_link.Host(graph)
        self.addCleanup(host.close)
        client = host.client()
        client.wait("watch_objects")
        client.wait("interface")
        client.geo()  # initial waiting geometry
        obj, depsgraph = custom_attribute_object()
        fields, arrays = reader.read_object(obj, depsgraph)
        client.link.send({"type": "object_geo", "object": "Cube", **fields}, arrays)
        _, result = client.geo()
        np.testing.assert_allclose(result["point/temperature"], [1.5, 2.5, 3.5])
        np.testing.assert_array_equal(result["prim/region"], [7])

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
