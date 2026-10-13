"""Painted attributes: the data codec, the attrib_paint node (points, primitives, edge groups, stale and damaged data) and the
paint_assign edit (one undo step, erase, clear, default)."""
import unittest

import numpy as np

import minihou  # noqa: F401
from minihou import Evaluator, Graph, edit, edges, paint, topo
from minihou.edit import EditError
from tests.test_nodes import run


def graph(domain="point", kind="float", **params):
    params.setdefault("attribute", "w")
    return Graph.from_dict({"nodes": {"g": {"type": "grid", "params": {"columns": 4, "rows": 3}},
                                      "p": {"type": "attrib_paint", "inputs": {"geo": "g"},
                                            "params": dict(domain=domain, kind=kind, **params)}}, "display": "p"})


class Codec(unittest.TestCase):
    def test_round_trip_and_newest_value_wins(self):
        t = paint.merge("", 3, [2, 5], [[1, 2, 3], [4, 5, 6]])
        self.assertTrue(t.startswith("mhp1:"))
        t = paint.merge(t, 3, [5, 7], [[9, 9, 9], [1, 1, 1]])
        i, v = paint.decode(t, 3)
        self.assertEqual(i.tolist(), [2, 5, 7])
        np.testing.assert_array_equal(v, [[1, 2, 3], [9, 9, 9], [1, 1, 1]])
        self.assertEqual(paint.decode(paint.merge(t, 3, [2, 7], erase=True), 3)[0].tolist(), [5])
        self.assertEqual(paint.merge(paint.merge("", 1, [4], [[1]]), 1, [4], erase=True), "")
        self.assertEqual(paint.decode("", 1)[0].tolist(), [])

    def test_damaged_data_is_a_clean_error(self):
        good = paint.merge("", 1, [1, 2], [[1], [2]])
        for bad in ("garbage", "mhp1:@@@", "mhp1:" + good[5:-6], "mhp1:", good[:-4]):
            with self.assertRaises(ValueError, msg=bad):
                paint.decode(bad, 1)
        with self.assertRaisesRegex(ValueError, "does not fit"):
            paint.decode(good, 3)                                                   # painted as float, read as vec3
        with self.assertRaisesRegex(ValueError, "negative"):
            paint.merge("", 1, [-1], [[1]])
        with self.assertRaisesRegex(ValueError, "not a number"):
            paint.merge("", 1, [1], [[float("nan")]])

    def test_size_caps(self):
        with self.assertRaisesRegex(ValueError, "too many"):
            paint.encode(np.arange(paint.MAX_EDITS + 1), np.zeros(paint.MAX_EDITS + 1))

    def test_names(self):
        for bad in ("", "1a", "has space", "P"):
            with self.assertRaises(ValueError):
                paint.check_name(bad, "point")
        with self.assertRaises(ValueError):
            paint.check_name("edge_group_a", "edge")
        self.assertEqual(paint.check_name("weight_2", "prim"), "weight_2")

    def test_brush_arithmetic(self):
        t = np.array([0.0, 0.5, 1.0, 2.0])
        np.testing.assert_allclose(paint.falloff(t, "linear"), [1, 0.5, 0, 0])
        np.testing.assert_allclose(paint.falloff(t, "constant"), [1, 1, 0, 0])
        self.assertAlmostEqual(float(paint.falloff(0.5, "smooth")), (1 - 0.25) ** 2)
        np.testing.assert_allclose(paint.blend([0.0, 1.0], 1.0, [0.5, 0.5], "set"), [0.5, 1.0])
        np.testing.assert_allclose(paint.blend([0.0, 1.0], 0.5, [1.0, 0.5], "add"), [0.5, 1.25])
        np.testing.assert_allclose(paint.blend([2.0, 2.0], 0.0, [1.0, 0.5], "multiply"), [0.0, 1.0])
        np.testing.assert_allclose(paint.blend([[0, 0, 0]], [1, 1, 1], [0.25], "set"), [[0.25] * 3])


