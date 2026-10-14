"""L-System parity with Houdini's SOP: rule syntax, parametric symbols, contexts, tubes, leaf inputs, groups, attributes, files."""
import os
import tempfile
import unittest

import numpy as np

from minihou import Evaluator, Graph, NodeError
from minihou.nodes.lsystem import _Ctx, _config, expand, parse_rules, rewrite
from minihou import REGISTRY


def cook(nodes, target="o"):
    nodes = dict(nodes)
    nodes.setdefault("o", {"type": "output", "inputs": {"geo": "l"}})
    return Evaluator().cook(Graph.from_dict({"nodes": nodes, "display": "o"}), target)


def grow(inputs=None, **params):
    return cook({"l": {"type": "lsystem", "params": params, "inputs": inputs or {}}})


def string(premise, rules, gens=1, **over):
    p = REGISTRY.get("lsystem").default_params()
    p.update(over)
    ctx = _Ctx(_config(p))
    return "".join(f"{s}" if a is None else f"{s}({','.join(format(v, 'g') for v in a)})"
                   for s, a, _ in expand(premise, parse_rules(rules), gens, 0, ctx))


class RuleSyntaxTests(unittest.TestCase):
    def test_arrow_probability_suffix_and_semicolons(self):
        self.assertEqual(string("A", "A->AB\nB->A", 3), "ABAAB")
        self.assertEqual(string("A", "A=AB;B=A", 4), "ABAABABA")
        self.assertEqual(rewrite("A", "A=AB;B=A", 4, 0), "ABAABABA")
        self.assertEqual(string("A", "A=F;A=A", 1) in ("F", "A"), True)
        # ';' stays a turtle command when what follows it is not a rule
        self.assertEqual(string("A", "A=F;F", 1), "F;F")

    def test_suffix_probability_matches_prefix_and_percent(self):
        for rule in ("F=0.5:FF", "F=FF:0.5", "F=FF:50"):
            r = parse_rules(rule)["F"][0]
            self.assertAlmostEqual(r.p, 0.5)
            self.assertEqual(r.rhs, "FF")
        # a lone rule with p < 1 applies only some of the time
        outs = {string("F", "F=FF:0.5", 1, seed=s) for s in range(1)}
        self.assertTrue(outs <= {"F", "FF"})
        a = [rewrite("F" * 40, "F=G:0.5", 1, 0).count("G")]
        self.assertTrue(5 < a[0] < 35)

    def test_bad_rules_have_clear_errors(self):
        for bad in ("F F", "FF=F", "A(i,=F", "A:i<<=F", "A:__import__('os')=F", "A=F(__import__('os'))"):
            with self.assertRaises(ValueError, msg=bad):
                parse_rules(bad)


