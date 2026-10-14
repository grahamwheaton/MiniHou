"""Group editing keeps geometry and attributes while changing only memberships."""
import unittest

import numpy as np

from minihou import Evaluator, Graph, NodeError, REGISTRY, groups
from minihou.nodes import group_rules as R


def cook(steps):
    nodes = {"box": {"type": "box", "params": {"size": [2, 2, 2]}}}
    previous = "box"
    for i, (kind, params) in enumerate(steps):
        name = f"n{i}"
        nodes[name] = {"type": kind, "inputs": {"geo": previous}, "params": params}
        previous = name
    return Evaluator().cook(Graph.from_dict({"nodes": nodes, "display": previous}))


class GroupRuleTests(unittest.TestCase):
    def test_four_domains_and_immutable_topology(self):
        original = cook([])
        for dom, selection in (("points", "0-2"), ("primitives", "0-2"),
                               ("edges", "0-2"), ("vertices", "0-2")):
            with self.subTest(domain=dom):
                out = cook([("group_paint", {"domain": dom, "name": "picked", "selection": selection})])
                self.assertEqual(int(R.membership(out, dom, "picked").sum()), 3)
                self.assertEqual(R.names(out, dom), ["picked"])
                self.assertEqual(out.topology.digest, original.topology.digest)
                np.testing.assert_array_equal(out.P, original.P)
                self.assertEqual(R.names(original, dom), [])
        empty = cook([("group_lasso", {"name": "empty", "selection": ""})])
        self.assertFalse(R.membership(empty, "point", "empty").any())

    def test_group_create_vertex_pattern_and_expression(self):
        numbered = cook([("group_create", {"name": "corners", "domain": "vertices",
                                          "method": "pattern", "pattern": "0-2"})])
        self.assertEqual(int(R.membership(numbered, "vertex", "corners").sum()), 3)
        expressed = cook([("group_create", {"name": "upper", "domain": "vertices",
                                           "method": "pattern", "pattern": "@P.z > 0"})])
        self.assertGreater(int(R.membership(expressed, "vertex", "upper").sum()), 0)
        self.assertEqual(numbered.topology.digest, expressed.topology.digest)

    def test_delete_removes_group_metadata_only_and_unused(self):
        src = cook([("group_paint", {"name": "a", "selection": "0-2"}),
                    ("group_paint", {"name": "empty", "selection": ""}),
                    ("group_paint", {"name": "other", "domain": "vertices", "selection": "0"})])
        out = cook([("group_paint", {"name": "a", "selection": "0-2"}),
                    ("group_paint", {"name": "empty", "selection": ""}),
                    ("group_paint", {"name": "other", "domain": "vertices", "selection": "0"}),
                    ("group_delete", {"rules": [{"domain": "points", "name": "a"}], "delete_unused": True})])
        self.assertEqual(out.topology.digest, src.topology.digest)
        self.assertEqual((out.npoints, out.nprims), (src.npoints, src.nprims))
        self.assertEqual(R.names(out, "point"), [])
        self.assertEqual(R.names(out, "vertex"), ["other"])

    def test_promote_rules_chain_auto_any_contained_and_attribute(self):
        setup = [("group_paint", {"name": "seed", "selection": "0"})]
        any_rule = {"from": "auto", "to": "primitives", "name": "seed",
                    "new_name": "faces", "keep_original": True}
        out = cook(setup + [("group_promote", {"rules": [any_rule,
                    {"from": "primitives", "to": "points", "name": "faces", "new_name": "back",
                     "keep_original": True}]})])
        self.assertGreater(int(R.membership(out, "prim", "faces").sum()), 0)
        self.assertGreater(int(R.membership(out, "point", "back").sum()), 0)
        self.assertIn("seed", R.names(out, "point"))
        contained = cook(setup + [("group_promote", {"rules": [dict(any_rule, contained=True)]})])
        self.assertEqual(int(R.membership(contained, "prim", "faces").sum()), 0)
        attr = cook(setup + [("group_promote", {"rules": [{"from": "points", "to": "primitives",
                            "name": "seed", "output_attribute": True, "attribute_name": "hit"}]})])
        self.assertIn("hit", attr.prim)
        self.assertEqual(attr.prim["hit"].dtype, np.int32)
        self.assertNotIn("group_seed", attr.point)
        self.assertNotIn("group_seed", attr.prim)
        same_domain = cook(setup + [("group_promote", {"rules": [
            {"from": "points", "to": "points", "name": "seed", "output_attribute": True,
             "attribute_name": "seed_integer"}]})])
        self.assertIn("seed_integer", same_domain.point)
        self.assertNotIn("group_seed", same_domain.point)

    def test_promote_disabled_rule_and_sequential_wildcard_repromotion(self):
        out = cook([("group_paint", {"name": "piece_a", "selection": "0"}),
                    ("group_paint", {"name": "piece_b", "selection": "1"}),
                    ("group_promote", {"rules": [
                        {"enabled": False, "from": "points", "to": "edges", "name": "piece_*"},
                        {"from": "points", "to": "primitives", "name": "piece_*", "new_name": "face_*"},
                        {"from": "primitives", "to": "edges", "name": "face_*", "new_name": "rim_*"},
                    ]})])
        self.assertEqual(R.names(out, "edge"), ["rim_a", "rim_b"])
        self.assertEqual(R.names(out, "point"), [])
        self.assertEqual(R.names(out, "prim"), [])

    def test_promote_edge_vertex_and_explicit_unsupported_diagnostic(self):
        out = cook([("group_paint", {"name": "one", "domain": "edges", "selection": "0"}),
                    ("group_promote", {"rules": [{"from": "edges", "to": "vertices", "name": "one"}]})])
        self.assertGreaterEqual(int(R.membership(out, "vertex", "one").sum()), 2)
        with self.assertRaisesRegex(NodeError, "connectivity attribute"):
            cook([("group_paint", {"name": "one", "selection": "0"}),
                  ("group_promote", {"rules": [{"from": "points", "to": "primitives", "name": "one",
                                                  "boundary": True, "connectivity_attribute": "uv"}]})])

    def test_edge_and_vertex_groups_resolve_in_ordinary_point_primitive_fields(self):
        edge = cook([("group_paint", {"name": "rim", "domain": "edges", "selection": "0"})])
        self.assertEqual(groups.names(edge, "edge"), ["rim"])
        self.assertEqual(int(groups.mask(edge, "point", "rim").sum()), 2)
        self.assertGreaterEqual(int(groups.mask(edge, "prim", "rim").sum()), 1)
        vertex = cook([("group_paint", {"name": "corner", "domain": "vertices", "selection": "0"})])
        self.assertEqual(groups.names(vertex, "vertex"), ["corner"])
        self.assertEqual(int(groups.mask(vertex, "point", "corner").sum()), 1)
        self.assertEqual(int(groups.mask(vertex, "prim", "corner").sum()), 1)
        expr = cook([("group_expression", {"domain": "vertices", "rules": [
            {"name": "upper", "expression": "@P.z > 0"}]})])
        self.assertGreater(int(R.membership(expr, "vertex", "upper").sum()), 0)
        self.assertEqual(int(R.select(expr, "vertex", "@P.z > 0").sum()),
                         int(R.membership(expr, "vertex", "upper").sum()))

    def test_combine_chains_rename_invert_expression(self):
        steps = [("group_paint", {"name": "a", "selection": "0-2"}),
                 ("group_paint", {"name": "b", "selection": "2-4"}),
                 ("group_combine", {"rules": [{"domain": "auto", "name": "result", "base": "a",
                    "operations": [{"op": "union", "group": "b"}, {"op": "subtract", "group": "a"}]}]}),
                 ("group_rename", {"rules": [{"domain": "points", "name": "result", "new_name": "chosen"}]}),
                 ("group_invert", {"domain": "points", "name": "chosen", "new_name": "other"}),
                 ("group_expression", {"domain": "points", "rules": [{"name": "above",
                    "expression": "@P.z > 0", "combine": "replace"}]})]
        out = cook(steps)
        self.assertEqual(np.flatnonzero(R.membership(out, "point", "chosen")).tolist(), [3, 4])
        self.assertEqual(int(R.membership(out, "point", "other").sum()), out.npoints - 2)
        self.assertEqual(int(R.membership(out, "point", "above").sum()), 4)
        self.assertNotIn("result", R.names(out, "point"))

    def test_wildcard_rename_and_empty_geometry(self):
        out = cook([("group_paint", {"name": "part_a", "selection": "0"}),
                    ("group_paint", {"name": "part_b", "selection": "1"}),
                    ("group_rename", {"rules": [{"domain": "points", "name": "part_*", "new_name": "piece_*"}]})])
        self.assertEqual(R.names(out, "point"), ["piece_a", "piece_b"])
        from minihou.geometry import Geometry
        blank = Geometry.points_only(np.empty((0, 3), np.float32))
        for dom in ("point", "prim", "edge", "vertex"):
            written = R.write(blank, dom, "empty", np.zeros(R.count(blank, dom), bool))
            self.assertEqual(R.membership(written, dom, "empty").shape, (0,))
            written.validate()

    def test_expand_edge_vertex_signed_steps_and_flood(self):
        for dom in ("edges", "vertices"):
            with self.subTest(domain=dom):
                seed = cook([("group_paint", {"name": "seed", "domain": dom, "selection": "0"})])
                out = cook([("group_paint", {"name": "seed", "domain": dom, "selection": "0"}),
                            ("group_expand", {"group": "seed", "domain": dom, "name": "grown", "steps": 1})])
                self.assertGreater(int(R.membership(out, dom, "grown").sum()), 1)
                contracted = cook([("group_paint", {"name": "seed", "domain": dom, "selection": "0"}),
                                   ("group_expand", {"group": "seed", "domain": dom, "name": "shrunk", "steps": -1})])
                self.assertFalse(R.membership(contracted, dom, "shrunk").any())
                flooded = cook([("group_paint", {"name": "seed", "domain": dom, "selection": "0"}),
                                ("group_expand", {"group": "seed", "domain": dom, "name": "all",
                                                  "steps": 1, "flood_fill": True})])
                self.assertEqual(int(R.membership(flooded, dom, "all").sum()), R.count(seed, dom))

    def test_vertex_create_uses_vertex_normals_and_random_seed_attribute(self):
        from minihou.geometry import Geometry
        from minihou.nodes.group import group_create, _hash01
        geo = Geometry.from_arrays(np.array([[0, 0, 0], [1, 0, 0], [0, 1, 0]], np.float32),
                                   [0, 1, 2], [0, 3])
        geo.vertex["N"] = np.array([[0, 0, 1], [0, 0, -1], [0, 0, -1]], np.float32)
        geo.vertex["seedid"] = np.array([1, 2, 3], np.int32)
        spec = next(n for n in REGISTRY if n.name == "group_create")
        defaults = {p.name: p.default for p in spec.params}
        params = dict(defaults, domain="vertices", method="filters", name="normal",
                      base_enable=False, normals_enable=True, direction=(0, 0, 1), angle=1)
        out = group_create(geo, None, **params)
        np.testing.assert_array_equal(R.membership(out, "vertex", "normal"), [True, False, False])
        random = group_create(geo, None, **dict(defaults, domain="vertices", method="filters", name="random",
                                                base_enable=False, random_enable=True, seed_attrib="seedid", percent=50))
        np.testing.assert_array_equal(R.membership(random, "vertex", "random"),
                                      _hash01(geo.vertex["seedid"], 0) < .5)
        with self.assertRaisesRegex(ValueError, "require an edge group"):
            group_create(geo, None, **dict(defaults, domain="vertices", method="filters", name="bad",
                                                edges_enable=True, min_edge_length_on=True))
        with self.assertRaisesRegex(ValueError, "ordered groups are unsupported"):
            group_create(geo, None, **dict(defaults, ordered=True))

    def test_promote_attribute_seams_tolerance_and_point_sharing(self):
        from minihou.geometry import Geometry
        from minihou.nodes.group import group_promote
        geo = Geometry.from_arrays(np.array([[0, 0, 0], [1, 0, 0], [1, 1, 0], [0, 1, 0],
                                              [2, 0, 0], [2, 1, 0], [1, 2, 0], [2, 2, 0]], np.float32),
                                   [0, 1, 2, 3, 1, 4, 5, 2, 2, 6, 7], [0, 4, 8, 11])
        geo.prim["piece"] = np.array([0.0, 0.005, 0.005], np.float32)
        geo.prim["other"] = np.array([0, 1, 1], np.int32)
        geo.prim["uv"] = np.zeros(3, np.int32)
        geo.vertex["uv"] = np.array([0] * 4 + [1] * 4 + [1] * 3, np.int32)
        geo = R.write(geo, "prim", "all", np.ones(geo.nprims, bool))
        base = {"from": "primitives", "to": "primitives", "name": "all", "new_name": "seam",
                "boundary": True, "unshared_edges": False, "connectivity_attribute": "piece",
                "keep_original": True}
        narrow = group_promote(geo, "", "points", [base])
        np.testing.assert_array_equal(R.membership(narrow, "prim", "seam"), [True, True, False])
        wide = group_promote(geo, "", "points", [dict(base, connectivity_tolerance=.01)])
        self.assertFalse(R.membership(wide, "prim", "seam").any())
        shared_point = group_promote(geo, "", "points", [dict(base,
            include_all_primitives_sharing_attribute_boundary_points=True)])
        np.testing.assert_array_equal(R.membership(shared_point, "prim", "seam"), [True, True, True])
        multi = group_promote(geo, "", "points", [dict(base, connectivity_attribute="piece other",
                                                      connectivity_tolerance=.01)])
        np.testing.assert_array_equal(R.membership(multi, "prim", "seam"), [True, True, False])
        vertex_first = group_promote(geo, "", "points", [dict(base, connectivity_attribute="uv")])
        np.testing.assert_array_equal(R.membership(vertex_first, "prim", "seam"), [True, True, False])

    def test_promote_open_curve_endpoints_and_all_curve_edges(self):
        from minihou.geometry import Geometry
        from minihou.nodes.group import group_promote
        geo = Geometry.from_arrays(np.array([[0, 0, 0], [1, 0, 0], [2, 0, 0]], np.float32),
                                   [0, 1, 2], [0, 3], closed=[False])
        geo = R.write(geo, "prim", "curve", np.array([True]))
        base = {"from": "primitives", "to": "points", "name": "curve", "new_name": "border",
                "boundary": True, "unshared_edges": True, "keep_original": True}
        ends = group_promote(geo, "", "points", [base])
        np.testing.assert_array_equal(R.membership(ends, "point", "border"), [True, False, True])
        all_points = group_promote(geo, "", "points", [dict(base, include_all_unshared_curve_edges=True)])
        np.testing.assert_array_equal(R.membership(all_points, "point", "border"), [True, True, True])
        edge_rule = dict(base, to="edges")
        no_edges = group_promote(geo, "", "points", [edge_rule])
        self.assertFalse(R.membership(no_edges, "edge", "border").any())
        all_edges = group_promote(geo, "", "points", [dict(edge_rule, include_all_unshared_curve_edges=True)])
        self.assertEqual(int(R.membership(all_edges, "edge", "border").sum()), 2)

    def test_promote_nonmanifold_attribute_seam(self):
        from minihou.geometry import Geometry
        from minihou.nodes.group import group_promote
        geo = Geometry.from_arrays(np.array([[0, 0, 0], [1, 0, 0], [0, 1, 0], [0, -1, 0], [0, 0, 1]], np.float32),
                                   [0, 1, 2, 1, 0, 3, 0, 1, 4], [0, 3, 6, 9])
        geo.prim["region"] = np.array([0, 1, 1], np.int32)
        geo = R.write(geo, "prim", "all", np.ones(3, bool))
        rule = {"from": "primitives", "to": "edges", "name": "all", "boundary": True,
                "unshared_edges": False, "connectivity_attribute": "region"}
        out = group_promote(geo, "", "points", [rule])
        from minihou import topo
        e = topo.edges(geo)
        selected = e.points[R.membership(out, "edge", "all")]
        np.testing.assert_array_equal(selected, [[0, 1]])
        partial = R.write(geo, "prim", "one", np.array([True, False, False]))
        region_only = group_promote(partial, "", "points", [{"from": "primitives", "to": "edges",
            "name": "one", "boundary": True, "unshared_edges": False}])
        np.testing.assert_array_equal(e.points[R.membership(region_only, "edge", "one")], [[0, 1]])

    def test_contained_edge_to_vertex_uses_outgoing_half_edge(self):
        from minihou.geometry import Geometry
        geo=Geometry.from_arrays(np.array([[0,0,0],[1,0,0],[1,1,0]],np.float32),[0,1,2],[0,3])
        from minihou import topo
        e=topo.edges(geo);mask=np.all(e.points==[0,1],axis=1)
        self.assertEqual(np.flatnonzero(R.convert(geo,mask,'edge','vertex',True)).tolist(),[0])
        self.assertEqual(np.flatnonzero(R.convert(geo,mask,'edge','vertex',False)).tolist(),[0,1])

    def test_create_normal_filter_normalizes_supplied_N(self):
        from minihou import REGISTRY
        geo=cook([]).copy()
        geo.point['N']=np.tile([0.,0.,.01],(geo.npoints,1)).astype(np.float32)
        geo.prim['N']=np.tile([10.,0.,10.],(geo.nprims,1)).astype(np.float32)
        params=REGISTRY.get('group_create').default_params()
        out=REGISTRY.get('group_create').fn(geo,**dict(params,name='up',method='normal',angle=10))
        self.assertTrue(R.membership(out,'point','up').all())
        out=REGISTRY.get('group_create').fn(geo,**dict(params,name='up',domain='primitives',method='normal',angle=10))
        self.assertFalse(R.membership(out,'prim','up').any())

    def test_create_primitive_type_filter_preserves_topology(self):
        from minihou import Geometry,REGISTRY
        geo=Geometry.from_arrays(np.array([[0,0,0],[1,0,0],[0,1,0]],np.float32),[0,1,2,0,1],[0,3,5],closed=[True,False])
        params=REGISTRY.get('group_create').default_params()
        for method in ('pattern','filters'):
            for kind,expected in [('polygons',[True,False]),('curves',[False,True])]:
                out=REGISTRY.get('group_create').fn(geo,**dict(params,domain='primitives',method=method,base_geometry_type=kind))
                np.testing.assert_array_equal(R.membership(out,'prim','group1'),expected)
                self.assertIs(out.topology,geo.topology)

    def test_create_edge_seed_attribute_and_nonmanifold_widest_angle(self):
        from minihou import Geometry,REGISTRY,topo
        from minihou.nodes.group import _edge_normals
        geo=Geometry.from_arrays(np.array([[0,0,0],[1,0,0],[0,0,1],[0,1,0],[0,-1,0]],np.float32),[0,1,2,0,1,3,0,1,4],[0,3,6,9])
        geo.point['id']=np.arange(geo.npoints,dtype=np.int32)
        params=REGISTRY.get('group_create').default_params()
        out=REGISTRY.get('group_create').fn(geo,**dict(params,domain='edges',method='filters',random_enable=True,seed_attrib='id'))
        self.assertEqual(R.membership(out,'edge','group1').shape,(len(topo.edges(geo).points),))
        e=topo.edges(geo);idx=int(np.flatnonzero(np.all(e.points==[0,1],axis=1))[0])
        self.assertAlmostEqual(_edge_normals(geo)[1][idx],180)

    def test_vertex_masks_convert_point_groups_and_empty_edge_masks(self):
        from minihou import Geometry,preview
        geo=Geometry.from_arrays(np.array([[0,0,0],[1,0,0],[1,1,0],[0,1,0]],np.float32),[0,1,2,3],[0,4])
        geo=R.write(geo,'point','p',np.array([True,False,False,False]))
        self.assertEqual(groups.member(geo,'vertex','p').shape,(4,))
        np.testing.assert_array_equal(groups.mask(geo,'vertex','p'),[True,False,False,False])
        head={'proxy':{'active':False}};arrays={}
        preview.mask_arrays(geo,{'domain':'vertex','name':'p'},head,arrays)
        np.testing.assert_array_equal(arrays['mask'],[1,0,0,0])
        blank=Geometry.points_only(np.empty((0,3),np.float32));blank=R.write(blank,'edge','empty',np.zeros(0,bool))
        preview.mask_arrays(blank,{'domain':'edge','name':'empty'},head,arrays)
        self.assertEqual(arrays['mask'].shape,(0,))

    def test_output_attribute_removes_preexisting_destination_group(self):
        from minihou.nodes.group import group_promote
        geo=cook([])
        geo=R.write(geo,'point','same',np.ones(geo.npoints,bool))
        geo=R.write(geo,'prim','same',np.ones(geo.nprims,bool))
        out=group_promote(geo,'','points',[{'from':'points','to':'primitives','name':'same','output_attribute':True,'keep_original':True}])
        self.assertIn('group_same',out.point)
        self.assertNotIn('group_same',out.prim)
        self.assertTrue(out.prim['same'].all())

    def test_auto_rule_snapshots_sources_and_multi_pattern_captures(self):
        from minihou.nodes.group import group_promote
        geo=cook([])
        geo=R.write(geo,'point','p_a',np.ones(geo.npoints,bool))
        geo=R.write(geo,'prim','f_b',np.ones(geo.nprims,bool))
        out=group_promote(geo,'','points',[{'from':'auto','to':'primitives','name':'*','new_name':'converted_*'}])
        self.assertEqual(R.names(out,'prim'),['converted_f_b','converted_p_a'])
        self.assertEqual(R.substituted_name('f_b','p_* f_*','new_*'),'new_b')


if __name__ == "__main__":
    unittest.main()
