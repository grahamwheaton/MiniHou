import time
import unittest

import numpy as np

from minihou import Evaluator, Geometry, REGISTRY
from minihou.wrangle import FUNCTIONS, WrangleError, compile_snippet
from tests.helpers import graph_from
from tests.test_nodes import run


def grid(n=5):
    return run("grid", columns=n, rows=n, size_x=4, size_y=4)


def w(geo, snippet):
    return run("wrangle", geo, snippet=snippet)


class WrangleSemanticsTests(unittest.TestCase):
    def test_component_update(self):
        g = grid()
        out = w(g, "@P.z += sin(@P.x)")
        np.testing.assert_allclose(out.P[:, 2], np.sin(g.P[:, 0]), atol=1e-6)
        np.testing.assert_array_equal(out.P[:, :2], g.P[:, :2])

    def test_float_times_vector_broadcasts(self):
        g = grid()
        out = w(g, "f@k = @P.x; @P = @P * @k")
        np.testing.assert_allclose(out.P, g.P * g.P[:, :1], atol=1e-6)

    def test_new_attributes_by_prefix_and_inference(self):
        g = grid()
        out = w(g, "v@Cd = set(@P.x, 0, 1); i@id = @ptnum * 2; f@h = @P.y; @auto = length(@P)")
        self.assertEqual((out.point["Cd"].shape, out.point["Cd"].dtype), ((25, 3), np.float32))
        self.assertEqual(out.point["id"].dtype, np.int32)
        np.testing.assert_array_equal(out.point["id"], np.arange(25) * 2)
        np.testing.assert_allclose(out.point["Cd"][:, 0], g.P[:, 0])
        np.testing.assert_allclose(out.point["Cd"][:, 2], 1)
        self.assertEqual(out.point["auto"].shape, (25,))
        np.testing.assert_allclose(out.point["auto"], np.linalg.norm(g.P, axis=1), atol=1e-6)

    def test_component_write_on_new_vector_starts_at_zero(self):
        out = w(grid(), "v@Cd.y = 1")
        np.testing.assert_array_equal(out.point["Cd"], np.tile([0, 1, 0], (25, 1)))

    def test_conditionals_and_boolean_logic(self):
        g = grid()
        out = w(g, "f@q = 1 if @P.x > 0 and not @P.y > 0 else -1")
        want = np.where((g.P[:, 0] > 0) & ~(g.P[:, 1] > 0), 1.0, -1.0)
        np.testing.assert_array_equal(out.point["q"], want)
        out2 = w(g, "f@r = 0 < @P.x <= 1")          # chained comparison
        np.testing.assert_array_equal(out2.point["r"], ((g.P[:, 0] > 0) & (g.P[:, 0] <= 1)).astype(np.float32))

    def test_locals_multiline_and_comments(self):
        g = grid()
        out = w(g, "# lift by distance\nd = length(@P)\nd *= 0.5\n@P.z = d")
        np.testing.assert_allclose(out.P[:, 2], np.linalg.norm(g.P, axis=1) * 0.5, atol=1e-6)

    def test_vector_functions(self):
        g = grid()
        out = w(g, "v@N = normalize(set(@P.x, @P.y, 1)); f@d = dot(@N, set(0, 0, 1)); v@c = cross(@N, set(0, 0, 1))")
        n = np.stack([g.P[:, 0], g.P[:, 1], np.ones(25)], 1)
        n /= np.linalg.norm(n, axis=1, keepdims=True)
        np.testing.assert_allclose(out.point["N"], n, atol=1e-6)
        np.testing.assert_allclose(out.point["d"], n[:, 2], atol=1e-6)
        np.testing.assert_allclose(out.point["c"], np.cross(n, [0, 0, 1]), atol=1e-6)

    def test_fit_clamp_lerp_rand_noise(self):
        g = grid()
        out = w(g, "f@a = fit(@P.x, -2, 2, 0, 1); f@b = clamp(@P.x, -1, 1); f@c = lerp(0, 10, 0.25);"
                   " f@r = rand(@ptnum, 3); f@n = noise(@P)")
        np.testing.assert_allclose(out.point["a"], (g.P[:, 0] + 2) / 4, atol=1e-6)
        np.testing.assert_allclose(out.point["b"], np.clip(g.P[:, 0], -1, 1))
        np.testing.assert_allclose(out.point["c"], 2.5)
        r = out.point["r"]
        self.assertTrue(((r >= 0) & (r < 1)).all() and len(np.unique(r)) == 25)
        np.testing.assert_array_equal(r, w(g, "f@r = rand(@ptnum, 3)").point["r"])           # deterministic
        self.assertFalse(np.array_equal(r, w(g, "f@r = rand(@ptnum, 4)").point["r"]))          # seeded
        self.assertLess(float(np.abs(out.point["n"]).max()), 1.2)

    def test_constants_and_integer_attribute_type_is_kept(self):
        g = grid().copy()
        g.point["id"] = np.arange(25, dtype=np.int32)
        out = w(g.freeze(), "@id += 1; f@pi = PI")
        self.assertEqual(out.point["id"].dtype, np.int32)
        np.testing.assert_array_equal(out.point["id"], np.arange(1, 26))
        np.testing.assert_allclose(out.point["pi"], np.pi, atol=1e-6)

    def test_points_only_and_empty_geometry(self):
        pts = Geometry.points_only(np.random.default_rng(0).random((10, 3), dtype=np.float32)).freeze()
        self.assertEqual(w(pts, "@P.z = 0").P[:, 2].tolist(), [0.0] * 10)
        empty = Geometry.points_only(np.zeros((0, 3), np.float32)).freeze()
        self.assertEqual(w(empty, "@P.z += 1; f@m = 1").npoints, 0)


