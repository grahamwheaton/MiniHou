"""Object scale and edge attributes through a real MiniHou modifier cook (Blender + the Rust link server).

    blender --background --factory-startup --python tests/test_blender_scale_attrs.py

Needs the debug ``link_selftest`` example (see tests/test_blender_rust_placements.py for how to build it, or set
MINIHOU_LINK_SELFTEST).
"""
import json
import sys
import tempfile
import unittest
from pathlib import Path

import numpy as np

try:
    import bpy
except ImportError:
    bpy = None

ROOT = Path(__file__).resolve().parent.parent
sys.path[:0] = [str(ROOT), str(ROOT / "blender_addon")]

if bpy is not None:
    from tests.test_blender_rust_placements import RustEngine

TRANSFORM = ROOT / "examples" / "modifiers" / "transform.json"
QUAD = [(-1, -1, 0), (1, -1, 0), (1, 1, 0), (-1, 1, 0)]


def evaluated(obj):
    """(positions, {edge pair: bevel weight}) of the evaluated mesh in object space."""
    dg = bpy.context.evaluated_depsgraph_get()
    ev = obj.evaluated_get(dg)
    me = ev.to_mesh(preserve_all_data_layers=True, depsgraph=dg)
    try:
        co = np.empty(len(me.vertices) * 3, np.float32)
        me.vertices.foreach_get("co", co)
        weights = {}
        a = me.attributes.get("bevel_weight_edge")
        if a is not None:
            v = np.empty(len(me.edges), np.float32)
            a.data.foreach_get("value", v)
            weights = {tuple(sorted(e.vertices)): round(float(v[e.index]), 4) for e in me.edges}
        return co.reshape(-1, 3), weights
    finally:
        ev.to_mesh_clear()


