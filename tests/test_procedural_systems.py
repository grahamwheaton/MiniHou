"""Behavioral tests for bounded architecture, region operations and geometric goals."""
import json
from pathlib import Path
import unittest

import numpy as np

from minihou import Evaluator, Geometry, Graph, topo
from minihou.sdk import REGISTRY


def run(name, *inputs, **params):
    nt = REGISTRY.get(name)
    result = nt.fn(*inputs, **nt.coerce_params(params))
    result.validate()
    return result


def quad():
    return Geometry.from_arrays(np.array([[0, 0, 0], [8, 0, 0], [8, 6, 0], [0, 6, 0]], np.float32), [0, 1, 2, 3], [0, 4])


def line(n=9):
    P = np.column_stack([np.linspace(-2, 2, n), np.zeros(n), np.zeros(n)]).astype(np.float32)
    return Geometry.from_arrays(P, np.arange(n), [0, n], closed=[False])


class BuildingRulesTests(unittest.TestCase):
    def test_storey_bay_counts_and_semantic_groups(self):
        g = run("building_rules", quad(), storeys=3, bay_width=2, balcony_probability=0)
        component = g.prim["component"]
        # Four + three + four + three bays on each floor.
        self.assertEqual(np.sum(component == 0), 42)
        self.assertEqual(np.sum(component == 1), 42)
        self.assertEqual(np.sum(component == 2), 0)
        self.assertEqual(np.sum(component == 3), 1)
        self.assertEqual(np.sum(component == 4), 1)
        self.assertAlmostEqual(float(g.P[:, 2].max()), 9)
        np.testing.assert_array_equal(g.prim["group_windows"], component == 1)

    def test_roofs_and_clockwise_footprint(self):
        q = quad()
        clockwise = Geometry.from_arrays(q.P, [3, 2, 1, 0], [0, 4])
        for roof, roof_faces in (("flat", 1), ("pyramid", 4), ("gable", 4)):
            for source in (q, clockwise):
                g = run("building_rules", source, roof=roof, roof_height=2)
                self.assertEqual(np.sum(g.prim["component"] == 3), roof_faces)
                self.assertAlmostEqual(float(g.P[:, 2].max()), 9 if roof == "flat" else 11)

    def test_rule_override_and_seed_replay(self):
        params = dict(storeys=4, seed=19, rules='[{"floor":"ground","window_width":0},{"floor":"top","balcony_probability":1}]')
        a = run("building_rules", quad(), **params)
        b = run("building_rules", quad(), **params)
        np.testing.assert_array_equal(a.P, b.P)
        self.assertFalse(np.any((a.prim["component"] == 1) & (a.prim["floor"] == 0)))
        self.assertTrue(np.any((a.prim["component"] == 2) & (a.prim["floor"] == 3)))
        other = run("building_rules", quad(), **{**params, "seed":20})
        self.assertFalse(a.P.shape == other.P.shape and np.array_equal(a.P, other.P))

    def test_invalid_footprints_and_rules(self):
        shapes = [
            [[0,0,0],[2,0,0],[1,1,0],[2,2,0],[0,2,0]],
            [[0,0,0],[2,2,0],[0,2,0],[2,0,0]],
            [[0,0,0],[2,0,0],[2,2,1],[0,2,0]],
        ]
        for shape in shapes:
            g = Geometry.from_arrays(np.asarray(shape, np.float32), np.arange(len(shape)), [0,len(shape)])
            with self.assertRaises(ValueError): run("building_rules", g)
        for rules in ('{}', '[{"unknown":2}]', '[{"window_width":2}]', '[{"floor":99}]'):
            with self.assertRaises(ValueError): run("building_rules", quad(), rules=rules)
        tri = Geometry.from_arrays(quad().P[:3], [0,1,2], [0,3])
        with self.assertRaisesRegex(ValueError, "rectangular"): run("building_rules", tri, roof="gable")

    def test_empty_and_multiple_footprints(self):
        empty = run("building_rules", Geometry.from_arrays(np.empty((0,3),np.float32)))
        self.assertEqual(empty.npoints, 0)
        merged = run("merge", [quad(), run("transform", quad(), translate=[20,0,0])])
        g = run("building_rules", merged)
        self.assertEqual(set(g.prim["building_id"]), {0,1})


