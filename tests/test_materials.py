"""Material names in the engine: the list that gives ``material_index`` its meaning (minihou/materials.py), the set_material node,
merges and booleans, the wire form from Blender and the pieces manifest."""
import unittest

import numpy as np

import minihou  # noqa: F401
from minihou import Evaluator, Geometry, Graph
from minihou import materials as M
from minihou.link.diff import DiffState
from minihou.link.server import _decode_object
from minihou.nodes.scene_objects import plan
from minihou.link.protocol import TOPO_PRIM_OFFSETS, TOPO_VERTEX_POINT, attr_key


def cook(nodes, target):
    return Evaluator().cook(Graph.from_dict({"nodes": nodes, "display": target}), target)


def boxes(*mats, boolean=None):
    """Graph of unit boxes, side by side, each given a material (``None``: none), merged or combined."""
    nodes = {}
    ids = []
    for i, m in enumerate(mats):
        nodes[f"b{i}"] = {"type": "box", "params": {"center": [i * 3.0, 0, 0]}}
        src = f"b{i}"
        if m:
            nodes[f"m{i}"] = {"type": "set_material", "inputs": {"geo": src}, "params": {"material": m}}
            src = f"m{i}"
        ids.append(src)
    if boolean:
        nodes["out"] = {"type": "boolean", "inputs": {"a": ids[0], "b": ids[1]}, "params": {"operation": boolean}}
    else:
        nodes["out"] = {"type": "merge", "inputs": {"geos": ids}}
    return nodes


def names_per_face(geo):
    lst = M.names(geo)
    return [lst[i] if i < len(lst) else "" for i in M.indices(geo)]


class AssignTests(unittest.TestCase):
    def test_a_name_gives_the_faces_an_index_into_the_geometry_list(self):
        geo = cook({"b": {"type": "box"}, "m": {"type": "set_material", "inputs": {"geo": "b"}, "params": {"material": "Wood"}}}, "m")
        self.assertEqual(M.names(geo), ["", "Wood"])
        self.assertEqual(set(M.indices(geo)), {1})
        self.assertEqual(M.used(geo), ["Wood"])
        self.assertEqual(geo.prim["material_index"].dtype, np.int32)

    def test_a_group_leaves_the_other_faces_alone_and_a_second_name_is_appended(self):
        nodes = {"b": {"type": "box"},
                 "a": {"type": "set_material", "inputs": {"geo": "b"}, "params": {"group": "@primnum < 2", "material": "Roof"}},
                 "c": {"type": "set_material", "inputs": {"geo": "a"}, "params": {"group": "@primnum >= 5", "material": "Floor"}}}
        geo = cook(nodes, "c")
        self.assertEqual(M.names(geo), ["", "Roof", "Floor"])
        faces = names_per_face(geo)
        self.assertEqual(sorted(faces), ["", "", "", "Floor", "Roof", "Roof"])
        self.assertEqual(len(faces), geo.nprims)

    def test_an_empty_group_changes_nothing(self):
        geo = cook({"b": {"type": "box"}, "m": {"type": "set_material", "inputs": {"geo": "b"},
                                                  "params": {"group": "@P.y > 100", "material": "X"}}}, "m")
        self.assertEqual(M.names(geo), [])
        self.assertNotIn("material_index", geo.prim)

    def test_existing_indices_from_blender_keep_their_meaning(self):
        geo = Geometry.from_arrays(np.array([[0, 0, 0], [1, 0, 0], [1, 1, 0], [0, 1, 0]], np.float32),
                                   np.array([0, 1, 2, 0, 2, 3], np.int32), np.array([0, 3, 6], np.int32))
        geo.prim["material_index"] = np.array([2, 0], np.int32)
        geo.detail["materials"] = ["A", "B", "C"]
        out = M.assign(geo, np.array([False, True]), "D")
        self.assertEqual(names_per_face(out), ["C", "D"])
        self.assertEqual(M.names(out), ["A", "B", "C", "D"])
        self.assertEqual(names_per_face(geo), ["C", "A"])               # the input is untouched

    def test_names_are_trimmed_and_capped(self):
        self.assertEqual(M.clean("  Brick  "), "Brick")
        self.assertEqual(len(M.clean("x" * 200)), 63)
        self.assertEqual(M.clean(None), "")

    def test_an_empty_name_puts_faces_back_to_not_ours(self):
        geo = cook({"b": {"type": "box"}, "m": {"type": "set_material", "inputs": {"geo": "b"}, "params": {"material": "Wood"}},
                    "r": {"type": "set_material", "inputs": {"geo": "m"}, "params": {"material": ""}}}, "r")
        self.assertEqual(set(names_per_face(geo)), {""})
        self.assertEqual(M.used(geo), [])