@unittest.skipIf(bpy is None, "requires actual Blender")
class ScaleAndAttributeTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.engine = RustEngine(TRANSFORM)

    @classmethod
    def tearDownClass(cls):
        cls.engine.stop()

    def setUp(self):
        from minihou_link import modifiers, proxy
        from minihou_link.instance_client import InstanceClient
        self.m, self.proxy = modifiers, proxy
        modifiers.disconnect()
        bpy.ops.wm.read_factory_settings(use_empty=True)
        self.sent = []
        original = InstanceClient.cook

        def cook(client, instance_id, graph_path, params, arrays, **kw):
            self.sent.append(np.array(arrays["point/P"], np.float64))
            return original(client, instance_id, graph_path, params, arrays, **kw)
        InstanceClient.cook = cook
        self.addCleanup(setattr, InstanceClient, "cook", original)
        self.addCleanup(modifiers.disconnect)

    def plane(self, scale=(1, 1, 1)):
        mesh = bpy.data.meshes.new("Plane")
        mesh.from_pydata(QUAD, [], [(0, 1, 2, 3)])
        obj = bpy.data.objects.new("Plane", mesh)
        obj.scale = scale
        bpy.context.scene.collection.objects.link(obj)
        return obj

    def modifier(self, obj, **values):
        mod = self.m.add_modifier(obj, str(TRANSFORM), port=self.engine.port)
        self.proxy.write_values(mod, mod.node_group, self.m.parameters(mod), values)
        count = self.m.pump(port=self.engine.port, force=True)
        self.assertEqual(self.m.last_error(), "")
        bpy.context.view_layer.update()
        return mod

    def test_minihou_receives_the_scene_sized_geometry_and_blender_gets_object_space_back(self):
        obj = self.plane(scale=(4, 2, 1))
        self.modifier(obj, Scale=1.0, Translate=(0, 0, 0))
        np.testing.assert_allclose(self.sent[-1][:, :2], np.array(QUAD, np.float64)[:, :2] * [4, 2], atol=1e-5)
        co, _ = evaluated(obj)
        np.testing.assert_allclose(co, np.array(QUAD, np.float32), atol=1e-5)        # round trip changes nothing

    def test_a_translate_in_minihou_is_in_scene_units(self):
        obj = self.plane(scale=(4, 2, 1))
        self.modifier(obj, Scale=1.0, Translate=(8, 0, 0))
        co, _ = evaluated(obj)
        # 8 scene units on a 4x-scaled axis is 2 units in object space
        np.testing.assert_allclose(co[:, 0], np.array(QUAD, np.float32)[:, 0] + 2, atol=1e-5)

    def test_unscaled_objects_are_untouched(self):
        obj = self.plane()
        self.modifier(obj, Scale=1.0, Translate=(0, 0, 0))
        np.testing.assert_allclose(self.sent[-1], np.array(QUAD, np.float64), atol=1e-6)

    def test_bevel_weights_survive_the_round_trip_on_a_scaled_object(self):
        obj = self.plane(scale=(15.421, 15.421, 1))
        a = obj.data.attributes.new("bevel_weight_edge", "FLOAT", "EDGE")
        pairs = [tuple(sorted(e.vertices)) for e in obj.data.edges]
        want = {pair: 0.25 * (i + 1) for i, pair in enumerate(pairs)}
        a.data.foreach_set("value", [want[p] for p in pairs])
        self.modifier(obj, Scale=1.0, Translate=(0, 0, 0))
        _, got = evaluated(obj)
        self.assertEqual(got, {p: round(v, 4) for p, v in want.items()})

    def cube_with_weights(self, scale=(1, 1, 1)):
        mesh = bpy.data.meshes.new("Cube")
        bpy.context.scene.collection.objects.link(bpy.data.objects.new("Cube", mesh))
        obj = bpy.data.objects["Cube"]
        bpy.ops.object.select_all(action="DESELECT")
        bm_pts = [(x, y, z) for z in (-1, 1) for y in (-1, 1) for x in (-1, 1)]
        faces = [(0, 1, 3, 2), (4, 6, 7, 5), (0, 4, 5, 1), (2, 3, 7, 6), (0, 2, 6, 4), (1, 5, 7, 3)]
        mesh.from_pydata(bm_pts, [], faces)
        obj.scale = scale
        a = mesh.attributes.new("bevel_weight_edge", "FLOAT", "EDGE")
        a.data.foreach_set("value", [1.0] * len(mesh.edges))           # every edge weighted
        return obj

    def bevel_after(self, obj):
        bev = obj.modifiers.new("Bevel", "BEVEL")
        bev.limit_method = "WEIGHT"
        bev.width = 0.1
        bev.segments = 1
        bpy.context.view_layer.update()
        return len(evaluated(obj)[0])

    def test_a_bevel_after_minihou_still_sees_the_weights(self):
        plain = self.cube_with_weights(scale=(15.421, 15.421, 1))
        before = self.bevel_after(plain)                                  # Blender alone: all edges bevel
        self.assertGreater(before, 8)
        bpy.data.objects.remove(plain)
        obj = self.cube_with_weights(scale=(15.421, 15.421, 1))
        mod = self.modifier(obj, Scale=1.0, Translate=(0, 0, 0))
        obj.modifiers.move(len(obj.modifiers) - 1, 0) if False else None
        after = self.bevel_after(obj)
        self.assertEqual(after, before, "the weights did not reach the Bevel modifier after MiniHou")

    def test_weights_survive_a_second_cook_after_a_parameter_change(self):
        obj = self.cube_with_weights(scale=(3, 3, 1))
        mod = self.modifier(obj, Scale=1.0, Translate=(0, 0, 0))
        self.proxy.write_values(mod, mod.node_group, self.m.parameters(mod), {"Scale": 2.0})
        self.m.pump(port=self.engine.port)
        self.assertEqual(self.m.last_error(), "")
        bpy.context.view_layer.update()
        co, weights = evaluated(obj)
        self.assertEqual(len(weights), 12)
        self.assertTrue(all(w == 1.0 for w in weights.values()), weights)


if __name__ == "__main__":
    unittest.main(argv=["x"], exit=bpy is None)