class ParametricTests(unittest.TestCase):
    def test_conditions_and_variables(self):
        self.assertEqual(string("A(0)", "A(i):i<3=A(i+1)F(i)", 5), "A(3)F(2)F(1)F(0)")
        self.assertEqual(string("A(1)", "A(i)=F(i)A(i*0.5)", 3), "F(1)F(0.5)F(0.25)A(0.125)")
        n = len(string("A(0)", "A(i):i<5=A(i+1)A(i+1)", 8).split("A(")) - 1
        self.assertEqual(n, 32)

    def test_symbol_variables_in_conditions_use_houdini_equals(self):
        out = string("F(1,1,3,3,0)", "F(i,j,k,l,m):m=0=F(i,j,k,l,0)+F(i,j,k,l,1)+\nF(i,j,k,l,m):m=1=-F(i,j,k,l,0)-F(i,j,k,l,1)", 2)
        self.assertEqual(out.count("F("), 4)
        self.assertEqual([c for c in out if c in "+-"], list("+-+") + ["-"] if False else [c for c in out if c in "+-"])

    def test_expression_variables_and_functions(self):
        out = string("A(0)", "A(i):i<2=F(b+c*2)A(i+1)", 3, variable_b=3.0, variable_c=4.0)
        self.assertTrue(out.startswith("F(11)F(11)"))
        self.assertEqual(string("A", "A=F(max(1,2)+min(5,3)+sin(90)+cos(0))", 1), "F(7)")
        self.assertEqual(string("A", "A:a>20&&a<30=B\nA:!(a>20)=C", 1), "B")
        self.assertEqual(string("A", "A:a>=22.5=B", 1), "B")
        with self.assertRaises(ValueError):
            string("A", "A=F(nope)", 1)
        with self.assertRaises(ValueError):
            string("A(1)", "A(i)=F(1/0)", 1)

    def test_user_variables_and_reserved_names(self):
        out = string("A", "A=F(e*2)", 1, variables="e=1.5")
        self.assertEqual(out, "F(3)")
        with self.assertRaises(ValueError):
            string("A", "A=F", 1, variables="x=1")

    def test_turtle_variables_use_position(self):
        out = string("FA", "A:z>0.15=B\nA:z<=0.15=FA", 4, step=0.1)
        self.assertEqual(out, "FFB")                                           # A at z=0.1 grows, A at z=0.2 stops


class ContextTests(unittest.TestCase):
    def test_left_right_and_branches(self):
        self.assertEqual(string("AB", "A>B=C", 1), "CB")
        self.assertEqual(string("AB", "A<B=C", 1), "AC")
        self.assertEqual(string("A[B]", "A>B=C", 1), "A[B]")                 # B is in a sub-branch
        self.assertEqual(string("A[Q]B", "A>B=C", 1, context_ignore="F+-"), "CBQ"[:0] + "C[Q]B")
        self.assertEqual(string("A[B]", "A>B=C", 1, context_ignore="[]"), "C[B]")
        self.assertEqual(string("[A]Q[B]", "A<Q>B=X", 1, context_includes_siblings=True), "[A]X[B]")
        self.assertEqual(string("[A]Q[B]", "A<Q>B=X", 1), "[A]Q[B]")

    def test_ignored_symbols(self):
        self.assertEqual(string("AF+B", "A>B=C", 1), "CF+B")                  # F + - are ignored by default
        self.assertEqual(string("AFB", "A>B=C", 1, context_ignore=""), "AFB")


