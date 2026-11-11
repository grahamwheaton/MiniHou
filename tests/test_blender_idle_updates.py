"""An idle MiniHou modifier must not tag the scene as changed (Cycles restarts its render on every depsgraph update).

    blender --background --factory-startup --python tests/test_blender_idle_updates.py

Needs the debug ``link_selftest`` example (see tests/test_blender_rust_placements.py), or MINIHOU_LINK_SELFTEST.
"""
import sys
import unittest
from pathlib import Path

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


@unittest.skipIf(bpy is None, "requires actual Blender")
class IdleUpdateTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.engine = RustEngine(TRANSFORM)

    @classmethod
    def tearDownClass(cls):
        cls.engine.stop()

    def test_idle_pumps_leave_the_depsgraph_alone(self):
        from minihou_link import modifiers, scene_objects
        modifiers.disconnect()
        bpy.ops.wm.read_factory_settings(use_empty=True)
        self.addCleanup(modifiers.disconnect)
        mesh = bpy.data.meshes.new("Plane")
        mesh.from_pydata(QUAD, [], [(0, 1, 2, 3)])
        obj = bpy.data.objects.new("Plane", mesh)
        bpy.context.scene.collection.objects.link(obj)
        modifiers.add_modifier(obj, str(TRANSFORM), port=self.engine.port)
        modifiers.pump(port=self.engine.port, force=True)
        self.assertEqual(modifiers.last_error(), "")
        for _ in range(3):                      # let the first cook settle (status, plumbing, instances)
            modifiers.pump(port=self.engine.port)
            bpy.context.view_layer.update()
        seen = []

        def record(scene, depsgraph):
            for update in depsgraph.updates:
                seen.append(f"{type(update.id).__name__}:{update.id.name}")

        bpy.app.handlers.depsgraph_update_post.append(record)
        self.addCleanup(bpy.app.handlers.depsgraph_update_post.remove, record)
        for _ in range(10):
            modifiers.pump(port=self.engine.port)
            scene_objects.flush()
            bpy.context.view_layer.update()
        self.assertEqual(seen, [], f"idle pumps tagged: {sorted(set(seen))}")

    def test_an_edit_to_the_source_mesh_is_still_picked_up(self):
        from minihou_link import modifiers
        modifiers.disconnect()
        bpy.ops.wm.read_factory_settings(use_empty=True)
        self.addCleanup(modifiers.disconnect)
        mesh = bpy.data.meshes.new("Plane")
        mesh.from_pydata(QUAD, [], [(0, 1, 2, 3)])
        obj = bpy.data.objects.new("Plane", mesh)
        bpy.context.scene.collection.objects.link(obj)
        mod = modifiers.add_modifier(obj, str(TRANSFORM), port=self.engine.port)
        for _ in range(3):
            modifiers.pump(port=self.engine.port)
            bpy.context.view_layer.update()
        cache = modifiers._cache(mod).data
        before = cache.vertices[0].co.x
        mesh.vertices[0].co.x += 3.0
        mesh.update()
        bpy.context.view_layer.update()
        modifiers.pump(port=self.engine.port)
        self.assertAlmostEqual(cache.vertices[0].co.x, before + 3.0, places=4)


if __name__ == "__main__":
    unittest.main(argv=[sys.argv[0]])