class Node(unittest.TestCase):
    def cook(self, g):
        return Evaluator().cook(g)

    def test_points_prims_and_edges_follow_the_viewers_numbering(self):
        for domain, n in (("point", 12), ("prim", 6)):
            g = graph(domain, data=paint.merge("", 1, [1, 4], [[0.25], [0.75]]))
            out = self.cook(g)
            a = out.table(domain)["w"]
            self.assertEqual(a.shape, (n,))
            self.assertEqual(a.dtype, np.float32)
            self.assertEqual(a.nonzero()[0].tolist(), [1, 4])
            self.assertEqual((a[1], a[4]), (np.float32(0.25), np.float32(0.75)))
        geo = self.cook(graph("point"))
        e = topo.edges(geo)
        g = graph("edge", kind="int", data=paint.merge("", 1, [0, 5], [[1], [1]]))
        out = self.cook(g)
        self.assertIn("edge_group_w", out.vertex)
        member = edges.values(out, "edge_group_w") != 0
        self.assertEqual(member.nonzero()[0].tolist(), [0, 5])                      # derived edge ids, as topo.edges numbers them
        self.assertEqual(len(member), len(e.points))

    def test_unpainted_elements_keep_the_inputs_value_or_the_default(self):
        g = Graph.from_dict({"nodes": {"g": {"type": "grid", "params": {"columns": 3, "rows": 3}},
                                       "a": {"type": "attrib_paint", "inputs": {"geo": "g"}, "params": {"attribute": "w", "default": 0.5}},
                                       "b": {"type": "attrib_paint", "inputs": {"geo": "a"},
                                             "params": {"attribute": "w", "default": 9.0, "data": paint.merge("", 1, [2], [[7]])}}}, "display": "b"})
        out = self.cook(g)
        self.assertEqual(out.point["w"].tolist(), [0.5, 0.5, 7.0] + [0.5] * 6)          # the input's 0.5 beats the second node's default

    def test_int_vec3_and_default_vec(self):
        out = self.cook(graph("point", "int", data=paint.merge("", 1, [3], [[2.6]]), default=1.0))
        self.assertEqual(out.point["w"].dtype, np.int32)
        self.assertEqual((out.point["w"][3], out.point["w"][0]), (3, 1))
        out = self.cook(graph("prim", "vec3", data=paint.merge("", 3, [0], [[1, 0, 0]]), default_vec=(0.2, 0.2, 0.2)))
        self.assertEqual(out.prim["w"].shape, (6, 3))
        np.testing.assert_allclose(out.prim["w"][:2], [[1, 0, 0], [0.2, 0.2, 0.2]])

    def test_a_float_input_becomes_a_vector_and_stale_edits_are_counted_not_fatal(self):
        g = Graph.from_dict({"nodes": {"g": {"type": "grid", "params": {"columns": 3, "rows": 3}},
                                       "w": {"type": "wrangle", "inputs": {"geo": "g"}, "params": {"snippet": "f@w = @ptnum;"}},
                                       "p": {"type": "attrib_paint", "inputs": {"geo": "w"},
                                             "params": {"attribute": "w", "kind": "vec3", "data": paint.merge("", 3, [1, 500], [[1, 1, 1], [2, 2, 2]])}}},
                            "display": "p"})
        out = self.cook(g)
        self.assertEqual(out.point["w"].shape, (9, 3))
        np.testing.assert_allclose(out.point["w"][3], [3, 3, 3])                         # the float, broadcast
        np.testing.assert_allclose(out.point["w"][1], [1, 1, 1])
        self.assertEqual(out.detail["paint_stale_w"], 1)

    def test_bad_data_name_or_kind_change_fail_cleanly_and_empty_geometry_works(self):
        for g in (graph(data="mhp1:zzzz"), graph(attribute="P"), graph(data=paint.merge("", 1, [0], [[1]]), kind="vec3")):
            with self.assertRaises(Exception):
                self.cook(g)
        empty = run("attrib_paint", run("blast", run("grid", columns=3, rows=3), group="*", domain="points"), attribute="w")
        self.assertEqual(len(empty.point["w"]), 0)


