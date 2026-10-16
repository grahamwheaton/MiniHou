"""Repeated syncs keep one nested layout, stable datablocks and user counts, and per-placement controls (add-on 0.2.1).

Runs inside headless Blender against the Rust ``link_selftest`` server (see tests/test_blender_rust_placements.py):

    blender --background --factory-startup --python-exit-code 1 --python tools/blender_selftest.py
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

SYNCS = 12


def snapshot():
    return sorted((g.name, g.users) for g in bpy.data.node_groups)


@unittest.skipIf(bpy is None, "requires real Blender")
class SessionProxyLayoutTests(unittest.TestCase):
    """The 'Open Blender' path: runtime interface/geo updates build the proxy through proxy.ensure_objects."""

    def setUp(self):
        from minihou_link import proxy, runtime
        bpy.ops.wm.read_factory_settings(use_empty=True)
        self.proxy, self.runtime = proxy, runtime

    def interface(self, value=1.0, structure="s1"):
        self.runtime.get_runtime()._apply_interface(
            {"type": "interface", "structure": structure,
             "params": [{"name": "Scale", "kind": "float", "value": value},
                        {"name": "Translate", "kind": "vec3", "value": [0, 0, 0]}]})

    def session_group(self):
        return bpy.data.objects[self.proxy.PROXY_OBJECT].modifiers[self.proxy.MODIFIER].node_group

    def test_session_proxy_is_nested_like_add_modifier(self):
        self.interface()
        ng = self.session_group()
        self.assertTrue(self.proxy.is_nested(ng))
        self.assertEqual(sorted(n.bl_idname for n in ng.nodes),
                         ["GeometryNodeGroup", "NodeGroupInput", "NodeGroupOutput"])
        inner = self.proxy.plumbing(ng)
        info = self.proxy.cache_info(ng)
        self.assertEqual(info.id_data, inner)
        self.assertEqual(info.inputs["Object"].default_value.name, self.proxy.OUTPUT_OBJECT)
        self.assertFalse(self.proxy.passthrough(ng))          # engine output, not the empty proxy mesh

    def test_flat_session_group_from_older_addon_migrates_keeping_values(self):
        out, _obj, mod = self.legacy_flat()
        ident = self.proxy.mapping(mod.node_group)["Scale"]
        self.proxy.set_input(mod, ident, 3.5)
        self.proxy.ensure_objects()
        ng = self.session_group()
        self.assertTrue(self.proxy.is_nested(ng))
        self.assertEqual(self.proxy.get_input(mod, ident), 3.5)
        self.assertEqual(self.proxy.cache_info(ng).inputs["Object"].default_value, out)

    def legacy_flat(self):
        # What add-on <= 0.2.0 built: everything flat in the "MiniHou" group.
        out = bpy.data.objects.new(self.proxy.OUTPUT_OBJECT, bpy.data.meshes.new(self.proxy.OUTPUT_OBJECT))
        obj = bpy.data.objects.new(self.proxy.PROXY_OBJECT, bpy.data.meshes.new(self.proxy.PROXY_OBJECT))
        bpy.context.scene.collection.objects.link(obj)
        ng = bpy.data.node_groups.new(self.proxy.GROUP, "GeometryNodeTree")
        self.proxy._build_nodes(ng, out)
        self.proxy.sync_interface(ng, out, [{"name": "Scale", "kind": "float", "value": 1.0}], "legacy")
        mod = obj.modifiers.new(self.proxy.MODIFIER, "NODES")
        mod.node_group = ng
        self.assertFalse(self.proxy.is_nested(ng))
        return out, obj, mod

    def test_repeated_syncs_are_idempotent(self):
        self.interface()
        self.proxy.ensure_objects()
        before = snapshot()
        for k in range(SYNCS):
            self.interface(value=1.0 + k)
            self.proxy.ensure_objects()
            self.proxy.sync_instances(self.session_group(), [])
        self.assertEqual(snapshot(), before)
        self.assertTrue(all(users == 1 for _name, users in before), before)

    def test_session_group_kept_when_renamed_or_copied(self):
        self.interface()
        ng = self.session_group()
        ng.name = "Renamed"
        copy = ng.copy()                                     # what a privatising copy produces
        mod = bpy.data.objects[self.proxy.PROXY_OBJECT].modifiers[self.proxy.MODIFIER]
        mod.node_group = copy
        for _ in range(SYNCS):
            self.interface()
            self.proxy.ensure_objects()
        self.assertEqual(mod.node_group, copy)               # no ping-pong back to the old group
        self.proxy.cleanup_plumbing()
        self.assertNotIn("Renamed", bpy.data.node_groups)    # the orphan (and its plumbing user) is gone
        self.assertEqual(self.proxy.plumbing(copy).users, 1)


@unittest.skipIf(bpy is None, "requires real Blender")
class PlacementSyncTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        from tests.test_blender_rust_placements import RustEngine, TRANSFORM
        cls.graph = TRANSFORM
        cls.engine = RustEngine(TRANSFORM)

    @classmethod
    def tearDownClass(cls):
        cls.engine.stop()

    def setUp(self):
        from minihou_link import geonodes, modifiers, proxy
        from minihou_link.instance_client import InstanceClient
        self.g, self.m, self.proxy = geonodes, modifiers, proxy
        self.g.unregister()
        self.m.disconnect()
        bpy.ops.wm.read_factory_settings(use_empty=True)
        self.cooks = []
        original, recorder = InstanceClient.cook, self.cooks

        def cook(client, instance_id, graph_path, params, arrays, **kw):
            recorder.append({"id": instance_id, "params": dict(params)})
            return original(client, instance_id, graph_path, params, arrays, **kw)
        InstanceClient.cook = cook
        self.addCleanup(setattr, InstanceClient, "cook", original)
        self.addCleanup(self.m.disconnect)

    def pump(self):
        del self.cooks[:]
        count = self.m.pump(port=self.engine.port)
        for _ in range(3):   # the debug link_selftest server can miss the add-on's 200 ms interactive budget
            if "did not reply within" not in self.m.last_error() + self.g.last_error():
                break
            del self.cooks[:]
            count = self.m.pump(port=self.engine.port, force=True)
        self.assertEqual(self.m.last_error(), "")
        self.assertEqual(self.g.last_error(), "")
        return count

    def two_placements(self, path=None):
        """The user's layout: 'Add MiniHou Modifier', then Shift+D of its MiniHou Plumbing node, wired in series."""
        bpy.ops.mesh.primitive_plane_add()
        obj = bpy.context.object
        mod = self.m.add_modifier(obj, str(self.graph) if path is None else path, port=self.engine.port)
        self.pump()
        tree = mod.node_group
        first = self.proxy.plumbing_node(tree)
        area = bpy.context.screen.areas[0]
        old = area.type
        area.type = "NODE_EDITOR"
        area.ui_type = "GeometryNodeTree"
        space = area.spaces.active
        space.pin = True
        space.node_tree = tree
        self.addCleanup(setattr, area, "type", old)
        region = next(r for r in area.regions if r.type == "WINDOW")
        for node in tree.nodes:
            node.select = node == first
        tree.nodes.active = first
        with bpy.context.temp_override(area=area, region=region):
            self.assertEqual(bpy.ops.node.duplicate(), {"FINISHED"})
        second = tree.nodes.active
        gout = next(n for n in tree.nodes if n.bl_idname == "NodeGroupOutput")
        tree.links.new(first.outputs[0], second.inputs["Geometry"])
        tree.links.new(second.outputs[0], gout.inputs[0])
        return obj, mod, first, second

    def test_repeated_syncs_keep_datablocks_and_user_counts_stable(self):
        _obj, _mod, _first, second = self.two_placements(path=" ")   # new-graph modifier: "MiniHou Plumbing"
        self.pump()
        before = snapshot()
        self.assertTrue(all(users == 1 for _name, users in before), before)
        for k in range(SYNCS):
            second.inputs["Scale"].default_value = 2.0 + k
            self.pump()
            self.assertEqual(snapshot(), before, f"sync {k}")

    def test_orphaned_copies_from_older_syncs_are_released(self):
        _obj, mod, first, _second = self.two_placements()
        self.pump()
        before = snapshot()
        for _ in range(5):                                   # what each sync of 0.2.0 could leave behind
            mod.node_group.copy()
        self.assertGreater(first.node_tree.users, 1)
        self.pump()
        self.assertEqual(snapshot(), before)
        self.pump()
        self.assertEqual(snapshot(), before)

    def test_each_placement_has_its_own_panel_controls(self):
        _obj, mod, first, second = self.two_placements()
        self.pump()
        sections = self.g.placement_controls(mod)
        self.assertEqual([s["title"] for s in sections], ["MiniHou 1", "MiniHou 2"])
        self.assertEqual([s["node"] for s in sections], [first, second])
        rows = {s["title"]: {label: (data, prop) for label, data, prop in s["rows"]} for s in sections}
        self.assertIn("Scale (modifier input)", rows["MiniHou 1"])      # placement 1 stays on the modifier inputs
        data, prop = rows["MiniHou 2"]["Scale"]
        self.assertEqual(data, second.inputs["Scale"])
        self.assertEqual(prop, "default_value")
        setattr(data, prop, 4.0)                                         # edit placement 2 from the panel
        self.assertEqual(self.pump(), 1)
        self.assertEqual([c["id"] for c in self.cooks], [second.node_tree[self.g.KEY]])
        self.assertEqual(self.cooks[0]["params"]["Scale"], 4.0)
        data, prop = rows["MiniHou 1"]["Scale (modifier input)"]
        setattr(data, prop, 3.0)                                         # edit placement 1 from the panel
        self.assertEqual(self.pump(), 2)                                 # 1, then 2 downstream of it
        self.assertEqual([c["params"]["Scale"] for c in self.cooks], [3.0, 4.0])
        second.label = "Detail"
        self.assertEqual([s["title"] for s in self.g.placement_controls(mod)], ["MiniHou 1", "Detail"])
        # The panel draws one collapsible section per placement (recorded; headless has no real UILayout).
        import minihou_link
        layout = RecordingLayout()
        for section in self.g.placement_controls(mod):
            minihou_link.draw_placement(layout, mod, section)
        titles = [kw["text"] for name, args, kw in layout.calls if name == "label" and kw.get("icon") == "NODETREE"]
        self.assertEqual(titles, [f"{mod.name} / MiniHou 1", f"{mod.name} / Detail"])
        props = [(args[0], args[1], kw["text"]) for name, args, kw in layout.calls if name == "prop"]
        self.assertIn((second.inputs["Scale"], "default_value", "Scale"), props)
        self.assertIn((second, "label", "Label"), props)


class RecordingLayout:
    def __init__(self):
        self.calls = []

    def panel(self, idname, default_closed=False):
        self.calls.append(("panel", (idname,), {}))
        return self, self

    def __getattr__(self, name):
        def record(*args, **kw):
            self.calls.append((name, args, kw))
            return self
        return record


if __name__ == "__main__":
    unittest.main(argv=[sys.argv[0]], verbosity=2)
