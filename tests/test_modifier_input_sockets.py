"""Mocked bpy contract for dynamic interface identity, values, and version guards."""
import importlib
import json
from pathlib import Path
import sys
import types
import unittest
from unittest.mock import patch


ROOT = Path(__file__).resolve().parents[1]
PACKAGE = "_modifier_input_addon"
package = types.ModuleType(PACKAGE)
package.__path__ = [str(ROOT / "blender_addon/minihou_link")]
mesh_writer = types.ModuleType(PACKAGE + ".mesh_writer")
mesh_writer.assert_main_thread = lambda: None
bpy = types.ModuleType("bpy")
bpy.app = types.SimpleNamespace(version=(5, 2, 0), version_string="5.2")
bpy.data = types.SimpleNamespace(**{name: {} for name in ("objects", "collections", "materials", "images", "textures")})
with patch.dict(sys.modules, {PACKAGE: package, PACKAGE + ".mesh_writer": mesh_writer, "bpy": bpy}):
    proxy = importlib.import_module(PACKAGE + ".proxy")


class Socket:
    next_id = 0

    def __init__(self, name, in_out, socket_type):
        Socket.next_id += 1
        self.identifier = f"Socket_{Socket.next_id}"
        self.name, self.in_out, self.socket_type = name, in_out, socket_type
        self.item_type = "SOCKET"
        self.position = 0
        self.structure_type = "SINGLE"
        self.description = ""
        self.default_value = None
        self.min_value = -1e9
        self.max_value = 1e9


class Interface:
    def __init__(self):
        self.items_tree = []

    def new_socket(self, name, *, in_out, socket_type):
        item = Socket(name, in_out, socket_type)
        self.items_tree.append(item)
        self.positions()
        return item

    def remove(self, item):
        self.items_tree.remove(item)
        self.positions()

    def move(self, item, target):
        self.items_tree.remove(item)
        self.items_tree.insert(target, item)
        self.positions()

    def positions(self):
        for index, item in enumerate(self.items_tree):
            item.position = index


class Group(dict):
    def __init__(self):
        super().__init__()
        self.interface = Interface()
        self.interface.new_socket("Geometry", in_out="INPUT", socket_type="NodeSocketGeometry")


class Modifier(dict):
    def __init__(self, group):
        super().__init__()
        self.node_group = group
        self.id_data = types.SimpleNamespace(update_tag=lambda: None)


def param(ident, name, kind, value):
    return {"node": "input", "id": ident, "name": name, "kind": kind, "value": value}