class Assign(unittest.TestCase):
    def apply(self, g, *ops):
        return edit.apply_ops(g, list(ops))[0]

    def test_assign_erase_clear_and_default(self):
        g = graph("point", "float")
        g = self.apply(g, {"op": "paint_assign", "id": "p", "indices": [1, 2], "value": 0.5})
        g = self.apply(g, {"op": "paint_assign", "id": "p", "indices": [2, 3], "values": [1.0, 0.25]})
        out = Evaluator().cook(g)
        self.assertEqual(out.point["w"][:5].tolist(), [0, 0.5, 1.0, 0.25, 0])
        g = self.apply(g, {"op": "paint_assign", "id": "p", "indices": [1], "erase": True})
        self.assertEqual(Evaluator().cook(g).point["w"][1], 0)
        g = self.apply(g, {"op": "paint_assign", "id": "p", "clear": True, "default": 0.75})
        out = Evaluator().cook(g)
        self.assertTrue((out.point["w"] == 0.75).all())
        self.assertEqual(g.nodes["p"].params["data"], "")

    def test_one_assign_is_one_session_edit_and_undo_restores_it(self):
        from minihou.link.session import Session
        s = Session(graph("point"))
        s.start()
        self.addCleanup(s.stop)
        s.edit([{"op": "paint_assign", "id": "p", "indices": [0, 1, 2], "value": 1.0}])
        self.assertTrue(s.graph.nodes["p"].params["data"].startswith("mhp1:"))
        s.undo()
        self.assertEqual(s.graph.nodes["p"].params.get("data", ""), "")

    def test_bad_assignments_are_edit_errors(self):
        g = graph("point")
        for op in ({"op": "paint_assign", "id": "g", "indices": [0], "value": 1},
                   {"op": "paint_assign", "id": "p", "indices": [0.5], "value": 1},
                   {"op": "paint_assign", "id": "p", "indices": [0, 1], "values": [1]},
                   {"op": "paint_assign", "id": "p", "indices": [0]},
                   {"op": "paint_assign", "id": "p", "indices": [-3], "value": 1},
                   {"op": "paint_assign", "id": "p", "indices": [0], "value": [1, 2, 3]}):
            with self.assertRaises(EditError, msg=op):
                edit.apply_ops(g, [op])

    def test_int_assignments_round(self):
        g = self.apply(graph("prim", "int"), {"op": "paint_assign", "id": "p", "indices": [0], "value": 2.7})
        self.assertEqual(Evaluator().cook(g).prim["w"][0], 3)