class RegionTests(unittest.TestCase):
    def test_connected_patch_extrudes_only_boundary(self):
        grid = run("grid", columns=3, rows=2, size_x=2, size_y=1)
        marked = run("region_identify", grid)
        self.assertEqual(len(np.unique(marked.prim["region_id"])), 1)
        result = run("region_transform", marked, extrude=.2)
        # Two original backs + two fronts + six outer boundary edges. No internal wall.
        self.assertEqual(result.nprims, 10)
        self.assertAlmostEqual(float(np.ptp(result.P[:,2])), .2, places=6)

    def test_primitive_regions_split_shared_points_and_preserve_attributes(self):
        grid = run("grid", columns=3, rows=2, size_x=2, size_y=1)
        grid.prim["material_id"] = np.array([7,8],np.int32)
        grid.point["weight"] = np.arange(grid.npoints,dtype=np.float32)
        grid.vertex["uv_test"] = np.arange(grid.nvertices,dtype=np.float32)
        marked = run("region_identify", grid, mode="primitive")
        result = run("region_transform", marked, scale=[.8,.8,1])
        self.assertEqual(result.npoints, 8)
        self.assertEqual(set(result.prim["material_id"]), {7,8})
        self.assertIn("weight", result.point)
        np.testing.assert_array_equal(result.vertex["uv_test"], grid.vertex["uv_test"])
        pieces = topo.edges(result).points
        self.assertEqual(len(pieces),8)

    def test_seed_stability_and_attribute_labels(self):
        g = run("grid", columns=4, rows=3)
        g.prim["class"] = np.array([10,20,10,20,10,20],np.int32)
        identified = run("region_identify", g, mode="attribute")
        params = dict(random_rotate=[0,0,30],random_translate=[.1,.2,.3],random_scale=.1,seed=42)
        a = run("region_transform", identified, **params)
        b = run("region_transform", identified, **params)
        np.testing.assert_array_equal(a.P,b.P)
        c = run("region_transform", identified, **{**params,"seed":43})
        self.assertFalse(np.array_equal(a.P,c.P))
        self.assertEqual(set(a.prim["region_id"]),{10,20})

    def test_frozen_input_untouched_and_normals(self):
        source = run("normal", quad(), domain="point")
        before = source.P.copy()
        source.freeze()
        g = run("region_transform",source,rotate=[90,0,0],scale=[2,1,1])
        np.testing.assert_array_equal(source.P,before)
        np.testing.assert_allclose(g.point["N"],np.tile([0,-1,0],(4,1)),atol=1e-6)
        with self.assertRaisesRegex(ValueError,"polygon"): run("region_transform",line())

    def test_extrusion_removes_stale_corner_normals(self):
        g = quad()
        g.vertex["N"] = np.tile([0,0,1],(4,1)).astype(np.float32)
        out = run("region_transform",g,extrude=.2)
        self.assertNotIn("N",out.vertex)

    def test_region_label_survives_reordering(self):
        g = run("grid",columns=3,rows=2)
        g.prim["class"] = np.array([10,100],np.int32)
        a = run("region_identify",g,mode="attribute",seed=13)
        b = run("region_identify",run("sort",g,domain="primitives",key="reverse"),mode="attribute",seed=13)
        for key in (10,100):
            np.testing.assert_array_equal(a.prim["Cd"][a.prim["region_id"]==key],b.prim["Cd"][b.prim["region_id"]==key])