class WrangleEngineTests(unittest.TestCase):
    def test_inputs_are_not_modified_and_untouched_attributes_are_shared(self):
        g = grid()
        before = g.P.copy()
        out = w(g, "@P.z += 1")
        np.testing.assert_array_equal(g.P, before)
        self.assertIs(out.point["uv"], g.point["uv"])                 # unchanged attrs stay shared -> not re-sent
        self.assertIs(out.topology, g.topology)

    def test_compiled_once_per_snippet(self):
        compile_snippet.cache_clear()
        g = grid()
        for _ in range(3):
            w(g, "@P.z += 2")
        info = compile_snippet.cache_info()
        self.assertEqual((info.misses, info.hits), (1, 2))

    def test_in_a_graph_editing_the_snippet_recooks_only_the_wrangle(self):
        gr = graph_from({"g": {"type": "grid"}, "w": {"type": "wrangle", "params": {"snippet": "@P.z = 1"},
                                                     "inputs": {"geo": "g"}},
                         "o": {"type": "output", "inputs": {"geo": "w"}}}, display="o")
        ev = Evaluator()
        np.testing.assert_allclose(ev.cook(gr).P[:, 2], 1)
        gr.set_param("w", "snippet", "@P.z = 2")
        np.testing.assert_allclose(ev.cook(gr).P[:, 2], 2)
        self.assertEqual(ev.last_stats.cooked, ["w", "o"])

    def test_errors_surface_as_node_errors_with_location(self):
        from minihou import NodeError
        gr = graph_from({"g": {"type": "grid"}, "w": {"type": "wrangle", "params": {"snippet": "@P.z = foo"},
                                                     "inputs": {"geo": "g"}}}, display="w")
        with self.assertRaises(NodeError) as cm:
            Evaluator().cook(gr)
        self.assertIn("line 1", str(cm.exception))
        self.assertIn("foo", str(cm.exception))

    def test_speed_one_million_points(self):
        g = run("grid", columns=1000, rows=1000)
        w(g, "@P.z += sin(@P.x * 2) * 0.5")            # warm
        t = time.perf_counter()
        w(g, "@P.z += sin(@P.x * 2) * 0.5")
        self.assertLess(time.perf_counter() - t, 1.0)   # numpy-vectorised; typically ~20-40 ms


