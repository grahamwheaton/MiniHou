"""Real Blender tests of scene_objects: a MiniHou graph that makes real, separate scene objects (linked duplicates of a source
object) through the live link and through a modifier. Skipped without bpy; run by tools/blender_selftest.py on CI.

The point of the feature is that the user can work on the objects afterwards, so most checks are about *survival*: the same
objects (with the user's own modifiers on them) come back when the point count changes, and nothing is deleted by a cook.
"""
import json
import sys
import tempfile
import time
import unittest
from pathlib import Path

import numpy as np

try:
    import bpy
except ImportError:                                    # pragma: no cover
    bpy = None

ROOT = Path(__file__).resolve().parent.parent
sys.path[:0] = [str(ROOT), str(ROOT / "blender_addon")]
from tests.test_blender_addon import Engine  # noqa: E402


def forest(count=6, seed=1, objects="Tree", prefix="Forest", **so):
    return {"version": 1, "display": "main", "nodes": {
        "g": {"type": "grid", "params": {"columns": 4, "rows": 4, "size_x": 10, "size_y": 10}},
        "p": {"type": "scatter", "inputs": {"geo": "g"}, "params": {"count": count, "seed": seed}},
        "i": {"type": "instance", "inputs": {"points": "p"}, "params": {"objects": objects, "seed": 3}},
        "so": {"type": "scene_objects", "inputs": {"geo": "i"}, "params": dict({"prefix": prefix}, **so)},
        "main": {"type": "output", "inputs": {"geo": "g"}}}}


def scene_objs(tag="session:so"):
    return sorted((o for o in bpy.data.objects if o.get("minihou_scene") == tag), key=lambda o: o.get("minihou_id", 0))


