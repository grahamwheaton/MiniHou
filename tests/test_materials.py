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


class PerElementTests(unittest.TestCase):
    """index_attrib picks a name per element; the points domain gives points (Scene Objects instances) their own."""

    def pieces(self, **params):
        nodes = {"b": {"type": "box", "params": {"size": [2, 2, 2]}},
                 "pts": {"type": "voronoi_fracture_points", "inputs": {"geo": "b"}, "params": {"count": 5, "seed": 2}},
                 "f": {"type": "voronoi_fracture", "inputs": {"geo": "b", "points": "pts"}, "params": {}},
                 "a": {"type": "assemble", "inputs": {"geo": "f"}, "params": {}},
                 "m": {"type": "set_material", "inputs": {"geo": "a"}, "params": dict({"index_attrib": "piece", "names": "Wood, Metal ,Glass"}, **params)}}
        return cook(nodes, "m")

    def test_each_piece_takes_the_name_its_number_picks_wrapping_around(self):
        geo = self.pieces()
        piece = geo.prim["piece"].astype(int)
        lst = M.names(geo)
        want = ["Wood", "Metal", "Glass"]
        self.assertEqual([lst[i] for i in M.indices(geo)], [want[p % 3] for p in piece])
        self.assertGreater(len(set(piece)), 3)                                     # it wrapped

    def test_a_group_limits_which_pieces_are_touched(self):
        geo = self.pieces(group="@piece == 0")
        faces = names_per_face(geo)
        piece = geo.prim["piece"].astype(int)
        self.assertEqual({f for f, p in zip(faces, piece) if p == 0}, {"Wood"})
        self.assertEqual({f for f, p in zip(faces, piece) if p != 0}, {""})

    def test_a_missing_attribute_or_names_or_a_float_attribute_is_a_clear_error(self):
        from minihou.evaluator import NodeError
        for bad, text in (({"index_attrib": "nope"}, "no primitive attribute"), ({"names": ""}, "list the material names")):
            with self.subTest(bad=bad), self.assertRaises(NodeError) as cm:
                self.pieces(**bad)
            self.assertIn(text, str(cm.exception))
        nodes = {"b": {"type": "box"}, "w": {"type": "wrangle", "inputs": {"geo": "b"}, "params": {"run_over": "primitives", "snippet": "f@frac = 0.5;"}},
                 "m": {"type": "set_material", "inputs": {"geo": "w"}, "params": {"index_attrib": "frac", "names": "A,B"}}}
        with self.assertRaises(NodeError) as cm:
            cook(nodes, "m")
        self.assertIn("must be an integer", str(cm.exception))

    def test_points_get_material_id_and_leave_the_faces_alone(self):
        nodes = {"g": {"type": "grid", "params": {"columns": 3, "rows": 3}},
                 "p": {"type": "scatter", "inputs": {"geo": "g"}, "params": {"count": 6, "seed": 1}},
                 "r": {"type": "attribute_randomize", "inputs": {"geo": "p"}, "params": {}},
                 "m": {"type": "set_material", "inputs": {"geo": "p"}, "params": {"domain": "points", "material": "Leaf"}}}
        geo = cook(nodes, "m")
        self.assertEqual(M.names(geo), ["", "Leaf"])
        self.assertEqual(set(geo.point["material_id"].tolist()), {1})
        self.assertNotIn("material_index", geo.prim)

    def test_merge_remaps_point_indices_too(self):
        def pts(name, x):
            return {f"p{name}": {"type": "scatter", "inputs": {"geo": f"g{name}"}, "params": {"count": 3, "seed": 1}},
                    f"g{name}": {"type": "grid", "params": {"columns": 2, "rows": 2, "center": [x, 0, 0]}},
                    f"m{name}": {"type": "set_material", "inputs": {"geo": f"p{name}"}, "params": {"domain": "points", "material": name}}}
        nodes = {**pts("A", 0), **pts("B", 9), "out": {"type": "merge", "inputs": {"geos": ["mA", "mB"]}}}
        geo = cook(nodes, "out")
        lst = M.names(geo)
        picked = [lst[i] for i in M.indices(geo, "point")]
        self.assertEqual(sorted(set(picked)), ["A", "B"])
        self.assertEqual(picked[:geo.npoints // 2].count("A") + picked[geo.npoints // 2:].count("B"), geo.npoints)

    def test_instance_manifest_carries_the_materials_per_object(self):
        nodes = {"g": {"type": "grid", "params": {"columns": 3, "rows": 3}},
                 "p": {"type": "scatter", "inputs": {"geo": "g"}, "params": {"count": 6, "seed": 1}},
                 "m": {"type": "set_material", "inputs": {"geo": "p"},
                       "params": {"domain": "points", "group": "@ptnum < 2", "material": "Red"}}}
        geo = cook(nodes, "m")
        cols = plan(geo, "instances", "Tree", "id", 100)[0]
        self.assertEqual(cols["material"], ["Red", "Red", "", "", "", ""])
        self.assertNotIn("material", plan(cook({"g": {"type": "grid"}}, "g"), "instances", "Tree", "id", 100)[0])