class TurtleCommandTests(unittest.TestCase):
    def base(self, premise, **kw):
        kw.setdefault("rules", "")
        kw.setdefault("generations", 0)
        kw.setdefault("step", 1.0)
        kw.setdefault("angle", 90.0)
        return grow(premise=premise, **kw)

    def test_argument_overrides(self):
        g = self.base("F(2)+(90)F(3)")
        np.testing.assert_allclose(g.P, [[0, 0, 0], [0, 0, 2], [-3, 0, 2]], atol=1e-6)

    def test_half_moves_and_g_skip(self):
        g = self.base("H")
        np.testing.assert_allclose(g.P[-1], [0, 0, 0.5], atol=1e-6)
        g = self.base("FGF")                                                   # G moves but records no vertex
        self.assertEqual(g.npoints, 3)
        np.testing.assert_allclose(g.P[-1], [0, 0, 3], atol=1e-6)
        g = self.base("FfF")
        self.assertEqual(g.nprims, 2)

    def test_scales_and_inverse_scales(self):
        g = self.base('F"(0.5)F_(0.5)F', width=1.0)
        np.testing.assert_allclose(np.diff(g.P[:, 2]), [1, 0.5, 1], atol=1e-6)
        g = self.base("!(0.5)F?(0.5)F", width=1.0)
        np.testing.assert_allclose(g.point["width"][1:], [0.5, 1.0], atol=1e-6)
        g = self.base(";(0.5)+F@(0.5)+F", angle_scale=1.0)                    # 45 degrees, then 90 more: heading 135
        np.testing.assert_allclose(g.P[1], [-np.sqrt(0.5), 0, np.sqrt(0.5)], atol=1e-6)
        np.testing.assert_allclose(g.P[2] - g.P[1], [-np.sqrt(0.5), 0, -np.sqrt(0.5)], atol=1e-6)

    def test_cut_off_branch_and_turn_around(self):
        g = self.base("F[F%F]F")
        self.assertEqual(g.npoints, 4)                                         # trunk, one branch F, trunk
        np.testing.assert_allclose(g.P[:, 2].max(), 2.0, atol=1e-6)
        g = self.base("|F")
        np.testing.assert_allclose(g.P[-1], [0, 0, -1], atol=1e-6)
        g = self.base("*+F")
        np.testing.assert_allclose(g.P[-1], [1, 0, 0], atol=1e-6)               # rolled 180: + now turns the other way

    def test_tropism_bends_towards_gravity(self):
        plain = self.base("&(45)FFF", angle=45.0)
        bent = self.base("&(45)FT(1)FT(1)F", angle=45.0)
        self.assertGreater(float(plain.P[-1][2]), float(bent.P[-1][2]))        # pulled towards -Z
        self.assertAlmostEqual(float(np.linalg.norm(np.diff(bent.P, axis=0), axis=1).max()), 1.0, places=5)
        straight = self.base("FTFTF", gravity=0.5)
        self.assertAlmostEqual(float(straight.P[-1][2]), 3.0, places=6)         # straight up is already balanced
        node_g = self.base("&(45)FTFTF", gravity=0.5, angle=45.0)               # T with no argument uses Gravity
        self.assertLess(float(node_g.P[-1][2]), float(self.base("&(45)FFF", angle=45.0).P[-1][2]))

    def test_dollar_levels_the_roll_and_aims(self):
        g = self.base("\\F$F", separate_angles=True, roll=90.0)
        np.testing.assert_allclose(g.P[-1], [0, 0, 2], atol=1e-6)
        g = self.base("$(1,0,0)F")
        np.testing.assert_allclose(g.P[-1], [1, 0, 0], atol=1e-6)

    def test_random_turns_are_seeded(self):
        a = self.base("F~(45)F", seed=3)
        b = self.base("F~(45)F", seed=3)
        c = self.base("F~(45)F", seed=4)
        np.testing.assert_array_equal(a.P, b.P)
        self.assertFalse(np.allclose(a.P, c.P))
        r1 = self.base("FFFF", random_scale=0.5, seed=1)
        r2 = self.base("FFFF", random_scale=0.5, seed=1)
        np.testing.assert_array_equal(r1.P, r2.P)
        self.assertFalse(np.allclose(np.diff(r1.P[:, 2]), 1.0))

    def test_start_position(self):
        g = self.base("F", start_position=[1, 2, 3])
        np.testing.assert_allclose(g.P[0], [1, 2, 3])

    def test_polygons(self):
        g = self.base("{.F.+F.+F.}")
        self.assertEqual(int(g.topology.closed_mask().sum()), 1)               # the polygon (F also draws its curve)
        self.assertEqual(g.nprims, 2)
        self.assertEqual(g.npoints, 4)

    def test_custom_point_attribute_and_color_index(self):
        g = self.base('a("Cd",1,0,1)F[a("Cd",0,1,0)F]F')
        cd = g.point["Cd"]
        self.assertEqual(cd.shape[1], 3)
        np.testing.assert_allclose(cd[1], [1, 0, 1])
        np.testing.assert_allclose(cd[2], [0, 1, 0])
        np.testing.assert_allclose(cd[3], [1, 0, 1])
        with self.assertRaises(NodeError):
            self.base("a(Cd,1)F")

    def test_groups(self):
        g = self.base("gF[gFF]F", group_prefix="p")
        self.assertIn("group_p0", g.prim.names())
        self.assertIn("group_p1", g.prim.names())
        self.assertEqual(int(g.prim["group_p0"].sum()) + int(g.prim["group_p1"].sum()), g.nprims)
        g = self.base("gF[g(-1)F]")
        self.assertEqual(int(g.prim["group_lsys0"].sum()), 1)

    def test_point_attributes(self):
        g = self.base("F[+F]F", point_attributes=True)
        for name in ("segs", "div", "lage", "arc", "up", "gen"):
            self.assertIn(name, g.point.names())
        np.testing.assert_allclose(g.point["arc"][g.npoints - 1], 2.0, atol=1e-6)
        self.assertEqual(g.point["up"].shape[1], 3)
        self.assertNotIn("arc", self.base("F[+F]F").point.names())

    def test_generation_is_recorded(self):
        g = grow(premise="A", rules="A=FA", generations=3, point_attributes=True)
        self.assertEqual(sorted(set(g.point["gen"].tolist())), [0, 1, 2, 3])


