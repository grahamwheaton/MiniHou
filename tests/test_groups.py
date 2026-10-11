"""Groups: patterns, group_create methods, promote, blast, and group fields on wrangle / transform / noise."""
import unittest

import numpy as np

from minihou import Evaluator, Geometry, Graph, NodeError, groups


def cook(nodes, display=None):
    nodes = dict(nodes)
    last = display or list(nodes)[-1]
    nodes["out"] = {"type": "output", "inputs": {"geo": last}}
    return Evaluator().cook(Graph.from_dict({"nodes": nodes, "display": "out"}))


GRID = {"g": {"type": "grid", "params": {"columns": 5, "rows": 5, "size_x": 4, "size_y": 4}}}   # 25 pts, 16 prims


def with_groups(*extra):
    nodes = dict(GRID)
    prev = "g"
    for i, params in enumerate(extra):
        nid = f"gc{i}"
        nodes[nid] = {"type": "group_create", "inputs": {"geo": prev}, "params": params}
        prev = nid
    return nodes, prev


class PatternTests(unittest.TestCase):
    def setUp(self):
        nodes, last = with_groups({"name": "top", "pattern": "@P.y > 0"},
                                  {"name": "left", "pattern": "@P.x < 0"},
                                  {"name": "rim", "domain": "primitives", "method": "boundary"})
        self.geo = cook(nodes, last)

    def test_names_ranges_exclusions_and_wildcards(self):
        g = self.geo
        self.assertEqual(groups.names(g, "point"), ["left", "top"])
        top, left = g.point["group_top"] == 1, g.point["group_left"] == 1
        np.testing.assert_array_equal(groups.mask(g, "point", "top left"), top | left)
        np.testing.assert_array_equal(groups.mask(g, "point", "top ^left"), top & ~left)
        np.testing.assert_array_equal(groups.mask(g, "point", "* ^top"), ~top)
        np.testing.assert_array_equal(groups.mask(g, "point", "^top"), ~top)
        np.testing.assert_array_equal(np.flatnonzero(groups.mask(g, "point", "0-3 10 20-24:2")), [0, 1, 2, 3, 10, 20, 22, 24])
        np.testing.assert_array_equal(groups.mask(g, "point", "t*"), top)
        self.assertIsNone(groups.mask(g, "point", "  "))
        np.testing.assert_array_equal(np.flatnonzero(groups.mask(g, "point", "100-200")), [])  # out of range: nothing

    def test_expressions_and_unknown_groups(self):
        g = self.geo
        np.testing.assert_array_equal(groups.mask(g, "point", "@P.y > 0"), g.point["group_top"] == 1)
        np.testing.assert_array_equal(groups.mask(g, "prim", "@P.x > 0").sum(), 8)   # prim centres
        with self.assertRaises(groups.GroupError) as cm:
            groups.mask(g, "point", "nope")
        self.assertIn("left, rim, top", str(cm.exception))

    def test_empty_and_repeated_separators(self):
        for pattern in ("", "  ", ",", " , , ", ",\t,\n"):
            with self.subTest(pattern=pattern):
                self.assertIsNone(groups.mask(self.geo, "point", pattern))
        for pattern in (",top,,left,", "top, , left"):
            np.testing.assert_array_equal(groups.mask(self.geo, "point", pattern),
                                          groups.mask(self.geo, "point", "top left"))
        np.testing.assert_array_equal(groups.mask(self.geo, "point", ", ,^top,,"),
                                      groups.mask(self.geo, "point", "^top"))

    def test_range_strides_must_be_positive_integers(self):
        for stride in ("0", "-1", "1.5", "abc", "", "2:3"):
            for base in ("0-9", "100-200", "3"):
                for prefix in ("", "^"):
                    with self.subTest(stride=stride, base=base, prefix=prefix):
                        with self.assertRaisesRegex(groups.GroupError, "stride.*positive integer"):
                            groups.mask(self.geo, "point", prefix + base + ":" + stride)
        np.testing.assert_array_equal(np.flatnonzero(groups.mask(self.geo, "point", "0-9:2")),
                                      [0, 2, 4, 6, 8])
        np.testing.assert_array_equal(np.flatnonzero(groups.mask(self.geo, "point", "3:2")), [3])

    def test_parenthesized_attribute_expressions(self):
        for domain, name in (("point", "top"), ("prim", "rim")):
            expected = self.geo.table(domain)["group_" + name] != 0
            for expr in (f"(@group_{name})", f"( ( @group_{name} ) )"):
                with self.subTest(domain=domain, expr=expr):
                    np.testing.assert_array_equal(groups.mask(self.geo, domain, expr), expected)
        np.testing.assert_array_equal(groups.mask(self.geo, "point", "((@P.x))"), self.geo.P[:, 0] != 0)
        np.testing.assert_array_equal(groups.mask(self.geo, "point", "(@P.x > 0)"), self.geo.P[:, 0] > 0)

    def test_conversion_between_points_and_primitives(self):
        g = self.geo
        rim_pts = groups.mask(g, "point", "rim")                  # points of the boundary prims: all 25 but the centre
        self.assertEqual(int(rim_pts.sum()), 24)
        self.assertFalse(rim_pts[12])
        top_prims = groups.mask(g, "prim", "top")                 # prims whose points are all in 'top'
        self.assertEqual(int(top_prims.sum()), 4)


