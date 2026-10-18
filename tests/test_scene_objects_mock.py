"""Small mocked-bpy contract for the Scene Objects placement hierarchy."""
import importlib.util
import sys
import types
import unittest
from pathlib import Path
from unittest.mock import patch

import numpy as np


class FakeObject(dict):
    def __init__(self, name, data):
        super().__init__()
        self.name, self.data = name, data
        self.type = "EMPTY" if data is None else "MESH"
        self.parent = None
        self.location = (0, 0, 0)
        self.rotation_euler = (0, 0, 0)
        self.scale = (1, 1, 1)
        self.hide_viewport = self.hide_render = False

    def hide_set(self, value):
        self.hidden = value


class FakeObjects(list):
    def new(self, name, data):
        obj = FakeObject(name, data)
        self.append(obj)
        return obj

    def get(self, name):
        return next((o for o in self if o.name == name), None)

    def remove(self, obj, do_unlink=True):
        super().remove(obj)


class FakeMesh(dict):
    def __init__(self, name):
        super().__init__()
        self.name, self.users = name, 1


class FakeCollections(dict):
    def new(self, name):
        col = types.SimpleNamespace(name=name, objects=types.SimpleNamespace(link=lambda obj: None))
        self[name] = col
        return col


def load_scene_objects():
    root = Path(__file__).resolve().parents[1] / "blender_addon/minihou_link"
    package = types.ModuleType("_scene_objects_mock")
    package.__path__ = [str(root)]
    objects = FakeObjects()
    collections = FakeCollections()
    scene_collection = types.SimpleNamespace(name="Scene", children_recursive=[], children=types.SimpleNamespace(link=lambda col: None))
    bpy = types.ModuleType("bpy")
    bpy.data = types.SimpleNamespace(objects=objects, collections=collections, meshes=types.SimpleNamespace(new=lambda name: FakeMesh(name)))
    bpy.context = types.SimpleNamespace(scene=types.SimpleNamespace(collection=scene_collection))
    mathutils = types.ModuleType("mathutils")
    mathutils.Matrix = types.SimpleNamespace(Identity=lambda size: None)
    writer = types.ModuleType(package.__name__ + ".mesh_writer")
    writer.NATIVE = set()
    for name in ("apply_material_names", "material_for", "set_corner_verts", "set_positions", "write_attribute", "write_native"):
        setattr(writer, name, lambda *args: None)
    writer.assert_main_thread = lambda: None
    image_exchange = types.ModuleType(package.__name__ + ".image_exchange")
    modules = {"bpy": bpy, "mathutils": mathutils, package.__name__: package,
               writer.__name__: writer, image_exchange.__name__: image_exchange}
    with patch.dict(sys.modules, modules):
        spec = importlib.util.spec_from_file_location(package.__name__ + ".scene_objects", root / "scene_objects.py")
        module = importlib.util.module_from_spec(spec)
        sys.modules[spec.name] = module
        spec.loader.exec_module(module)
    module._write_object_mesh = lambda *args: []
    module.apply_material_names = lambda *args: []
    return module, objects


class SceneObjectsMockTests(unittest.TestCase):
    def test_apply_parents_each_mesh_to_its_placement_empty(self):
        scene, objects = load_scene_objects()
        manifest = {"node": "so", "mode": "objects", "prefix": "Chunk", "id": [7], "loc": [[2, 3, 4]],
                    "npts": [3], "ncorner": [3], "nprim": [1]}
        arrays = {"P": np.zeros((3, 3), np.float32), "vp": np.array([0, 1, 2]), "sizes": np.array([3])}
        scene.apply(manifest, arrays=arrays)
        mesh = next(obj for obj in objects if obj.get("minihou_scene") == "session:so")
        self.assertEqual(mesh.type, "MESH")
        self.assertEqual(mesh["minihou_id"], 7)
        self.assertEqual(mesh.parent.type, "EMPTY")
        self.assertEqual(tuple(mesh.parent.location), (2, 3, 4))
        self.assertEqual(tuple(mesh.location), (0, 0, 0))
        control = mesh.parent
        scene.apply(manifest, arrays=arrays)
        self.assertIs(mesh.parent, control)


if __name__ == "__main__":
    unittest.main()