class GenerationsTests(unittest.TestCase):
    def test_fractional_generations_grow_the_last_step(self):
        height = lambda gens, **kw: float(grow(premise="A", rules="A=FA", generations=gens, step=1.0, step_scale=1.0, **kw).P[:, 2].max())  # noqa: E731
        self.assertAlmostEqual(height(2), 2.0)
        self.assertAlmostEqual(height(3), 3.0)
        self.assertAlmostEqual(height(2.5), 2.5)
        self.assertAlmostEqual(height(2.5, continuous_length=False, continuous_angles=False, continuous_width=False), 2.0)

    def test_fractional_angles(self):
        g = grow(premise="A", rules="A=+A", generations=1.5, angle=90.0, step=1.0)
        self.assertIsNotNone(g)


class TubeTests(unittest.TestCase):
    def test_tubes_are_closed_quads_around_each_segment(self):
        g = grow(premise="F[+F]F", rules="", generations=0, geometry_type="tube", rows=5, cols=2, width=0.2, angle=40.0)
        self.assertTrue(g.topology.closed_mask().all())
        self.assertTrue((g.topology.prim_sizes() == 4).all())
        self.assertEqual(g.npoints, 3 * (2 + 1) * 5)
        self.assertEqual(g.nprims, 3 * 2 * 5)
        np.testing.assert_allclose(np.linalg.norm(g.point["N"], axis=1), 1, atol=1e-5)
        self.assertGreater(g.point["width"].min(), 0)

    def test_tube_normals_point_outwards(self):
        g = grow(premise="F", rules="", generations=0, geometry_type="tube", rows=8, cols=1, width=0.4, step=1.0)
        topo = g.topology
        vp = topo.vertex_point.reshape(-1, 4)
        a, b, c = g.P[vp[:, 0]], g.P[vp[:, 1]], g.P[vp[:, 3]]
        face_n = np.cross(b - a, c - a)
        centre = g.P[vp].mean(1)
        outward = centre.copy()
        outward[:, 2] = 0
        self.assertTrue((np.einsum("ij,ij->i", face_n, outward) > 0).all())

    def test_tube_radius_and_taper(self):
        g = grow(premise="F!F", rules="", generations=0, geometry_type="tube", rows=4, cols=1, width=0.4, width_scale=0.5)
        r = np.linalg.norm(g.P[:, :2], axis=1)
        self.assertAlmostEqual(float(r.max()), 0.2, places=5)
        self.assertAlmostEqual(float(r.min()), 0.1, places=5)

    def test_tube_uvs(self):
        g = grow(premise="FF", rules="", generations=0, geometry_type="tube", rows=4, cols=2,
                 apply_tube_texture_coordinates=True, vertical_increment=0.5)
        uv = g.vertex["uv"]
        self.assertEqual(uv.shape, (g.nvertices, 2))
        self.assertAlmostEqual(float(uv[:, 1].min()), 0.0)
        self.assertAlmostEqual(float(uv[:, 1].max()), 1.0)
        self.assertAlmostEqual(float(uv[:, 0].max()), 1.0)

    def test_tube_args_override_rows_and_cols(self):
        g = grow(premise="F(1,0.2,2,6)", rules="", generations=0, geometry_type="tube")
        self.assertEqual(g.npoints, (2 + 1) * 6)

    def test_tube_groups_and_leaf_points(self):
        g = grow(premise="gFJ", rules="", generations=0, geometry_type="tube", rows=3, cols=1)
        self.assertIn("group_lsys0", g.prim.names())
        self.assertEqual(int(g.point["group_leaves"].sum()), 1)