class GroupCreateTests(unittest.TestCase):
    def test_methods(self):
        nodes, last = with_groups(
            {"name": "box", "method": "bounding_box", "box_center": [0, 0, 0], "box_size": [2.1, 2.1, 1]},
            {"name": "up", "domain": "primitives", "method": "normal", "direction": [0, 0, 1], "angle": 10},
            {"name": "down", "domain": "primitives", "method": "normal", "direction": [0, 0, -1], "angle": 10},
            {"name": "some", "method": "random", "percent": 50, "seed": 3})
        g = cook(nodes, last)
        self.assertEqual(int(g.point["group_box"].sum()), 9)       # 3x3 points within +-1.05
        self.assertEqual(int(g.prim["group_up"].sum()), 16)         # the grid faces +Z
        self.assertEqual(int(g.prim["group_down"].sum()), 0)
        self.assertTrue(5 <= int(g.point["group_some"].sum()) <= 20)
        self.assertEqual(g.point["group_some"].dtype, np.int32)

    def test_combine_modes(self):
        base = ({"name": "a", "pattern": "0-9"},)
        for combine, want in (("replace", [5, 15]), ("union", [0, 15]), ("intersect", [5, 9]), ("subtract", [0, 4])):
            nodes, last = with_groups(*base, {"name": "a", "pattern": "5-15", "combine": combine})
            idx = np.flatnonzero(cook(nodes, last).point["group_a"])
            self.assertEqual([idx.min(), idx.max()], want, combine)

    def test_a_name_lives_on_one_domain_and_promote(self):
        nodes, last = with_groups({"name": "a", "pattern": "0"}, {"name": "a", "domain": "primitives", "pattern": "0"})
        g = cook(nodes, last)
        self.assertNotIn("group_a", g.point)
        self.assertIn("group_a", g.prim)
        nodes["p"] = {"type": "group_promote", "inputs": {"geo": last}, "params": {"name": "a", "to": "points"}}
        g = cook(nodes, "p")
        self.assertEqual(np.flatnonzero(g.point["group_a"]).tolist(), [0, 1, 5, 6])
        self.assertNotIn("group_a", g.prim)
        nodes["bad"] = {"type": "group_promote", "inputs": {"geo": "p"}, "params": {"name": "zzz", "to": "points"}}
        with self.assertRaises(NodeError):
            cook(nodes, "bad")

    def test_promote_merged_same_name_groups_uses_source_domain(self):
        nodes = dict(GRID)
        for branch, domain in (("pt", "points"), ("pr", "primitives")):
            previous = "g"
            for name in ("shared_a", "shared_b"):
                nid = branch + name
                nodes[nid] = {"type": "group_create", "inputs": {"geo": previous},
                              "params": {"name": name, "domain": domain, "pattern": "*"}}
                previous = nid
        nodes["m"] = {"type": "merge", "inputs": {"geos": ["ptshared_b", "prshared_b"]}}
        merged = cook(nodes, "m")
        saved = {dom: {key: value.copy() for key, value in merged.table(dom).items()}
                 for dom in ("point", "prim", "vertex")}
        for name in ("shared_a", "shared_b"):
            np.testing.assert_array_equal(groups.mask(merged, "point", name), np.r_[np.ones(25), np.zeros(25)])
            np.testing.assert_array_equal(groups.mask(merged, "prim", name), np.r_[np.zeros(16), np.ones(16)])
        from minihou.nodes.group import group_promote
        for target, src, dst, expected in (
                ("points", "prim", "point", np.r_[np.zeros(25), np.ones(25)]),
                ("primitives", "point", "prim", np.r_[np.ones(16), np.zeros(16)])):
            for pattern in ("shared_a", "shared_*"):
                with self.subTest(target=target, pattern=pattern):
                    out = group_promote(merged, pattern, target)
                    out.validate()
                    np.testing.assert_array_equal(out.P, merged.P)
                    np.testing.assert_array_equal(out.topology.vertex_point, merged.topology.vertex_point)
                    np.testing.assert_array_equal(out.topology.prim_offsets, merged.topology.prim_offsets)
                    names = ("shared_a", "shared_b") if pattern.endswith("*") else ("shared_a",)
                    for name in names:
                        np.testing.assert_array_equal(out.table(dst)["group_" + name], expected)
                        self.assertNotIn("group_" + name, out.table(src))
                    for dom, attrs in saved.items():
                        for key, value in attrs.items():
                            np.testing.assert_array_equal(merged.table(dom)[key], value)
                            if key not in {"group_" + name for name in names}:
                                np.testing.assert_array_equal(out.table(dom)[key], value)

    def test_bad_name(self):
        nodes, last = with_groups({"name": "9lives"})
        with self.assertRaises(NodeError) as cm:
            cook(nodes, last)
        self.assertIn("invalid group name", str(cm.exception))