class WrangleSafetyTests(unittest.TestCase):
    """Snippets come from graph files people share: they must not be able to run arbitrary Python."""

    ATTACKS = [
        "import os", "from os import system", "__import__('os')", "@P.z = __import__('os')",
        "@P.z = ().__class__", "@P.z = sin.__globals__", "@P.z = (1).__class__.__bases__", "@P.z = [1][0]",
        "@P.z = {'a': 1}", "@P.z = 'text'", "@P.z = b'x'", "@P.z = lambda: 1", "@P.z = [x for x in (1,)]",
        "@P.z = open('x')", "@P.z = eval('1')", "@P.z = exec", "@P.z = getattr(@P, 'x')", "@P.z = globals()",
        "def f(): pass", "class A: pass", "for i in (1,): pass", "while 0: pass", "with x: pass", "try:\n x=1\nexcept: pass",
        "del @P", "@P.z = (yield)", "@P.z = sin(*(1,))", "@P.z = sin(x=1)", "@P.z = __builtins__",
        "sin = 1", "@P.z = @P.__class__", "@P.z = 1 @ 2", "@P.z = 1 << 2", "@P.z = True", "@P.z = None",
        "@P.z = (lambda: 1)()", "@P.z = sin(1)(2)", "@P.z := 1", "global x", "assert 1", "raise ValueError", "pass",
    ]

    def test_attacks_are_rejected_at_compile_time(self):
        for snippet in self.ATTACKS:
            with self.subTest(snippet=snippet):
                with self.assertRaises(WrangleError):
                    compile_snippet(snippet)

    def test_compiled_code_has_no_builtins(self):
        prog = compile_snippet("@P.z += 1")
        self.assertEqual(prog.fn.__globals__["__builtins__"], {})
        self.assertNotIn("__import__", prog.fn.__globals__)

    def test_reference_covers_every_function(self):
        from minihou.wrangle import SIGNATURES, reference
        self.assertEqual(set(SIGNATURES), set(FUNCTIONS))
        ref = reference()
        for name in FUNCTIONS:
            self.assertIn(name + "(", ref)

    def test_function_table_is_numeric_only(self):
        for name, fn in FUNCTIONS.items():
            self.assertTrue(callable(fn), name)
            self.assertFalse(name.startswith("_"), name)

    def test_error_messages_point_at_the_problem(self):
        with self.assertRaises(WrangleError) as cm:
            compile_snippet("@P.z += 1\n@P.y = bogus(2)")
        self.assertIn("line 2", str(cm.exception))
        self.assertIn("unknown function 'bogus'", str(cm.exception))
        with self.assertRaises(WrangleError) as cm:
            compile_snippet("@P.z = (1 +")
        self.assertIn("syntax error", str(cm.exception))
        with self.assertRaises(WrangleError) as cm:
            w(grid(), "@P.z = @missing")
        self.assertIn("unknown attribute(s) @missing", str(cm.exception))
        with self.assertRaises(WrangleError) as cm:
            w(grid(), "f@P = 1")
        self.assertIn("exists as vector", str(cm.exception))


def wd(geo, snippet, run_over):
    return run("wrangle", geo, snippet=snippet, run_over=run_over)