@unittest.skipIf(bpy is None, "needs Blender / the bpy module")
class SceneObjectsLiveTests(unittest.TestCase):
    def setUp(self):
        bpy.ops.wm.read_factory_settings(use_empty=True)
        import minihou_link
        from minihou_link import runtime, scene_objects
        minihou_link.register()
        self.minihou_link, self.so = minihou_link, scene_objects
        self.tmp = tempfile.TemporaryDirectory()
        self.path = Path(self.tmp.name) / "g.json"
        self.write(forest())
        self.engine = Engine(self.path)
        self.rt = runtime.get_runtime()
        self.tree = self.source("Tree")
        self.rt.connect("127.0.0.1", self.engine.port)

    def tearDown(self):
        self.minihou_link.unregister()
        self.engine.stop()
        self.tmp.cleanup()

    def write(self, graph):
        self.path.write_text(json.dumps(graph), encoding="utf-8")

    def source(self, name):
        me = bpy.data.meshes.new(name)
        me.from_pydata([(0, 0, 0), (1, 0, 0), (0, 1, 0), (0, 0, 1)], [], [(0, 1, 2), (0, 1, 3), (0, 2, 3), (1, 2, 3)])
        ob = bpy.data.objects.new(name, me)
        bpy.context.scene.collection.objects.link(ob)
        ob.location = (50, 50, 50)
        return ob

    def pump(self, pred, timeout=60.0):
        end = time.monotonic() + timeout
        while time.monotonic() < end:
            self.rt.tick()
            if pred():
                return True
            time.sleep(0.01)
        self.fail(f"timed out; last_error={self.rt.last_error!r} log={list(self.rt.log)[-5:]}")

    def shown(self):
        return [o for o in scene_objs() if not o.get("minihou_hidden")]

    def test_points_become_real_linked_duplicates_in_their_own_collection(self):
        self.pump(lambda: len(scene_objs()) == 6)
        objs = scene_objs()
        self.assertEqual([o.name for o in objs], [f"Forest_{i:04d}" for i in range(6)])
        for o in objs:
            self.assertIs(o.data, self.tree.data)                                 # linked duplicate: one mesh for all
            self.assertEqual(o.type, "MESH")
        col = bpy.data.collections["Forest"]
        self.assertEqual(sorted(o.name for o in col.objects), [o.name for o in objs])
        self.assertIn(col.name, [c.name for c in bpy.context.scene.collection.children])
        # positions are the engine's points
        from minihou import Evaluator, Graph
        want = Evaluator().cook(Graph.from_dict(forest()), "i").P
        got = np.array([tuple(o.location) for o in objs])
        np.testing.assert_allclose(got, want, atol=1e-4)
        self.assertEqual(self.tree.location[0], 50)                              # the source itself is untouched

    def test_the_users_work_on_the_objects_survives_changes_in_the_count(self):
        self.pump(lambda: len(scene_objs()) == 6)
        mine = bpy.data.objects["Forest_0002"]
        mod = mine.modifiers.new("Mine", "SUBSURF")
        mine.color = (1, 0, 0, 1)
        self.write(forest(count=3))                                              # fewer points
        self.pump(lambda: len(self.shown()) == 3)
        self.assertEqual(len(scene_objs()), 6)                                   # nothing deleted: parked
        parked = [o for o in scene_objs() if o.get("minihou_hidden")]
        self.assertEqual(sorted(o.name for o in parked), ["Forest_0003", "Forest_0004", "Forest_0005"])
        for o in parked:
            self.assertTrue(o.hide_viewport and o.hide_render)
        self.assertIn("Mine", [m.name for m in bpy.data.objects["Forest_0002"].modifiers])    # still there
        self.write(forest(count=6, seed=1))                                      # and back
        self.pump(lambda: len(self.shown()) == 6)
        self.assertEqual(len(scene_objs()), 6)                                   # the same six objects, no new ones
        again = bpy.data.objects["Forest_0002"]
        self.assertIs(again, mine)
        self.assertIn("Mine", [m.name for m in again.modifiers])
        self.assertEqual(tuple(again.color), (1, 0, 0, 1))
        self.assertFalse(again.hide_viewport)

    def test_a_moved_point_moves_the_same_object(self):
        self.pump(lambda: len(scene_objs()) == 6)
        before = {o.name: tuple(o.location) for o in scene_objs()}
        self.write(forest(seed=9))
        self.pump(lambda: any(tuple(o.location) != before[o.name] for o in scene_objs()) and len(scene_objs()) == 6)
        self.assertEqual(sorted(before), [o.name for o in scene_objs()])

    def test_rotation_and_scale_come_from_the_points(self):
        self.write(forest(objects="Tree"))
        self.pump(lambda: len(scene_objs()) == 6)
        from minihou import Evaluator, Graph
        want = Evaluator().cook(Graph.from_dict(forest()), "i")
        for o, rot, sc in zip(scene_objs(), want.point["rot"], want.point["scale"]):
            np.testing.assert_allclose(tuple(o.rotation_euler), rot, atol=1e-5)
            np.testing.assert_allclose(tuple(o.scale), sc, atol=1e-4)

    def test_a_missing_source_gets_placeholders_that_become_real_when_it_appears(self):
        self.pump(lambda: len(scene_objs()) == 6)
        self.write(forest(objects="Pine"))
        self.pump(lambda: len(scene_objs()) == 6 and all(o.get("minihou_placeholder") for o in scene_objs()))
        self.assertTrue(all(o.type == "EMPTY" for o in scene_objs()))
        self.assertTrue(any("Pine" in line and "not found" in line for line in self.rt.log))
        pine = self.source("Pine")
        self.write(forest(objects="Pine", seed=2))                              # any cook will do; the planner notices the source
        self.pump(lambda: all(o.type == "MESH" for o in scene_objs()) and len(scene_objs()) == 6)
        self.assertTrue(all(o.data is pine.data for o in scene_objs()))

    def test_cleanup_deletes_parked_objects_only_when_asked(self):
        self.pump(lambda: len(scene_objs()) == 6)
        self.write(forest(count=2))
        self.pump(lambda: len(self.shown()) == 2)
        self.assertEqual(self.so.counts(), {"total": 6, "hidden": 4})
        self.assertEqual(self.so.cleanup(), 4)
        self.assertEqual(len(scene_objs()), 2)
        self.assertEqual(self.so.cleanup(everything=True), 2)
        self.assertEqual(self.so.counts()["total"], 0)
        self.assertIsNotNone(bpy.data.objects.get("Tree"))                       # the source is never ours to delete

    def test_removing_the_node_parks_its_objects_and_never_touches_the_users(self):
        mine = bpy.data.objects.new("Forest_0001", None)                         # the user's own object with a clashing name
        bpy.context.scene.collection.objects.link(mine)
        self.pump(lambda: len(scene_objs()) == 6)
        self.assertFalse(mine.get("minihou_scene"))
        g = forest()
        del g["nodes"]["so"]
        self.write(g)
        self.pump(lambda: len(self.shown()) == 0)
        self.assertEqual(len(scene_objs()), 6)
        self.assertIsNotNone(bpy.data.objects.get("Forest_0001"))
        self.assertFalse(mine.get("minihou_hidden"))

    def test_objects_are_found_by_marker_not_by_name(self):
        self.pump(lambda: len(scene_objs()) == 6)
        o = bpy.data.objects["Forest_0003"]
        o.name = "Renamed by the user"
        self.write(forest(seed=4))
        self.pump(lambda: len(scene_objs()) == 6 and any(x.name == "Renamed by the user" for x in scene_objs()))
        self.assertEqual(len(scene_objs()), 6)                                   # not seven

    def test_an_id_attribute_keeps_identity_when_the_order_changes(self):
        g = forest(id_attrib="id")
        g["nodes"]["w"] = {"type": "wrangle", "inputs": {"geo": "p"}, "params": {"snippet": "i@id = 100 + @ptnum;", "run_over": "points"}}
        g["nodes"]["i"]["inputs"] = {"points": "w"}
        self.pump(lambda: len(scene_objs()) == 6)
        self.write(g)
        self.pump(lambda: [o["minihou_id"] for o in self.shown()] == [100 + i for i in range(6)])
        self.assertEqual(len(scene_objs()), 12)                                  # the six index-named ones were parked, the six by id made
        self.assertEqual(len(self.shown()), 6)