class BlastTests(unittest.TestCase):
    def blast(self, **params):
        nodes, last = with_groups({"name": "rim", "domain": "primitives", "method": "boundary"},
                                  {"name": "corner", "pattern": "0"})
        nodes["b"] = {"type": "blast", "inputs": {"geo": last}, "params": params}
        return cook(nodes, "b")

    def test_primitives(self):
        g = self.blast(group="rim")
        self.assertEqual((g.npoints, g.nprims), (9, 4))             # unused points go too
        self.assertEqual(int(g.prim["group_rim"].sum()), 0)
        g.validate()
        g = self.blast(group="rim", keep_unused_points=True)
        self.assertEqual((g.npoints, g.nprims), (25, 4))
        g = self.blast(group="rim", delete_non_selected=True)
        self.assertEqual(g.nprims, 12)

    def test_separator_only_pattern_has_empty_field_semantics(self):
        self.assertEqual(self.blast(group=", ,").nprims, 16)
        self.assertEqual(self.blast(group=", ,", delete_non_selected=True).nprims, 0)
        nodes, last = with_groups({"name": "all", "pattern": ", ,"})
        self.assertEqual(int(cook(nodes, last).point["group_all"].sum()), 25)

    def test_points_and_expressions(self):
        g = self.blast(group="corner", domain="points")
        self.assertEqual((g.npoints, g.nprims), (24, 15))
        g = self.blast(group="@P.x > 1.5", domain="points")
        self.assertEqual((g.npoints, g.nprims), (20, 12))
        self.assertEqual(self.blast(group="").nprims, 16)            # empty field deletes nothing


class GroupFieldTests(unittest.TestCase):
    def test_transform_noise_and_wrangle_only_touch_the_group(self):
        nodes, last = with_groups({"name": "top", "pattern": "@P.y > 0"})
        nodes["t"] = {"type": "transform", "inputs": {"geo": last}, "params": {"translate": [0, 0, 1], "group": "top"}}
        nodes["n"] = {"type": "noise", "inputs": {"geo": "t"}, "params": {"attrib": "h", "group": "^top", "seed": 2,
                                                                        "frequency": 0.37, "amplitude": [1, 1, 1]}}
        nodes["w"] = {"type": "wrangle", "inputs": {"geo": "n"},
                      "params": {"snippet": "f@k = 7; i@group_hot = 1", "group": "top"}}
        nodes["v"] = {"type": "wrangle", "inputs": {"geo": "w"},
                      "params": {"snippet": "f@pk = 3", "run_over": "vertices", "group": "0"}}
        g = cook(nodes, "v")
        top = g.point["group_top"] == 1
        np.testing.assert_allclose(g.P[top, 2], 1)
        np.testing.assert_allclose(g.P[~top, 2], 0)
        self.assertTrue(np.all(g.point["h"][top] == 0) and np.any(g.point["h"][~top] != 0))
        np.testing.assert_array_equal(g.point["k"], np.where(top, 7, 0))
        np.testing.assert_array_equal(g.point["group_hot"], top.astype(np.int32))   # a wrangle makes groups
        self.assertEqual(g.vertex["pk"].tolist(), [3, 3, 3, 3] + [0] * 60)     # vertices of primitive 0

    def test_wrangle_reads_groups_like_vex(self):
        nodes, last = with_groups({"name": "top", "pattern": "@P.y > 0"})
        nodes["w"] = {"type": "wrangle", "inputs": {"geo": last}, "params": {"snippet": "@P.z = @group_top * 2"}}
        g = cook(nodes, "w")
        np.testing.assert_allclose(g.P[:, 2], g.point["group_top"] * 2)

    def test_groups_survive_merge_and_copy(self):
        nodes, last = with_groups({"name": "top", "pattern": "@P.y > 0"})
        nodes["m"] = {"type": "merge", "inputs": {"geos": [last, "g"]}}
        g = cook(nodes, "m")
        self.assertEqual(int(g.point["group_top"].sum()), 10)          # the ungrouped copy is zero-filled


if __name__ == "__main__":
    unittest.main()