class DomainTests(unittest.TestCase):
    """Running over primitives and vertices, reading across domains (Blender's FACE / CORNER domains)."""

    def setUp(self):
        self.g = grid(3)                                     # 9 points, 4 quads, 16 corners; uv on points

    def test_over_primitives_reads_face_centres_and_writes_prim_attributes(self):
        out = wd(self.g, "f@h = @P.x; i@id = @primnum * 10; i@n = @numprim", "primitives")
        self.assertEqual(out.prim["h"].shape, (4,))
        centres = self.g.P[self.g.topology.vertex_point].reshape(4, 4, 3).mean(1)
        np.testing.assert_allclose(out.prim["h"], centres[:, 0], atol=1e-6)
        np.testing.assert_array_equal(out.prim["id"], [0, 10, 20, 30])
        np.testing.assert_array_equal(out.prim["n"], [4, 4, 4, 4])
        self.assertNotIn("h", out.point)

    def test_over_vertices_reads_point_and_prim_attributes(self):
        g = wd(self.g, "i@mat = @primnum", "primitives")
        out = wd(g, "u@uv = set(@P.x, @P.y); i@pt = @ptnum; i@m = @mat; i@v = @vtxnum", "vertices")
        vp = g.topology.vertex_point
        self.assertEqual(out.vertex["uv"].shape, (16, 2))
        np.testing.assert_allclose(out.vertex["uv"], g.P[vp][:, :2], atol=1e-6)
        np.testing.assert_array_equal(out.vertex["pt"], vp)
        np.testing.assert_array_equal(out.vertex["m"], np.repeat(np.arange(4), 4))
        np.testing.assert_array_equal(out.vertex["v"], np.arange(16))
        self.assertNotIn("uv", out.point)                    # moved from points to vertices (one domain per name)

    def test_writing_a_promoted_name_moves_it(self):
        g = wd(self.g, "v@Cd = set(@ptnum, 0, 0)", "points")
        out = wd(g, "@Cd *= 2", "vertices")
        self.assertNotIn("Cd", out.point)
        np.testing.assert_allclose(out.vertex["Cd"][:, 0], 2 * g.topology.vertex_point)
        out2 = wd(g, "@Cd.y = 1", "primitives")             # averaged over each face, then written per face
        self.assertEqual(out2.prim["Cd"].shape, (4, 3))
        np.testing.assert_allclose(out2.prim["Cd"][:, 1], 1)

    def test_vector2_and_vector4(self):
        out = wd(self.g, "p@Cd = set(1, 0.5, 0, 1); u@st = set(@P.x, 0)", "points")
        self.assertEqual(out.point["Cd"].shape, (9, 4))
        self.assertEqual(out.point["st"].shape, (9, 2))
        np.testing.assert_allclose(out.point["Cd"][0], [1, 0.5, 0, 1])
        out2 = wd(self.g, "@uv.x = 1 - @uv.x", "points")    # an existing 2-component attribute keeps its width
        self.assertEqual(out2.point["uv"].shape, (9, 2))
        np.testing.assert_allclose(out2.point["uv"][:, 0], 1 - self.g.point["uv"][:, 0])
        with self.assertRaises(WrangleError) as cm:
            wd(self.g, "@uv = set(1, 2, 3)", "points")
        self.assertIn("vector2", str(cm.exception))

    def test_errors_and_specials_per_domain(self):
        with self.assertRaises(WrangleError) as cm:
            wd(self.g, "@P.z = 1", "primitives")
        self.assertIn("only be written when running over points", str(cm.exception))
        with self.assertRaises(WrangleError) as cm:
            wd(self.g, "i@x = @vtxnum", "points")
        self.assertIn("@vtxnum", str(cm.exception))
        self.assertIn("@ptnum", str(cm.exception))           # lists what is available here
        with self.assertRaises(WrangleError):
            wd(self.g, "@primnum = 1", "primitives")         # read-only

    def test_material_index_and_sharp_face_style_attributes(self):
        out = wd(self.g, "i@material_index = 1 if @P.x > 0 else 0; i@sharp_face = 1", "primitives")
        self.assertEqual(out.prim["material_index"].dtype, np.int32)
        self.assertEqual(sorted(set(out.prim["material_index"].tolist())), [0, 1])


class AttributeNodeTests(unittest.TestCase):
    def test_delete_with_wildcards_and_rename(self):
        g = wd(grid(3), "f@tmp_a = 1; f@tmp_b = 2; f@keep = 3; v@N = set(0, 0, 1)", "points")
        out = run("attribute_delete", g, names="tmp_* N P")
        self.assertEqual(sorted(out.point.names()), ["P", "keep", "uv"])
        r = run("attribute_rename", g, from_name="keep", to_name="pscale")
        self.assertIn("pscale", r.point)
        self.assertNotIn("keep", r.point)
        np.testing.assert_allclose(r.point["pscale"], 3)
        self.assertIs(run("attribute_rename", g, from_name="", to_name="x"), g)
        with self.assertRaises(Exception):
            run("attribute_rename", g, from_name="missing", to_name="x")


if __name__ == "__main__":
    unittest.main()