class ModifierInputSocketTests(unittest.TestCase):
    def setUp(self):
        self.group = Group()
        self.output = object()
        self.build = proxy._build_nodes
        self.chain = proxy._sync_usage_chain
        self.remove = proxy._remove_input
        proxy._build_nodes = lambda *args: None
        proxy._sync_usage_chain = lambda *args: None
        proxy._remove_input = lambda ng, item: ng.interface.remove(item)

    def tearDown(self):
        proxy._build_nodes = self.build
        proxy._sync_usage_chain = self.chain
        proxy._remove_input = self.remove

    def test_types_rename_reorder_remove_keep_identifiers_and_values(self):
        params = [param("f", "Scale", "float", 1.0), param("i", "Count", "int", 2),
                  param("b", "Enabled", "bool", False), param("v", "Offset", "vector", [1, 2, 3]),
                  param("c", "Tint", "color", [1, 0, 0, 1]), param("s", "Tag", "string", "hi"),
                  param("o", "Asset", "object", "")]
        proxy.sync_interface(self.group, self.output, params, "initial")
        identifiers = json.loads(self.group[proxy.MAP_KEY])
        self.assertEqual(len(identifiers), 7)
        mod = Modifier(self.group)
        for item in proxy._active_inputs(self.group):
            mod[item.identifier] = item.default_value
        mod[identifiers["Scale"]] = 7.5
        mod[identifiers["Tag"]] = "placement A"
        self.assertEqual(proxy.read_values(mod, self.group, params)["Scale"], 7.5)
        revised = [dict(params[5], name="Label"), params[4], params[3], params[2], params[1], params[0]]
        proxy.sync_interface(self.group, self.output, revised, "reordered")
        migrated = json.loads(self.group[proxy.MAP_KEY])
        self.assertEqual(migrated["Label"], identifiers["Tag"])
        self.assertEqual(migrated["Scale"], identifiers["Scale"])
        self.assertNotIn(identifiers["Asset"], migrated.values())
        self.assertEqual(proxy.read_values(mod, self.group, revised)["Label"], "placement A")
        self.assertEqual(proxy.read_values(mod, self.group, revised)["Scale"], 7.5)
        self.assertEqual([s.name for s in proxy._active_inputs(self.group)], [p["name"] for p in revised])

    def test_version_guard(self):
        bpy.app.version = (4, 2, 0)
        try:
            with self.assertRaisesRegex(ValueError, "does not support bundle"):
                proxy.sync_interface(self.group, self.output, [param("bundle", "Pack", "bundle", "")], "bundle")
        finally:
            bpy.app.version = (5, 2, 0)
        bpy.types = types.SimpleNamespace(NodeSocketFloat=object)
        try:
            with self.assertRaisesRegex(ValueError, "does not support NodeSocketMatrix"):
                proxy.sync_interface(self.group, self.output, [param("matrix", "Transform", "matrix", "")], "matrix")
        finally:
            del bpy.types

    def test_reference_metadata_carries_names_and_transforms(self):
        matrix = ((1, 0, 0, 3), (0, 1, 0, 4), (0, 0, 1, 5), (0, 0, 0, 1))
        obj = types.SimpleNamespace(name="Cube", matrix_world=matrix)
        collection = types.SimpleNamespace(name="Assets", all_objects=[obj])
        material = types.SimpleNamespace(name="Paint")
        image = types.SimpleNamespace(name="Albedo", size=(64, 32))
        params = [param("o", "Asset", "object", ""), param("c", "Set", "collection", ""),
                  param("m", "Surface", "material", ""), param("im", "Pixels", "image", "")]
        refs = {"Asset": obj, "Set": collection, "Surface": material, "Pixels": image}
        metadata = proxy.reference_metadata(params, lambda p: refs[p["name"]])
        self.assertEqual(metadata["input:input:o"]["matrix"][3], 3)
        self.assertEqual(metadata["input:input:c"]["objects"][0]["name"], "Cube")
        self.assertEqual(metadata["input:input:m"]["name"], "Paint")
        self.assertEqual(metadata["input:input:im"]["size"], [64, 32])

    def test_every_declared_blender_socket_type_is_constructed(self):
        names = {"rotation": [0, 0, 0], "matrix": [1, 0, 0, 0] * 4,
                 "menu": "", "collection": "", "material": "", "image": "",
                 "texture": "", "geometry": "", "bundle": "", "closure": ""}
        params = [param(kind, "Extra Geometry" if kind == "geometry" else kind.title(), kind, value)
                  for kind, value in names.items()]
        params.append(dict(param("dir", "Direction", "vector", [0, 0, 1]), subtype="direction"))
        proxy.sync_interface(self.group, self.output, params, "types")
        sockets = {s.name: s.socket_type for s in proxy._active_inputs(self.group)}
        for p in params:
            self.assertEqual(sockets[p["name"]], proxy._socket_type(p))

    def test_numeric_vector_and_path_subtypes(self):
        variants = [("float", "angle", "NodeSocketFloatAngle"),
                    ("float", "distance", "NodeSocketFloatDistance"),
                    ("float", "factor", "NodeSocketFloatFactor"),
                    ("float", "percentage", "NodeSocketFloatPercentage"),
                    ("float", "time", "NodeSocketFloatTime"),
                    ("int", "factor", "NodeSocketIntFactor"),
                    ("int", "percentage", "NodeSocketIntPercentage"),
                    ("vector", "velocity", "NodeSocketVectorVelocity"),
                    ("vector", "acceleration", "NodeSocketVectorAcceleration"),
                    ("string", "filepath", "NodeSocketStringFilePath")]
        for kind, subtype, expected in variants:
            with self.subTest(kind=kind, subtype=subtype):
                self.assertEqual(proxy._socket_type(dict(param("x", "Test", kind, ""),
                                                         subtype=subtype)), expected)
        with self.assertRaisesRegex(ValueError, "Unsupported float subtype"):
            proxy._socket_type(dict(param("x", "Test", "float", 0), subtype="wrong"))

    def test_menu_switch_populates_and_updates_choices(self):
        class Items(list):
            def __init__(self, node):
                super().__init__()
                self.node = node

            def new(self, name):
                item = types.SimpleNamespace(name=name)
                self.append(item)
                self.node.inputs[name] = types.SimpleNamespace(default_value=None)
                return item

            def clear(self):
                super().clear()
                self.node.inputs = {"Menu": types.SimpleNamespace(default_value=None)}

        class Nodes(dict):
            def new(self, kind):
                self.created += 1
                node = types.SimpleNamespace(name="", data_type=None,
                                             inputs={"Menu": types.SimpleNamespace(default_value=None)},
                                             outputs=[object()])
                node.enum_items = Items(node)
                self[proxy.MENU_NODE_PREFIX + "choice"] = node
                return node

        nodes = Nodes()
        nodes.created = 0
        tree = types.SimpleNamespace(nodes=nodes)
        old_link = proxy._link
        links = []
        proxy._link = lambda tree, src, dst: links.append((src, dst))
        try:
            source = object()
            first = proxy._menu_source(tree, source, "choice", ["One", "Two"])
            self.assertEqual([nodes[proxy.MENU_NODE_PREFIX + "choice"].inputs[n].default_value
                              for n in ("One", "Two")], [0, 1])
            self.assertIs(proxy._menu_source(tree, source, "choice", ["One", "Two"]), first)
            self.assertEqual(nodes.created, 1)
            proxy._menu_source(tree, source, "choice", ["Three"])
            self.assertEqual([i.name for i in nodes[proxy.MENU_NODE_PREFIX + "choice"].enum_items], ["Three"])
            self.assertEqual(len(links), 3)
        finally:
            proxy._link = old_link


if __name__ == "__main__":
    unittest.main()
