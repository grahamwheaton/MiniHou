"""bpy-free regression for the ever-growing 'MiniHou Plumbing' user count (add-on 0.2.1).

A small fake of Blender's ID user counting: a group node or modifier assignment adds a user, ``copy()`` adds a user
to every group the copy's nodes reference, ``remove()`` releases them, and an unassigned copy (an orphan) keeps its
references until it is removed -- exactly what made ``ID.users`` over-report in real files.
"""
import importlib
import json
import sys
import types
import unittest
from pathlib import Path
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
PACKAGE = "_plumbing_users_addon"


class Node:
    def __init__(self, name, tree=None):
        self.name, self.label, self._tree = name, "", None
        self.bl_idname = "GeometryNodeGroup" if tree is not None else "NodeGroupInput"
        self.node_tree = tree

    @property
    def node_tree(self):
        return self._tree

    @node_tree.setter
    def node_tree(self, tree):
        if self._tree is not None:
            self._tree.users -= 1
        if tree is not None:
            tree.users += 1
        self._tree = tree


class Nodes(list):
    def get(self, name):
        return next((n for n in self if n.name == name), None)


class Group(dict):
    __hash__ = object.__hash__

    def __eq__(self, other):
        return self is other

    def __init__(self, data, name):
        super().__init__()
        self.data, self.name, self.users, self.use_fake_user = data, name, 0, False
        self.library, self.asset_data, self.nodes = None, None, Nodes()

    def as_pointer(self):
        return id(self)

    def copy(self):
        twin = self.data.new(self.name + ".copy")
        twin.update(self)
        for node in self.nodes:
            twin.nodes.append(Node(node.name, node.node_tree) if node.node_tree is not None else Node(node.name))
        return twin


class Groups(list):
    def new(self, name, _kind="GeometryNodeTree"):
        group = Group(self, name)
        self.append(group)
        return group

    def get(self, name):
        return next((g for g in self if g.name == name), None)

    def remove(self, group):
        for node in group.nodes:
            node.node_tree = None
        del self[next(k for k, g in enumerate(self) if g is group)]   # by identity: groups are dicts


class Modifier:
    def __init__(self):
        self._group = None

    @property
    def node_group(self):
        return self._group

    @node_group.setter
    def node_group(self, group):
        if self._group is not None:
            self._group.users -= 1
        if group is not None:
            group.users += 1
        self._group = group


def load():
    package = types.ModuleType(PACKAGE)
    package.__path__ = [str(ROOT / "blender_addon/minihou_link")]
    mesh_writer = types.ModuleType(PACKAGE + ".mesh_writer")
    mesh_writer.assert_main_thread = lambda: None
    bpy = types.ModuleType("bpy")
    bpy.app = types.SimpleNamespace(version=(5, 2, 0), version_string="5.2")
    groups = Groups()
    bpy.data = types.SimpleNamespace(node_groups=groups, objects=[])
    with patch.dict(sys.modules, {PACKAGE: package, PACKAGE + ".mesh_writer": mesh_writer, "bpy": bpy}):
        sys.modules.pop(PACKAGE + ".proxy", None)
        proxy = importlib.import_module(PACKAGE + ".proxy")
    return proxy, bpy


class PlumbingUserCountTests(unittest.TestCase):
    def setUp(self):
        self.proxy, self.bpy = load()
        groups = self.bpy.data.node_groups
        # A modifier whose root holds two placements (the user's 'MiniHou Plumbing' + Shift+D copy).
        self.a, self.b = groups.new("MiniHou Plumbing"), groups.new("MiniHou Plumbing.001")
        for tree in (self.a, self.b):
            tree["minihou_gn_id"] = tree.name
        self.root = groups.new("MiniHou ")
        self.root.nodes.extend([Node("Group Input"), Node("MH_plumbing", self.a), Node("MH_plumbing.001", self.b)])
        self.mod = Modifier()
        self.mod.node_group = self.root
        self.bpy.data.objects.append(types.SimpleNamespace(modifiers=[self.mod]))

    def counts(self):
        return sorted((g.name, g.users) for g in self.bpy.data.node_groups)

    def test_orphaned_copies_inflate_users_but_are_not_sharing(self):
        before = self.counts()
        for _ in range(5):
            self.root.copy()                                 # what each 0.2.0 sync could leave behind
        self.assertEqual(self.b.users, 6)                    # the reported symptom: users rise with every sync
        self.assertFalse(self.proxy.shared(self.b))          # ...but nothing live shares it: never copy it again
        self.assertFalse(self.proxy.shared(self.root))
        self.assertEqual(self.proxy.cleanup_plumbing(), 5)
        self.assertEqual(self.counts(), before)
        self.assertEqual(self.b.users, 1)

    def test_really_shared_placement_is_detected(self):
        self.root.nodes.append(Node("MH_plumbing.002", self.b))   # Shift+D: two live nodes, one tree
        self.assertTrue(self.proxy.shared(self.b))
        self.assertFalse(self.proxy.shared(self.a))

    def test_cleanup_keeps_fake_user_and_foreign_groups(self):
        kept = self.root.copy()
        kept.use_fake_user = True
        kept.users += 1
        foreign = self.bpy.data.node_groups.new("Unrelated")
        self.assertEqual(self.proxy.cleanup_plumbing(), 0)
        self.assertIn(kept, self.bpy.data.node_groups)
        self.assertIn(foreign, self.bpy.data.node_groups)

    def test_repeated_session_syncs_do_not_ping_pong_the_session_group(self):
        """ensure_objects must keep the session group the modifier uses, even after it was copied/renamed."""
        proxy, groups = self.proxy, self.bpy.data.node_groups
        session = groups.new(proxy.GROUP)
        session[proxy.SESSION_KEY] = 1
        session.nodes.append(Node(proxy.PLUMBING_NODE, groups.new("MiniHou Session Plumbing")))
        session.nodes[0].node_tree[proxy.PLUMBING_KEY] = "{}"
        mod = Modifier()
        mod.node_group = session.copy()                      # a privatising copy replaced it once
        self.bpy.data.objects.append(types.SimpleNamespace(modifiers=[mod]))
        self.assertEqual(proxy.cleanup_plumbing(), 1)        # the replaced original is an orphan now
        info = types.SimpleNamespace(inputs={"Object": types.SimpleNamespace(default_value=None)})
        out = types.SimpleNamespace(name="MiniHou_Output")
        with patch.object(proxy, "is_nested", return_value=True), patch.object(proxy, "ensure_plumbing"), \
                patch.object(proxy, "cache_info", return_value=info):
            before = self.counts()
            for _ in range(10):
                group = proxy.ensure_node_group(out, mod.node_group)
                if mod.node_group.name != group.name:
                    mod.node_group = group
            self.assertIs(mod.node_group, group)
            self.assertEqual(self.counts(), before)


if __name__ == "__main__":
    unittest.main()
