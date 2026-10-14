"""Real Blender tests of the collection plumbing: which collection a read object reports, and the membership messages the runtime
sends for watched collections. Skipped without bpy; run by tools/blender_selftest.py on CI."""
import sys
import unittest
from pathlib import Path

try:
    import bpy
except ImportError:                                    # pragma: no cover
    bpy = None

ROOT = Path(__file__).resolve().parent.parent
sys.path[:0] = [str(ROOT), str(ROOT / "blender_addon")]


class FakeClient:
    def __init__(self):
        self.sent = []

    def send(self, header, arrays=None):
        self.sent.append(header)
        return True


@unittest.skipUnless(bpy is not None, "bpy is not installed")
class CollectionTests(unittest.TestCase):
    def setUp(self):
        bpy.ops.wm.read_factory_settings(use_empty=True)

    def cube(self, name, collection=None):
        me = bpy.data.meshes.new(name)
        me.from_pydata([(0, 0, 0), (1, 0, 0), (1, 1, 0)], [], [(0, 1, 2)])
        ob = bpy.data.objects.new(name, me)
        (collection or bpy.context.scene.collection).objects.link(ob)
        return ob

    def test_read_object_reports_its_real_collection_not_the_scene_master(self):
        from minihou_link import mesh_reader
        props = bpy.data.collections.new("Props")
        bpy.context.scene.collection.children.link(props)
        inside, loose = self.cube("A", props), self.cube("B")
        dg = bpy.context.evaluated_depsgraph_get()
        self.assertEqual(mesh_reader.read_object(inside, dg)[0]["info"]["collection"], "Props")
        self.assertEqual(mesh_reader.read_object(loose, dg)[0]["info"]["collection"], "")      # in the scene's root collection
        from minihou_link import modifiers
        self.assertEqual(modifiers._picked(inside), ("A", "Props"))
        self.assertEqual(modifiers._picked(loose), ("B", ""))

    def test_membership_is_sent_on_change_and_includes_child_collections(self):
        from minihou_link import runtime
        rt = runtime.Runtime()
        rt.client = FakeClient()
        rt.watched_collections = ["Props", "Nope"]
        props = bpy.data.collections.new("Props")
        bpy.context.scene.collection.children.link(props)
        child = bpy.data.collections.new("Child")
        props.children.link(child)
        self.cube("A", props)
        self.cube("C", child)
        rt._last_collection_poll = 0.0
        self.assertTrue(rt._send_collections())
        msgs = {m["collection"]: m for m in rt.client.sent}
        self.assertEqual(msgs["Props"]["objects"], ["A", "C"])
        self.assertEqual(msgs["Props"]["error"], "")
        self.assertIn("no collection named", msgs["Nope"]["error"])
        rt.client.sent.clear()
        rt._last_collection_poll = 0.0
        self.assertFalse(rt._send_collections())                          # unchanged: nothing is sent again
        bpy.data.objects["A"].name = "A2"
        rt._last_collection_poll = 0.0
        self.assertTrue(rt._send_collections())
        self.assertEqual(rt.client.sent[0]["objects"], ["A2", "C"])


if __name__ == "__main__":
    unittest.main()