class GeometricGoalTests(unittest.TestCase):
    def test_length_converges_to_scaled_rest_length(self):
        source = line(2)
        g = run("goal_pin", run("goal_length", source, scale=.5),group="0")
        result = run("goal_solve",g,steps=20,iterations=10)
        self.assertAlmostEqual(float(np.linalg.norm(result.P[1]-result.P[0])),2,places=5)
        self.assertTrue(result.detail["goal_report"]["converged"])

    def test_cable_sags_pins_hold_replay_and_topology_preserved(self):
        g = run("goal_length",line(),scale=1.1)
        g = run("goal_gravity",g)
        g = run("goal_pin",g,group="0 8")
        g.point["Cd"] = np.ones((9,3),np.float32)
        original = g.P.copy(); g.freeze()
        a = run("goal_solve",g,steps=200,iterations=30)
        b = run("goal_solve",g,steps=200,iterations=30)
        np.testing.assert_array_equal(a.P,b.P)
        np.testing.assert_array_equal(g.P,original)
        np.testing.assert_array_equal(a.P[[0,8]],original[[0,8]])
        self.assertLess(a.P[4,2],-.2)
        self.assertEqual(a.topology.digest,g.topology.digest)
        self.assertIn("Cd",a.point)
        self.assertLess(a.detail["goal_report"]["residuals"]["length"],.015)

    def test_collision_separates_coincident_points(self):
        g = Geometry.from_arrays(np.zeros((2,3),np.float32))
        g = run("goal_collision",g,radius=.5)
        a = run("goal_solve",g,steps=20,iterations=10)
        self.assertGreaterEqual(np.linalg.norm(a.P[0]-a.P[1]),.99999)
        self.assertLess(a.detail["goal_report"]["residuals"]["collision"],1e-6)
        np.testing.assert_allclose(a.P.mean(0),0,atol=1e-7)

    def test_surface_projects_to_triangle_interior_and_boundary(self):
        p = Geometry.from_arrays(np.array([[1,1,3],[20,2,4]],np.float32))
        g = run("goal_surface",p,quad())
        a = run("goal_solve",g,steps=10,damping=1)
        np.testing.assert_allclose(a.P,[[1,1,0],[8,2,0]],atol=1e-6)
        self.assertLess(a.detail["goal_report"]["residuals"]["surface"],1e-6)

    def test_conflicting_goals_report_nonconvergence(self):
        p = Geometry.from_arrays(np.array([[1,1,3]],np.float32))
        g = run("goal_pin", run("goal_surface",p,quad()))
        a = run("goal_solve",g,steps=5)
        np.testing.assert_array_equal(a.P,p.P)
        self.assertFalse(a.detail["goal_report"]["converged"])
        self.assertAlmostEqual(a.detail["goal_report"]["residuals"]["surface"],3)

    def test_topology_change_rejected(self):
        g = run("goal_length",quad())
        changed = run("polyextrude",g)
        with self.assertRaisesRegex(ValueError,"topology|compatible"): run("goal_solve",changed)
        with self.assertRaisesRegex(ValueError,"topology"): run("goal_gravity",changed)

    def test_gravity_alone_is_not_misreported_as_static(self):
        g = run("goal_gravity",line(2))
        a = run("goal_solve",g,steps=10)
        self.assertFalse(a.detail["goal_report"]["converged"])
        self.assertLess(a.P[0,2],0)

    def test_work_bounds_and_degenerate_surface(self):
        big = Geometry.from_arrays(np.zeros((1001,3),np.float32))
        with self.assertRaisesRegex(ValueError,"1000"): run("goal_collision",big)
        g = run("goal_collision",Geometry.from_arrays(np.zeros((200,3),np.float32)))
        with self.assertRaisesRegex(ValueError,"budget"): run("goal_solve",g,steps=1000,iterations=100)
        target = Geometry.from_arrays(np.zeros((3,3),np.float32),[0,1,2],[0,3])
        with self.assertRaisesRegex(ValueError,"nondegenerate"): run("goal_surface",line(),target)


class ProceduralExampleTests(unittest.TestCase):
    def test_examples_cook_deterministically(self):
        for path in sorted((Path(__file__).resolve().parents[1]/"examples").glob("procedural_*.json")):
            with self.subTest(example=path.name):
                graph = Graph.from_dict(json.loads(path.read_text(encoding="utf-8")))
                a = Evaluator().cook(graph); a.validate()
                b = Evaluator().cook(graph); b.validate()
                np.testing.assert_array_equal(a.P,b.P)
                self.assertGreater(a.npoints,0)

    def test_fabric_and_packing_measured_behavior(self):
        root = Path(__file__).resolve().parents[1]/"examples"
        fabric = Graph.from_dict(json.loads((root/"procedural_fabric.json").read_text(encoding="utf-8")))
        f = Evaluator().cook(fabric,"relax")
        np.testing.assert_array_equal(f.P[[0,8,72,80],2],3)
        self.assertLess(float(f.P[40,2]),2.8)
        self.assertLess(f.detail["goal_max_residual"],.02)
        packing = Graph.from_dict(json.loads((root/"procedural_packing.json").read_text(encoding="utf-8")))
        p = Evaluator().cook(packing,"relax")
        delta = p.P[:,None]-p.P[None,:]
        distances = np.linalg.norm(delta,axis=-1)
        np.fill_diagonal(distances,np.inf)
        self.assertGreater(float(distances.min()),.5999)
        np.testing.assert_allclose(p.P[:,2],0,atol=1e-6)
        self.assertLessEqual(float(np.max(np.abs(p.P[:,:2]))),2.00001)
        self.assertTrue(p.detail["goal_converged"])

    def test_nonfinite_parameters_rejected_by_sdk(self):
        for name,params in (("building_rules",{"window_width":float("nan")}),
                            ("goal_gravity",{"acceleration":[0,0,float("inf")]}),
                            ("region_transform",{"scale":[1,float("nan"),1]}),
                            ("goal_solve",{"timestep":float("inf")})):
            with self.subTest(node=name), self.assertRaisesRegex(ValueError,"finite"):
                REGISTRY.get(name).coerce_params(params)

    def test_nonfinite_geometry_rejected(self):
        g = quad(); p = g.P.copy(); p[0,0] = np.nan; g.point["P"] = p
        for name in ("building_rules","region_identify","region_transform","goal_length"):
            with self.subTest(node=name), self.assertRaisesRegex(ValueError,"finite"):
                run(name,g)


if __name__ == "__main__":
    unittest.main()
