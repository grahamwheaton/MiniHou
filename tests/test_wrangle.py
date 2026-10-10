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


if __name__ == "__main__":
    unittest.main()