class CombineTests(unittest.TestCase):
    def test_merge_gives_every_input_the_same_list_and_remaps_indices(self):
        geo = cook(boxes("Wood", "Metal", None), "out")
        self.assertEqual(M.names(geo), ["", "Wood", "Metal"])
        faces = names_per_face(geo)
        self.assertEqual(faces[:6], ["Wood"] * 6)
        self.assertEqual(faces[6:12], ["Metal"] * 6)
        self.assertEqual(faces[12:], [""] * 6)                              # no material of ours

    def test_merge_of_the_same_name_shares_one_slot(self):
        geo = cook(boxes("Wood", "Wood"), "out")
        self.assertEqual(M.names(geo), ["", "Wood"])
        self.assertEqual(set(M.indices(geo)), {1})

    def test_merge_without_any_names_leaves_blender_style_integers_alone(self):
        a = Geometry.from_arrays(np.zeros((3, 3), np.float32), np.array([0, 1, 2], np.int32), np.array([0, 3], np.int32))
        b = a.copy()
        a.prim["material_index"] = np.array([2], np.int32)
        b.prim["material_index"] = np.array([1], np.int32)
        out = M.unify([a, b])
        self.assertIs(out[0], a)
        self.assertIs(out[1], b)

    def test_boolean_of_two_materials_keeps_each_side_its_own(self):
        nodes = boxes("Wood", "Metal", boolean="union")
        nodes["b1"]["params"]["center"] = [0.5, 0, 0]
        geo = cook(nodes, "out")
        self.assertEqual(set(M.used(geo)), {"Wood", "Metal"})
        lst = M.names(geo)
        self.assertIn("Wood", lst)
        self.assertIn("Metal", lst)
        self.assertEqual(len(set(names_per_face(geo))), 2)


class WireTests(unittest.TestCase):
    def test_the_names_from_blender_become_a_detail_list(self):
        arrays = {attr_key("point", "P"): np.array([[0, 0, 0], [1, 0, 0], [0, 1, 0]], np.float32),
                  TOPO_VERTEX_POINT: np.array([0, 1, 2], np.int32),
                  TOPO_PRIM_OFFSETS: np.array([0, 3], np.int32), attr_key("prim", "material_index"): np.array([1], np.int32)}
        geo = _decode_object({"materials": ["Skin", "Hair"]}, arrays)
        self.assertEqual(M.names(geo), ["Skin", "Hair"])
        self.assertEqual(names_per_face(geo), ["Hair"])
        self.assertEqual(M.names(_decode_object({}, arrays)), [])
        self.assertEqual(M.names(_decode_object({"materials": "nope"}, arrays)), [])

    def test_the_list_travels_to_blender_in_the_detail_header(self):
        geo = cook(boxes("Wood", None), "out")
        header, _ = DiffState().build(geo, 1)
        self.assertEqual(header["detail"]["materials"], ["", "Wood"])


class PiecesTests(unittest.TestCase):
    def test_the_pieces_manifest_carries_the_name_list(self):
        geo = cook(boxes("Wood", "Metal"), "out")
        cols, arrays = plan(geo, "pieces", "", "id", 100, "piece", "material_index")
        self.assertEqual(cols["materials"], ["", "Wood", "Metal"])
        self.assertIn("prim:material_index", arrays)
        self.assertEqual(sorted(set(arrays["prim:material_index"].tolist())), [1, 2])

    def test_no_names_no_column(self):
        geo = cook({"b": {"type": "box"}}, "b")
        cols, _ = plan(geo, "pieces", "", "id", 100, "piece", "material_index")
        self.assertNotIn("materials", cols)


if __name__ == "__main__":
    unittest.main()
