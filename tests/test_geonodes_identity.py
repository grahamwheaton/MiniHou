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
    def __init__(self, links=(), identifier="Geometry", value=None):
        self.links = list(links)
        self.type = "GEOMETRY"
        self.identifier = identifier
        self.name = identifier
        self.default_value = value

    @property
    def is_linked(self):
        return bool(self.links)

    def as_pointer(self):
        return id(self)


class Sockets(dict):
    def __iter__(self):
        return iter(self.values())


class Node(dict):
    bl_idname = "GeometryNodeGroup"

    def __init__(self, tree, source=None):
        super().__init__()
        self.node_tree = tree
        self.inputs = Sockets(Geometry=Socket([] if source is None else [types.SimpleNamespace(from_node=source)]))
        self.name = "MH_plumbing"


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
    package.proxy.PLUMBING_NODE = "MH_plumbing"
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
        duplicate.name = "MH_plumbing.001"
        group_input = types.SimpleNamespace(bl_idname="NodeGroupInput")
        for node in (original, duplicate):
            param = Socket([types.SimpleNamespace(from_node=group_input)], "Scale", 0)
            param.type = "VALUE"
            node.inputs["Scale"] = param
        root = Tree()
        root.update(modifier_id="old", path="example.json", graph="graph snapshot", params="params snapshot")
        root.nodes = [duplicate, original]
        root.links = types.SimpleNamespace(remove=lambda link: duplicate.inputs["Scale"].links.remove(link))
        with patch.object(gn, "_value", return_value=3):
            gn._promote_wired_modifier(types.SimpleNamespace(node_group=root))
        self.assertNotIn("modifier_id", root)
        self.assertIsNot(original.node_tree, duplicate.node_tree)
        self.assertNotEqual(original[gn.KEY], duplicate[gn.KEY])
        self.assertTrue(original.inputs["Scale"].is_linked)
        self.assertFalse(duplicate.inputs["Scale"].is_linked)
        for node in (original, duplicate):
            self.assertEqual(node[gn.KEY], node.node_tree[gn.KEY])
            self.assertEqual(node.node_tree["graph"], "graph snapshot")
            self.assertEqual(node.node_tree["params"], "params snapshot")
        self.assertEqual(duplicate.inputs["Scale"].default_value, 3)

    def test_panel_drives_first_and_socket_drives_copy_with_optional_links(self):
        gn = load_module()
        inner = Tree(users=2)
        inner["plumbing"] = "{}"
        group_input = types.SimpleNamespace(bl_idname="NodeGroupInput")
        original, duplicate = Node(inner, group_input), Node(inner)
        duplicate.name = "MiniHou Plumbing.001"
        panel = {"Scale": 0.4, "Translate": [2, 0, 0], "subdivide1_depth": 3}
        for name, value in panel.items():
            source = Socket(identifier=name, value=value)
            link = types.SimpleNamespace(from_node=group_input, from_socket=source)
            for node in (original, duplicate):
                socket = Socket([link], name, 1 if name == "Scale" and node == duplicate else 0)
                socket.type = "VALUE"
                node.inputs[name] = socket
        root = Tree()
        root.update(modifier_id="old", path="example.json", graph="{}", params="[]")
        root.nodes = [original, duplicate]
        root.links = types.SimpleNamespace(remove=lambda link: [s.links.remove(link) for s in duplicate.inputs if link in s.links])
        mod = types.SimpleNamespace(node_group=root, get=lambda _key, default=None: default)
        with patch.object(gn.proxy, "input_owner", return_value=None, create=True), \
             patch.object(gn.proxy, "has_input", return_value=True, create=True), \
             patch.object(gn.proxy, "get_input", side_effect=lambda _mod, ident: panel[ident], create=True):
            gn._promote_wired_modifier(mod)
            for name, value in panel.items():
                self.assertEqual(gn._value(root, mod, (), original.inputs[name]), value)
                self.assertEqual(gn._value(root, mod, (), duplicate.inputs[name]), value)
            self.assertEqual(duplicate.inputs["Scale"].default_value, 0.4)
            panel["Scale"] = 0.8
            self.assertEqual(gn._value(root, mod, (), original.inputs["Scale"]), 0.8)
            self.assertEqual(gn._value(root, mod, (), duplicate.inputs["Scale"]), 0.4)
            duplicate.inputs["subdivide1_depth"].default_value = 6
            self.assertEqual(gn._value(root, mod, (), duplicate.inputs["subdivide1_depth"]), 6)
            self.assertEqual(gn._value(root, mod, (), original.inputs["subdivide1_depth"]), 3)
            source = Socket(identifier="Scale")
            source.default_value = 2
            duplicate.inputs["Scale"].links.append(types.SimpleNamespace(
                from_node=types.SimpleNamespace(bl_idname="ShaderNodeValue"), from_socket=source))
            self.assertEqual(gn._value(root, mod, (), duplicate.inputs["Scale"]), 2)
            duplicate.inputs["Scale"].links.clear()
            duplicate.inputs["Scale"].links.append(types.SimpleNamespace(from_node=group_input, from_socket=source))
            self.assertEqual(gn._value(root, mod, (), duplicate.inputs["Scale"]), 0.8)

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
