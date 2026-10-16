"""bpy-free checks for promotion of edited nested modifier placements."""
import importlib.util
import sys
import types
import unittest
from pathlib import Path
from unittest.mock import patch


class Tree(dict):
    def __init__(self, users=1):
        super().__init__()
        self.users = users
        self.use_fake_user = False

    def copy(self):
        result = Tree()
        result.update(self)
        self.users -= 1
        return result


class Socket:
    def __init__(self, links=()):
        self.links = list(links)
        self.type = "GEOMETRY"


class Sockets(dict):
    def __iter__(self):
        return iter(self.values())


class Node(dict):
    bl_idname = "GeometryNodeGroup"

    def __init__(self, tree, source=None):
        super().__init__()
        self.node_tree = tree
        self.inputs = Sockets(Geometry=Socket([] if source is None else [types.SimpleNamespace(from_node=source)]))


def load_module():
    root = Path(__file__).resolve().parents[1] / "blender_addon/minihou_link"
    pkg = "_gn_identity_addon"
    package = types.ModuleType(pkg)
    package.__path__ = [str(root)]
    sys.modules[pkg] = package
    bpy = types.ModuleType("bpy")
    bpy.app = types.SimpleNamespace(handlers=types.SimpleNamespace(persistent=lambda f: f))
    sys.modules["bpy"] = bpy
    handlers = types.ModuleType("bpy.app.handlers")
    handlers.persistent = lambda f: f
    sys.modules["bpy.app.handlers"] = handlers
    for name in ("modifiers", "proxy", "mesh_reader", "mesh_writer", "scene_objects"):
        module = types.ModuleType(pkg + "." + name)
        sys.modules[module.__name__] = module
        setattr(package, name, module)
    package.modifiers.KEY = "modifier_id"
    package.modifiers.PATH = "path"
    package.modifiers.GRAPH = "graph"
    package.modifiers.PARAMS = "params"
    package.modifiers.STATUS = "status"
    package.proxy.PLUMBING_KEY = "plumbing"
    protocol = types.ModuleType(pkg + ".protocol")
    protocol.GeoUpdate = protocol.Mirror = protocol.instance_payload = object()
    sys.modules[protocol.__name__] = protocol
    spec = importlib.util.spec_from_file_location(pkg + ".geonodes", root / "geonodes.py")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


class PlacementIdentityTests(unittest.TestCase):
    def test_duplicate_gets_private_group_and_id_with_copied_graph(self):
        gn = load_module()
        inner = Tree(users=2)
        inner["plumbing"] = "{}"
        original, duplicate = Node(inner), Node(inner)
        group_input = types.SimpleNamespace(bl_idname="NodeGroupInput")
        for node in (original, duplicate):
            param = Socket([types.SimpleNamespace(from_node=group_input)])
            param.type = "VALUE"
            param.default_value = 0
            node.inputs["Scale"] = param
        root = Tree()
        root.update(modifier_id="old", path="example.json", graph="graph snapshot", params="params snapshot")
        root.nodes = [original, duplicate]
        root.links = types.SimpleNamespace(remove=lambda link: None)
        with patch.object(gn, "_value", return_value=3):
            gn._promote_wired_modifier(types.SimpleNamespace(node_group=root))
        self.assertNotIn("modifier_id", root)
        self.assertIsNot(original.node_tree, duplicate.node_tree)
        self.assertNotEqual(original[gn.KEY], duplicate[gn.KEY])
        for node in (original, duplicate):
            self.assertEqual(node[gn.KEY], node.node_tree[gn.KEY])
            self.assertEqual(node.node_tree["graph"], "graph snapshot")
            self.assertEqual(node.node_tree["params"], "params snapshot")
            self.assertEqual(node.inputs["Scale"].default_value, 3)

    def test_detached_single_placement_is_promoted_without_error(self):
        gn = load_module()
        inner = Tree()
        inner["plumbing"] = "{}"
        node = Node(inner)
        root = Tree()
        root.update(modifier_id="old", path="example.json", graph="{}", params="[]")
        root.nodes = [node]
        gn._promote_wired_modifier(types.SimpleNamespace(node_group=root))
        self.assertIn(gn.KEY, node)
        self.assertNotIn("modifier_id", root)

    def test_untouched_single_modifier_keeps_existing_identity(self):
        gn = load_module()
        inner = Tree()
        inner["plumbing"] = "{}"
        node = Node(inner, types.SimpleNamespace(bl_idname="NodeGroupInput"))
        root = Tree()
        root.update(modifier_id="old", path="example.json", graph="{}", params="[]")
        root.nodes = [node]
        gn._promote_wired_modifier(types.SimpleNamespace(node_group=root))
        self.assertEqual(root["modifier_id"], "old")
        self.assertNotIn(gn.KEY, node)


if __name__ == "__main__":
    unittest.main()