class Views(unittest.TestCase):
    """The spreadsheet pages and the preview's attribute option, directly and over HTTP."""

    def painted(self):
        g = graph("point", data=paint.merge("", 1, [1, 5], [[0.5], [1.0]]))
        return Evaluator().cook(g)

    def test_spreadsheet_domains_paging_selection_and_cap(self):
        from minihou import spreadsheet
        geo = self.painted()
        r = spreadsheet.page(geo, "point", 0, 3)
        self.assertEqual((r["total"], [c["name"] for c in r["columns"]]), (12, ["P", "uv", "w"]))
        self.assertEqual([row[0] for row in r["rows"]], [0, 1, 2])
        self.assertEqual(r["rows"][1][3], 0.5)
        self.assertEqual(r["rows"][1][1], [-1.66667, -5.0, 0.0])                       # rounded to 6 digits
        self.assertEqual([row[0] for row in spreadsheet.page(geo, "point", 10, 50)["rows"]], [10, 11])
        sel = spreadsheet.page(geo, "point", 1, 5, ids=[7, 3, 5, 99, -1])                  # sorted, deduplicated, valid only; paged within
        self.assertEqual((sel["total"], [row[0] for row in sel["rows"]]), (3, [5, 7]))
        self.assertEqual(len(spreadsheet.page(geo, "point", 0, 10**6)["rows"]), 12)
        self.assertEqual(spreadsheet.page(geo, "prim", 0, 2)["total"], 6)
        self.assertEqual([c["name"] for c in spreadsheet.page(geo, "vertex", 0, 1)["columns"]], ["point", "prim"])
        e = spreadsheet.page(geo, "edge", 0, 2)
        self.assertEqual((e["total"], [c["name"] for c in e["columns"]][:2]), (len(topo.edges(geo).points), ["a", "b"]))
        d = spreadsheet.page(geo, "detail", 0, 5)
        self.assertEqual(d["rows"][0][1:], ["grid_columns", 4])
        with self.assertRaises(ValueError):
            spreadsheet.page(geo, "wire", 0, 1)

    def test_spreadsheet_sorts_filters_and_pages_the_result(self):
        from minihou import spreadsheet
        g = graph("point", data=paint.merge("", 1, [1, 5, 9], [[0.5], [1.0], [0.25]]))
        geo = Evaluator().cook(g)
        r = spreadsheet.page(geo, "point", 0, 5, sort={"column": "w", "desc": True})
        self.assertEqual([row[0] for row in r["rows"]], [5, 1, 9, 0, 2])                       # descending; ties keep ascending element order
        self.assertEqual(r["total"], 12)
        asc = spreadsheet.page(geo, "point", 0, 12, sort={"column": "w"})
        self.assertEqual([row[0] for row in asc["rows"]][-3:], [9, 1, 5])                      # 0.25, 0.5, 1.0 last
        f = spreadsheet.page(geo, "point", 0, 50, filter="@w > 0.4")
        self.assertEqual((f["total"], [row[0] for row in f["rows"]]), (2, [1, 5]))
        both = spreadsheet.page(geo, "point", 0, 50, filter="@w > 0.1", sort={"column": "w", "desc": True})
        self.assertEqual([row[0] for row in both["rows"]], [5, 1, 9])
        sel = spreadsheet.page(geo, "point", 0, 50, ids=[9, 5, 2], filter="@w > 0.1", sort={"column": "w", "desc": True})   # selected only, in the table's order
        self.assertEqual([row[0] for row in sel["rows"]], [5, 9])
        rng = spreadsheet.page(geo, "point", 0, 50, filter="0-2")
        self.assertEqual([row[0] for row in rng["rows"]], [0, 1, 2])
        z = spreadsheet.page(geo, "point", 0, 3, sort={"column": "P", "comp": "y", "desc": True})   # a vector column by one component
        self.assertGreaterEqual(z["rows"][0][1][1], z["rows"][1][1][1])
        ln = spreadsheet.page(geo, "point", 0, 3, sort={"column": "P", "comp": "len"})
        self.assertEqual(ln["total"], 12)
        for bad in ({"column": "nope"}, {"column": "w", "comp": 7}, {"nocolumn": 1}):
            with self.assertRaises(ValueError, msg=str(bad)):
                spreadsheet.page(geo, "point", 0, 3, sort=bad)
        with self.assertRaises(ValueError):
            spreadsheet.page(geo, "point", 0, 3, filter="no_such_group")
        e = spreadsheet.page(geo, "edge", 0, 500, filter="faces == 1 and a < 3")
        self.assertTrue(e["total"] > 0 and all(row[1] < 3 for row in e["rows"]))
        e2 = spreadsheet.page(geo, "edge", 0, 5, sort={"column": "a", "desc": True})
        self.assertGreaterEqual(e2["rows"][0][1], e2["rows"][-1][1])
        with self.assertRaises(ValueError):
            spreadsheet.page(geo, "edge", 0, 5, filter="a ~ 3")
        with self.assertRaises(ValueError):
            spreadsheet.page(geo, "vertex", 0, 5, filter="nope > 1")

    def test_sorted_orders_are_remembered_per_frozen_geometry(self):
        from minihou import spreadsheet
        geo = self.painted()
        self.assertTrue(geo.frozen)                                                           # cooked geometry is frozen
        a = spreadsheet.page(geo, "point", 0, 3, sort={"column": "w", "desc": True})
        b = spreadsheet.page(geo, "point", 3, 3, sort={"column": "w", "desc": True})
        self.assertEqual(a["total"], b["total"])
        self.assertEqual(len(spreadsheet._ORDERS[geo]), 1)

    def test_column_origins_name_the_node_that_made_each_attribute(self):
        from minihou.link.session import Session
        g = Graph.from_dict({"nodes": {"g": {"type": "grid", "params": {"columns": 4, "rows": 3}},
                                       "p": {"type": "attrib_paint", "inputs": {"geo": "g"}, "params": {"attribute": "w", "domain": "point", "kind": "float"}},
                                       "t": {"type": "transform", "inputs": {"geo": "p"}, "params": {"translate": [1, 0, 0]}}}, "display": "t"})
        s = Session(g)
        s.start()
        self.addCleanup(s.stop)
        self.assertEqual(s.column_origins("t", "point", ["uv", "w"]), {"uv": "g", "w": "p"})
        self.assertEqual(s.column_origins("p", "point", ["w"]), {"w": "p"})
        self.assertEqual(s.column_origins("nope", "point", ["w"]), {})

    def test_preview_attr_arrays_use_the_viewers_numbering(self):
        from minihou import preview
        geo = self.painted()
        head, arrays = preview.arrays_for(geo, full=True)
        arrays = dict(arrays)
        preview.attr_arrays(geo, {"name": "w", "domain": "point"}, head, arrays)
        self.assertEqual(head["attr"]["comps"], 1)
        self.assertEqual((head["attr"]["min"], head["attr"]["max"]), ([0.0], [1.0]))
        self.assertEqual(arrays["attr"][[1, 5]].tolist(), [0.5, 1.0])
        np.testing.assert_array_equal(arrays["edge"].reshape(-1, 2), topo.edges(geo).points)             # edge k in the viewer is derived edge k
        head2, a2 = preview.arrays_for(geo, full=True)
        preview.attr_arrays(geo, {"name": "nothing", "domain": "prim"}, head2, dict(a2))
        self.assertTrue(head2["attr"]["missing"])
        eg = Evaluator().cook(graph("edge", "int", data=paint.merge("", 1, [2], [[1]])))
        h3, a3 = preview.arrays_for(eg, full=True)
        a3 = dict(a3)
        preview.attr_arrays(eg, {"name": "w", "domain": "edge"}, h3, a3)
        self.assertEqual(a3["attr"].nonzero()[0].tolist(), [2])
        h4, a4 = preview.arrays_for(geo, full=False, tris=4)                                          # a reduced view has no real numbering
        preview.attr_arrays(geo, {"name": "w", "domain": "point"}, h4, dict(a4))
        self.assertTrue(h4["attr"].get("reduced"))

    def test_preview_mask_arrays_are_group_membership_in_the_viewers_numbering(self):
        from minihou import preview
        g = Graph.from_dict({"nodes": {"g": {"type": "grid", "params": {"columns": 4, "rows": 3}},
                                       "grp": {"type": "group_create", "inputs": {"geo": "g"},
                                               "params": {"name": "left", "domain": "points", "method": "pattern", "pattern": "0-2 4-6"}}},
                             "display": "grp"})
        geo = Evaluator().cook(g)
        head, arrays = preview.arrays_for(geo, full=True)
        arrays = dict(arrays)
        preview.mask_arrays(geo, {"name": "left", "domain": "point"}, head, arrays)
        self.assertEqual(arrays["mask"].nonzero()[0].tolist(), [0, 1, 2, 4, 5, 6])
        self.assertEqual((head["mask"]["count"], head["mask"]["members"]), (12, 6))
        h2, a2 = preview.arrays_for(geo, full=True)
        a2 = dict(a2)
        preview.mask_arrays(geo, {"name": "left", "domain": "prim"}, h2, a2)                     # a point group on primitives: faces whose points are all in it
        self.assertTrue(set(a2["mask"].nonzero()[0].tolist()) <= {0, 1, 3, 4})
        self.assertGreater(h2["mask"]["members"], 0)
        h3, a3 = preview.arrays_for(geo, full=True)
        preview.mask_arrays(geo, {"name": "nope", "domain": "point"}, h3, dict(a3))
        self.assertTrue(h3["mask"]["missing"])
        h4, a4 = preview.arrays_for(geo, full=False, tris=4)
        preview.mask_arrays(geo, {"name": "left", "domain": "point"}, h4, dict(a4))
        self.assertTrue(h4["mask"].get("reduced"))
        with self.assertRaises(ValueError):
            preview.mask_arrays(geo, {"name": "left", "domain": "wire"}, *preview.arrays_for(geo, full=True))

    def test_http_spreadsheet_and_preview_attr(self):
        import http.client
        import json
        import tempfile
        from minihou.link.session import Session
        from minihou.web import WebServer
        d = tempfile.mkdtemp(prefix="minihou_paint_")
        self.addCleanup(__import__("shutil").rmtree, d, True)
        s = Session(graph("point", data=paint.merge("", 1, [1], [[0.5]])))
        s.start()
        self.addCleanup(s.stop)
        web = WebServer(s, "127.0.0.1", 0, root=d)
        port = web.start()
        self.addCleanup(web.stop)

        def post(path, body):
            c = http.client.HTTPConnection("127.0.0.1", port, timeout=15)
            c.request("POST", path, body=json.dumps(body), headers={"Content-Type": "application/json"})
            r = c.getresponse()
            data = r.read()
            c.close()
            return r.status, data
        st, raw = post("/api/spreadsheet", {"node": "p", "domain": "point", "start": 0, "count": 2})
        out = json.loads(raw)
        self.assertEqual((st, out["total"], len(out["rows"]), out["cook_level"]), (200, 12, 2, 1.0))
        self.assertEqual(post("/api/spreadsheet", {"node": "p", "domain": "nope"})[0], 400)
        self.assertEqual(post("/api/spreadsheet", {"node": "p", "ids": ["x"]})[0], 400)
        self.assertFalse(json.loads(post("/api/spreadsheet", {"node": "missing"})[1])["ok"])
        st, raw = post("/api/preview", {"node": "p", "full": True, "attr": {"name": "w", "domain": "point"}})
        self.assertEqual(st, 200)
        from minihou import preview
        head, arrays = preview.unpack(raw)
        self.assertEqual((head["attr"]["name"], head["attr"]["comps"], head["attr"]["max"]), ("w", 1, [0.5]))
        self.assertEqual(arrays["attr"][1], 0.5)
        self.assertEqual(post("/api/preview", {"node": "p", "attr": {"name": "w", "domain": "wire"}})[0], 400)


if __name__ == "__main__":
    unittest.main()
