"""Seeded, bounded parameter fuzzing: every node type cooked with random parameters inside (and at the edges of) their declared
ranges, on a few small inputs. A node may refuse with a clear ValueError, but it must not leak a numpy error, must not hang or
take unreasonably long, and must return valid geometry when it succeeds. The draws are deterministic (seeded per node), so a
failure names the exact parameters and reproduces."""
import random
import threading
import time
import unittest

from minihou import Evaluator, Graph, NodeError, proxy
from minihou.graph import GraphError
from minihou.sdk import REGISTRY

SKIP = {"blender_object", "input", "output", "modifier_input", "python", "rbd_solver", "usd_import", "solver", "for_each",
        "loop_begin", "loop_end", "lsystem"}                         # need a partner node, a file, a simulation or an outside engine
# Native Blender tools are external integrations, exercised with real meshes in
# test_blender_modelling_tools rather than multiplied through the NumPy fuzz sweep.
from minihou.nodes.blender_modeling import TOOLS as NATIVE_TOOLS
SKIP.update({spec[0] for spec in NATIVE_TOOLS} | {"blender_mesh"})
SLOW_SECONDS = 30.0                                                  # one cook taking longer than this is a finding, hung or not
import os
DRAWS = int(os.environ.get("MINIHOU_FUZZ_DRAWS", "3"))                       # more draws for a longer hunt: MINIHOU_FUZZ_DRAWS=20
NUMPY_WORDS = ("reshape", "zero-size", "non-negative integer", "broadcast", "out of bounds", "axis", "could not convert",
               "cannot convert", "unsupported operand", "object of type")
BASE = {
    "grid": {"type": "grid", "params": {"columns": 7, "rows": 6}},
    "box": {"type": "box", "params": {"divisions": 2}},
    "sphere": {"type": "sphere", "params": {"rows": 6, "columns": 8}},
    "curve": {"type": "helix", "params": {"points": 24}},
    "points": {"type": "scatter", "inputs": {"geo": "grid"}, "params": {"count": 30}},
}


def draw(p, rng):
    """One random value for parameter ``p``: mostly inside its range, sometimes at an edge."""
    if p.kind == "bool":
        return rng.random() < 0.5
    if p.kind == "enum":
        return rng.choice(p.choices)
    if p.kind == "str":
        return rng.choice([p.default, p.default, "", "*", "0", "0-3", "1 2 5", "nonexistent"]) if p.ui != "code" else p.default
    if p.kind == "vec3":
        base = [rng.uniform(-2, 2) for _ in range(3)]
        return tuple(min(max(v, p.min if p.min is not None else v), p.max if p.max is not None else v) for v in base)
    lo = p.min if p.min is not None else (p.soft_min if p.soft_min is not None else min(0, p.default) - 5)
    hi = p.max if p.max is not None else (p.soft_max if p.soft_max is not None else max(p.default * 4, 8))
    if p.soft_max is not None:
        hi = min(hi, p.soft_max * 2)                                 # past the slider range lies "hopeless" territory, not a bug
    if p.kind == "int":
        hi = min(hi, max(lo, 20_000))                                # counts of millions are a benchmark, not a unit test
    hi = max(hi, lo)
    if p.kind == "int":
        pool = [lo, hi, p.default, rng.randint(int(lo), int(hi)), rng.randint(int(lo), int(hi))]
        v = int(rng.choice(pool))
        return int(min(max(v, p.min if p.min is not None else v), p.max if p.max is not None else v))
    pool = [lo, hi, p.default, rng.uniform(lo, hi), rng.uniform(lo, hi), 0.0 if lo <= 0.0 <= hi else lo]
    v = float(rng.choice(pool))
    return min(max(v, p.min if p.min is not None else v), p.max if p.max is not None else v)