@unittest.skipIf(bpy is None, "needs Blender / the bpy module")
class SceneObjectsModifierTests(unittest.TestCase):
    def setUp(self):
        from minihou_link import modifiers, proxy, scene_objects
        modifiers.unregister()
        bpy.ops.wm.read_factory_settings(use_empty=True)
        self.m, self.proxy, self.so = modifiers, proxy, scene_objects
        self.tmp = tempfile.TemporaryDirectory()
        self.path = Path(self.tmp.name) / "scatter.json"
        self.write(6)
        self.engine = Engine(self.path)

    def tearDown(self):
        self.m.unregister()
        self.engine.stop()
        self.tmp.cleanup()

    def write(self, count):
        graph = {"version": 1, "display": "out", "nodes": {
            "m": {"type": "modifier_input"},
            "p": {"type": "scatter", "inputs": {"geo": "m"}, "params": {"count": count, "seed": 2}},
            "i": {"type": "instance", "inputs": {"points": "p"}, "params": {"objects": "Rock"}},
            "so": {"type": "scene_objects", "inputs": {"geo": "i"}, "params": {"prefix": "Rock", "collection": "Rocks"}},
            "out": {"type": "output", "inputs": {"geo": "m"}}},
            "exposed": [{"name": "Count", "node": "p", "param": "count"}]}
        self.path.write_text(json.dumps(graph), encoding="utf-8")

    def test_a_modifier_makes_objects_parented_to_its_owner(self):
        me = bpy.data.meshes.new("Ground")
        me.from_pydata([(0, 0, 0), (4, 0, 0), (4, 4, 0), (0, 4, 0)], [], [(0, 1, 2, 3)])
        ground = bpy.data.objects.new("Ground", me)
        bpy.context.scene.collection.objects.link(ground)
        rock = bpy.data.meshes.new("Rock")
        rock.from_pydata([(0, 0, 0), (1, 0, 0), (0, 1, 0)], [], [(0, 1, 2)])
        rock_obj = bpy.data.objects.new("Rock", rock)
        bpy.context.scene.collection.objects.link(rock_obj)
        mod = self.m.add_modifier(ground, str(self.path), port=self.engine.port)
        self.m.pump(port=self.engine.port, force=True)
        self.assertEqual(self.m.last_error(), "")
        self.assertEqual(len([o for o in bpy.data.objects if o.get("minihou_scene")]), 0)      # queued, not made inside pump
        self.assertGreater(self.so.flush(), 0)                                                  # the timer's job
        objs = sorted((o for o in bpy.data.objects if o.get("minihou_scene")), key=lambda o: o["minihou_id"])
        self.assertEqual(len(objs), 6)
        for o in objs:
            self.assertIs(o.parent, ground)                                                     # moves with its owner
            self.assertIs(o.data, rock)
            self.assertTrue(o["minihou_scene"].endswith(":so"))
        self.assertIn("Rocks", bpy.data.collections)
        # a native modifier on one of them survives the count going down and up
        mine = objs[1]
        mine.modifiers.new("Mine", "SUBSURF")
        values = self.proxy.read_values(mod, mod.node_group, self.m.parameters(mod))
        values["Count"] = 3
        self.proxy.write_values(mod, mod.node_group, self.m.parameters(mod), values)
        self.m.pump(port=self.engine.port)
        self.so.flush()
        shown = [o for o in objs if not o.get("minihou_hidden")]
        self.assertEqual(len(shown), 3)
        values["Count"] = 6
        self.proxy.write_values(mod, mod.node_group, self.m.parameters(mod), values)
        self.m.pump(port=self.engine.port)
        self.so.flush()
        self.assertEqual(len([o for o in objs if not o.get("minihou_hidden")]), 6)
        self.assertIn("Mine", [x.name for x in mine.modifiers])
        # deleting the modifier's owner parks the objects instead of leaving them animating nothing
        bpy.data.objects.remove(ground, do_unlink=True)
        self.m.pump(port=self.engine.port, force=True)
        self.assertTrue(all(o.get("minihou_hidden") for o in objs))


if __name__ == "__main__":
    unittest.main()
