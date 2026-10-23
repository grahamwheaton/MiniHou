"""Blender attributes reach the payload sent to MiniHou (no bpy: a fake mesh)."""
import ast
import importlib
import types
import unittest

import numpy as np

from tests import test_rust_link  # loads the shipped bpy-free add-on package
from tests.test_edge_link import Attributes, Collection

reader = importlib.import_module(test_rust_link.PKG + ".mesh_reader")


def attr(name, data_type, domain, prop, values):
    return types.SimpleNamespace(name=name, data_type=data_type, domain=domain,
                                 data=Collection(types.SimpleNamespace(**{prop: v}) for v in values))


class _Colors(list):
    render_color_index = 0


def fake_mesh():
    corner = 3
    return types.SimpleNamespace(
        vertices=Collection(types.SimpleNamespace(co=p) for p in [(0, 0, 0), (1, 0, 0), (0, 1, 0)]),
        loops=Collection(types.SimpleNamespace(vertex_index=i) for i in range(corner)),
        polygons=Collection([types.SimpleNamespace(loop_start=0)]), edges=Collection(),
        attributes=Attributes([
            attr("temperature", "FLOAT", "POINT", "value", [1.5, 2.5, 3.5]),
            attr("offset", "FLOAT_VECTOR", "POINT", "vector", [(1, 2, 3)] * 3),
            attr("Col", "FLOAT_COLOR", "CORNER", "color", [(0.1, 0.2, 0.3, 1.0)] * corner),
            attr("PCol", "BYTE_COLOR", "POINT", "color", [(1, 0, 0, 0.5)] * 3),
            attr("UVMap", "FLOAT2", "CORNER", "vector", [(0, 0), (1, 0), (0, 1)]),
            attr("UV2", "FLOAT2", "CORNER", "vector", [(0.5, 0.5)] * corner),
            attr("region", "INT", "FACE", "value", [7]),
            attr("flag", "BOOLEAN", "FACE", "value", [True]),
            attr(".hidden", "FLOAT", "POINT", "value", [0, 0, 0]),
        ]),
        uv_layers=[types.SimpleNamespace(name="UVMap", active_render=True), types.SimpleNamespace(name="UV2", active_render=False)],
        color_attributes=_Colors([types.SimpleNamespace(name="Col"), types.SimpleNamespace(name="PCol")]),
        has_custom_normals=False)


class AttributePayloadTests(unittest.TestCase):
    def test_every_domain_and_type_is_kept(self):
        me = fake_mesh()
        arrays, info = reader._read_mesh(types.SimpleNamespace(vertex_groups=[]), me)
        np.testing.assert_allclose(arrays["point/temperature"], [1.5, 2.5, 3.5])
        self.assertEqual(arrays["point/offset"].shape, (3, 3))
        np.testing.assert_allclose(arrays["vertex/Cd"], [(0.1, 0.2, 0.3)] * 3, rtol=1e-6)   # render colour
        self.assertEqual(arrays["point/PCol"].shape, (3, 4))                                  # other colours keep their names
        np.testing.assert_allclose(arrays["vertex/uv"], [(0, 0), (1, 0), (0, 1)])             # render UV map
        self.assertEqual(arrays["vertex/UV2"].shape, (3, 2))
        np.testing.assert_array_equal(arrays["prim/region"], [7])
        np.testing.assert_array_equal(arrays["prim/flag"], [1])
        self.assertEqual(arrays["prim/flag"].dtype, np.int32)
        self.assertNotIn("point/.hidden", arrays)
        self.assertEqual(info["skipped"], [])

    def test_modifier_prefix_read_keeps_all_data_layers(self):
        """The modifier input (read_prefix) must ask to_mesh for every data layer, as read_object does."""
        source = (test_rust_link.ROOT / "blender_addon/minihou_link/modifiers.py").read_text(encoding="utf-8")
        fn = next(n for n in ast.parse(source).body if isinstance(n, ast.FunctionDef) and n.name == "read_prefix")
        calls = [c for c in ast.walk(fn) if isinstance(c, ast.Call) and getattr(c.func, "attr", "") == "to_mesh"]
        self.assertTrue(calls)
        for call in calls:
            kw = {k.arg: k.value for k in call.keywords}
            self.assertIn("preserve_all_data_layers", kw)
            self.assertIs(kw["preserve_all_data_layers"].value, True)
            self.assertIn("depsgraph", kw)


if __name__ == "__main__":
    unittest.main()