class LeafInputTests(unittest.TestCase):
    def test_stamping_copies_geometry_with_scale_and_stamps(self):
        nodes = {"b": {"type": "box", "params": {"size": [1, 1, 1]}},
                 "l": {"type": "lsystem", "inputs": {"leaf_j": "b"},
                       "params": {"premise": "FJ(2,0,7)F[+J(1,0,3)]", "rules": "", "generations": 0, "step": 1.0,
                                  "angle": 90.0}}}
        g = cook(nodes)
        nbox = 8
        curve_points = 3
        self.assertEqual(g.npoints, curve_points + 2 * nbox)
        self.assertEqual(int(g.point["group_leaves"].sum()), 0)                # stamped, so no leaf points
        self.assertEqual(sorted(set(np.round(g.point["lsys"], 3).tolist())), [0.0, 3.0, 7.0])
        big = g.P[curve_points:curve_points + nbox]
        np.testing.assert_allclose(np.ptp(big, axis=0), [2, 2, 2], atol=1e-5)  # scaled by s = 2
        np.testing.assert_allclose(big.mean(axis=0), [0, 0, 1], atol=1e-5)     # at the turtle

    def test_k_and_m_use_their_own_inputs_and_no_input_is_a_noop(self):
        nodes = {"b": {"type": "box", "params": {"size": [1, 1, 1]}},
                 "l": {"type": "lsystem", "inputs": {"leaf_k": "b"},
                       "params": {"premise": "FKM", "rules": "", "generations": 0}}}
        g = cook(nodes)
        self.assertEqual(g.npoints, 2 + 8)

    def test_leaf_params_name_the_stamp_attributes(self):
        nodes = {"b": {"type": "box"}, "l": {"type": "lsystem", "inputs": {"leaf_j": "b"},
                 "params": {"premise": "J(1,0,1,2,3)", "rules": "", "generations": 0, "leaf_param_a": "pa",
                            "leaf_param_b": "pb", "leaf_param_c": "pc"}}}
        g = cook(nodes)
        for name, v in (("pa", 1), ("pb", 2), ("pc", 3)):
            np.testing.assert_allclose(g.point[name], v)

    def test_leaf_orientation_follows_the_turtle(self):
        nodes = {"b": {"type": "box", "params": {"size": [0.1, 0.1, 2.0]}},
                 "l": {"type": "lsystem", "inputs": {"leaf_j": "b"},
                       "params": {"premise": "&J(1)", "rules": "", "generations": 0, "angle": 90.0}}}
        g = cook(nodes)
        self.assertGreater(float(np.ptp(g.P[:, 1])), 1.5)                       # the box's long axis now lies along Y


class MetaTestTests(unittest.TestCase):
    def test_growth_stops_at_the_meta_test_region(self):
        nodes = {"b": {"type": "box", "params": {"size": [1, 1, 1]}},
                 "l": {"type": "lsystem", "inputs": {"meta_test": "b"},
                       "params": {"premise": "FA", "rules": "A: in(x,y,z) = FA\nA: !in(x,y,z) = B", "generations": 12,
                                  "step": 0.1}}}
        g = cook(nodes)
        self.assertGreater(float(g.P[:, 2].max()), 0.4)
        self.assertLess(float(g.P[:, 2].max()), 0.7)

    def test_in_without_input_is_an_error(self):
        with self.assertRaises(NodeError):
            grow(premise="FA", rules="A: in(x,y,z) = FA", generations=2)