def cook(node_type, src, params, level=None):
    nt = REGISTRY.get(node_type)
    inputs = {}
    for i, spec in enumerate(nt.inputs):
        inputs[spec.name] = [src, "box"] if spec.multi else (src if i == 0 else "sphere")
    nodes = dict(BASE)
    nodes["x"] = {"type": node_type, "inputs": inputs, "params": params}
    nodes["o"] = {"type": "output", "inputs": {"geo": "x"}}
    ev = Evaluator()
    ev.proxy_level = level
    g = ev.cook(Graph.from_dict({"nodes": nodes, "display": "o"}, check=False))
    g.validate()


def run_guarded(fn, seconds):
    """Run ``fn`` on a daemon thread; (result, exception, finished)."""
    box = {}

    def target():
        try:
            box["result"] = fn()
        except BaseException as e:  # noqa: BLE001
            box["error"] = e
    t = threading.Thread(target=target, daemon=True)
    t0 = time.perf_counter()
    t.start()
    t.join(seconds)
    return box, not t.is_alive(), time.perf_counter() - t0


class ParameterFuzzTests(unittest.TestCase):
    def check(self, node_type, src, params, level=None):
        box, finished, secs = run_guarded(lambda: cook(node_type, src, params, level), SLOW_SECONDS * 3)
        label = f"{node_type} on {src} with {params}" + (f" at proxy {level}" if level else "")
        self.assertTrue(finished, f"{label} did not finish in {SLOW_SECONDS * 3:.0f} s")
        self.assertLess(secs, SLOW_SECONDS, f"{label} took {secs:.1f} s")
        e = box.get("error")
        if e is None or isinstance(e, GraphError):
            return
        self.assertIsInstance(e, NodeError, f"{label}: {type(e).__name__}: {e}")
        cause = e.__cause__
        self.assertIsInstance(cause, (ValueError, ArithmeticError, type(None)), f"{label}: {type(cause).__name__}: {cause}")
        self.assertFalse(any(w in str(cause) for w in NUMPY_WORDS), f"{label}: {cause}")

    def test_random_parameters_on_every_node(self):
        names = sorted(n for n in REGISTRY.names() if n not in SKIP and REGISTRY.get(n).category != "system")
        self.assertGreater(len(names), 100)
        started = time.perf_counter()
        for name in names:
            nt = REGISTRY.get(name)
            if not nt.params:
                continue
            for k in range(DRAWS):
                rng = random.Random(f"{name}:{k}")
                params = {p.name: draw(p, rng) for p in nt.params}
                for src in ("box", "points", "curve"):
                    with self.subTest(node=name, input=src, draw=k):
                        self.check(name, src, params)
        self.assertLess(time.perf_counter() - started, 240, "the fuzz run itself got too slow")

    def test_proxy_levels_never_break_a_node(self):
        names = sorted(n for n in REGISTRY.names() if REGISTRY.get(n).proxy_params)
        self.assertGreater(len(names), 10)
        for name in names:
            nt = REGISTRY.get(name)
            for level in (0.5, 0.1, 0.01, proxy.MIN_LEVEL):
                rng = random.Random(f"{name}:proxy:{level}")
                params = {p.name: draw(p, rng) for p in nt.params}
                with self.subTest(node=name, level=level):
                    self.check(name, "box" if nt.inputs else "box", params, level)


class FuzzFindingsStayFixed(unittest.TestCase):
    """Bugs the fuzzer found, pinned so they cannot come back."""

    def test_subdivide_with_creases_on_geometry_without_faces(self):
        cook("subdivide", "points", {"depth": 1, "creases": "0", "use_creases": True, "override_crease_weight": True})   # was an IndexError
        with self.assertRaises(NodeError):                                                       # curves are refused with a clear message
            cook("subdivide", "curve", {"depth": 1, "creases": "0"})

    def test_rbd_material_fracture_refuses_a_hopeless_piece_count_quickly(self):
        t0 = time.perf_counter()
        with self.assertRaises(NodeError) as cm:
            cook("rbd_material_fracture", "box", {"pieces": 26, "fracture_level": 3, "sub_pieces": 64})
        self.assertIn("limit", str(cm.exception))
        self.assertLess(time.perf_counter() - t0, 5.0)


if __name__ == "__main__":
    unittest.main()