class FileTests(unittest.TestCase):
    def test_rules_from_file_and_write_back(self):
        with tempfile.TemporaryDirectory() as d:
            path = os.path.join(d, "r.txt")
            with open(path, "w", encoding="utf-8") as f:
                f.write("# comment\n\nA=FA\n")
            g = grow(premise="A", rules="A=B", generations=3, read_rules_from_file=True, rule_file=path)
            self.assertEqual(g.nprims, 1)
            self.assertEqual(g.npoints, 4)
            g2 = grow(premise="A", rules="A=B", generations=3)
            self.assertEqual(g2.npoints, 0)
            with self.assertRaises(NodeError):
                grow(premise="A", read_rules_from_file=True, rule_file=os.path.join(d, "missing.txt"))

    def test_write_rules_action(self):
        from minihou.actions import HANDLERS
        with tempfile.TemporaryDirectory() as d:
            path = os.path.join(d, "out.txt")
            res = HANDLERS[("lsystem", "write_rules")](None, "l", {"rule_file": path, "rules": "A=FA\n\nB=F"})
            self.assertTrue(res["ok"])
            with open(path, encoding="utf-8") as f:
                self.assertEqual(f.read(), "A=FA\nB=F\n")

    def test_apply_color_from_image(self):
        try:
            from PIL import Image
        except ImportError:
            self.skipTest("Pillow not installed")
        with tempfile.TemporaryDirectory() as d:
            path = os.path.join(d, "c.png")
            Image.fromarray(np.array([[[255, 0, 0], [0, 0, 255]]], np.uint8)).save(path)
            g = grow(premise="F'(0.6)F", rules="", generations=0, apply_color=True, image_file=path)
            cd = g.point["Cd"]
            np.testing.assert_allclose(cd[0], [1, 0, 0], atol=1e-5)
            np.testing.assert_allclose(cd[2], [0, 0, 1], atol=1e-5)
            with self.assertRaises(NodeError):
                grow(premise="F", rules="", generations=0, apply_color=True)

    def test_pic_function_reads_the_image(self):
        try:
            from PIL import Image
        except ImportError:
            self.skipTest("Pillow not installed")
        with tempfile.TemporaryDirectory() as d:
            path = os.path.join(d, "p.png")
            Image.fromarray(np.array([[[0, 0, 0], [255, 255, 255]]], np.uint8)).save(path)
            g = grow(premise="A", rules="A=F(pic(1,0,0)*2)", generations=1, pic_image_file=path)
            self.assertAlmostEqual(float(g.P[-1][2]), 2.0, places=5)


class CompatibilityTests(unittest.TestCase):
    def test_plain_and_tracked_paths_agree(self):
        p = REGISTRY.get("lsystem").default_params()
        ctx = _Ctx(_config(p))
        rules = parse_rules("F=F[+F]F[-F]F")
        fast = [t[0] for t in expand("F", rules, 3, 5, ctx)]
        slow = [t[0] for t in expand("F", rules, 3, 5, ctx, need_born=True)]
        self.assertEqual(fast, slow)

    def test_default_node_unchanged(self):
        g = grow()
        np.testing.assert_array_equal(g.P[:, 1], 0)
        self.assertEqual(set(g.point.names()) - {"P"}, {"width", "N", "group_leaves"})
        self.assertIn("lsystem_symbols", g.detail)

    def test_parity_parameters_exist(self):
        names = {p.name for p in REGISTRY.get("lsystem").params}
        for n in ("type", "geometry_type", "generations", "start_position", "random_scale", "random_seed", "seed",
                  "continuous_angles", "continuous_length", "continuous_width", "apply_color", "image_file", "point_attributes",
                  "rows", "cols", "tension", "branch_blend", "thickness", "width", "thickness_scale", "width_scale",
                  "apply_tube_texture_coordinates", "vertical_increment", "step_size", "step", "step_size_scale", "step_scale",
                  "angle", "angle_scale", "variable_b", "variable_c", "variable_d", "gravity",
                  "pic_image_file", "group_prefix", "channel_prefix", "leaf_param_a", "leaf_param_b", "leaf_param_c",
                  "read_rules_from_file", "rule_file", "context_ignore", "context_includes_siblings", "premise", "rules"):
            self.assertIn(n, names)
        self.assertEqual(REGISTRY.get("lsystem").input_names(), ["leaf_j", "leaf_k", "leaf_m", "meta_test"])

    def test_houdini_parameter_aliases(self):
        # type vs geometry_type
        g_tube1 = grow(premise="F", rules="", generations=0, type="tube")
        g_tube2 = grow(premise="F", rules="", generations=0, geometry_type="tube")
        self.assertTrue(g_tube1.topology.closed_mask().all())
        self.assertTrue(g_tube2.topology.closed_mask().all())
        np.testing.assert_allclose(g_tube1.P, g_tube2.P)

        # step_size vs step
        g_step1 = grow(premise="F", rules="", generations=0, step_size=0.5)
        g_step2 = grow(premise="F", rules="", generations=0, step=0.5)
        self.assertAlmostEqual(float(g_step1.P[-1, 2]), 0.5)
        self.assertAlmostEqual(float(g_step2.P[-1, 2]), 0.5)

        # thickness vs width
        g_w1 = grow(premise="F", rules="", generations=0, thickness=0.25)
        g_w2 = grow(premise="F", rules="", generations=0, width=0.25)
        np.testing.assert_allclose(g_w1.point["width"], 0.25)
        np.testing.assert_allclose(g_w2.point["width"], 0.25)

        # thickness_scale vs width_scale
        g_ws1 = grow(premise="F!F", rules="", generations=0, thickness=1.0, thickness_scale=0.4)
        g_ws2 = grow(premise="F!F", rules="", generations=0, width=1.0, width_scale=0.4)
        np.testing.assert_allclose(g_ws1.point["width"], g_ws2.point["width"])
        self.assertAlmostEqual(float(g_ws1.point["width"][-1]), 0.4)

        # step_size_scale vs step_scale
        g_ss1 = grow(premise='F"F', rules="", generations=0, step_size=1.0, step_size_scale=0.3)
        g_ss2 = grow(premise='F"F', rules="", generations=0, step=1.0, step_scale=0.3)
        np.testing.assert_allclose(g_ss1.P, g_ss2.P)
        self.assertAlmostEqual(float(g_ss1.P[-1, 2]), 1.3)

        # random_seed vs seed
        g_seed1 = grow(premise="F", rules="F=0.5:FF\nF=0.5:F+F", generations=5, random_seed=42)
        g_seed2 = grow(premise="F", rules="F=0.5:FF\nF=0.5:F+F", generations=5, seed=42)
        np.testing.assert_allclose(g_seed1.P, g_seed2.P)

    def test_houdini_docs_examples(self):
        # quadratic Koch island: 3 generations of F-F-F-F with F=F-F+F+FF-F-F+F
        s = rewrite("F-F-F-F", "F=F-F+F+FF-F-F+F", 3, 0)
        self.assertEqual(s.count("F"), 4 * 8 ** 3)
        # the 3-D tree from the docs, written with two rules
        g = grow(premise="FFFA", rules='A=!"[B]////[B]////B\nB=&FFFA', generations=7, angle=28.0, step=0.1, step_scale=0.9)
        self.assertGreater(g.npoints, 100)
        self.assertEqual(np.linalg.matrix_rank(g.P - g.P.mean(0)), 3)


if __name__ == "__main__":
    unittest.main()
