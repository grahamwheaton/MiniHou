"""Rust core vs Python (numpy) vs numba, on the kernels and a small graph, at 1M and 4M quads.

    python bench/bench_rust.py                 # prints a table
    python bench/bench_rust.py --json out.json # also writes the numbers with the machine they came from

Each timing is the median of --repeat runs after one warm-up run; numba's first-call compile time is reported separately
(``jit_s``). Rust timings include the Python <-> Rust calls and building the result arrays. Needs the Rust extension
(rust/README.md); numba columns are left out when numba is not installed.
"""
from __future__ import annotations

import argparse
import json
import os
import platform
import statistics
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

import numpy as np  # noqa: E402

from minihou import Evaluator, Graph, accel, preview, rust, topo  # noqa: E402
from minihou.nodes import normal as normal_node  # noqa: E402
from minihou.nodes._util import fbm3, fbm3_numpy  # noqa: E402


def timed(fn, repeat):
    fn()                                                        # warm-up (numba compiles here, caches fill)
    runs = []
    for _ in range(repeat):
        t0 = time.perf_counter()
        fn()
        runs.append(time.perf_counter() - t0)
    return statistics.median(runs)


def first_call(fn):
    t0 = time.perf_counter()
    fn()
    return time.perf_counter() - t0


def grid_graph(side, *steps):
    nodes = {"g": {"type": "grid", "params": {"columns": side + 1, "rows": side + 1, "size_x": 10, "size_y": 10}}}
    prev = "g"
    for i, (t, p) in enumerate(steps):
        nodes[f"n{i}"] = {"type": t, "inputs": {"geo": prev}, "params": p}
        prev = f"n{i}"
    return {"version": 1, "nodes": nodes, "display": prev}


def py_cook(graph):
    g = Graph.from_dict(graph)
    return Evaluator().cook(g, g.output_node())                 # a fresh evaluator: no cache hits


def engine_rows(side, repeat, row):
    """What the editor and the link actually run: the Python engine cooking a graph, then preparing the viewport, with the Rust fast
    paths on (the default when the extension is installed) and off (numba)."""
    graph = grid_graph(side, ("noise", {"amplitude": [0.2, 0.2, 1.0], "octaves": 4}), ("transform", {"rotate": [0, 0, 30]}),
                       ("normal", {}))

    def cook():
        return py_cook(graph)

    def view_new():
        preview._LAYOUTS.clear()
        return preview._arrays_for(geo, full=True)

    def view_moved():                                       # a deformer downstream: same topology, new positions
        return preview._arrays_for(moved, full=True)

    def first_and_median(fn):
        return first_call(fn), timed(fn, repeat)

    G = Graph.from_dict(graph)
    seeds = iter(range(10_000))

    def slider():                                           # a slider drag on the noise: the grid stays cached
        G.nodes["n0"].params["seed"] = next(seeds)
        return ev.cook(G, "n2")

    out = {}
    for label, ctx in (("numba", rust.disabled), ("rust", __import__("contextlib").nullcontext)):
        with ctx():
            ev = Evaluator()
            ev.cook(G, "n2")
            out[label, "slider"] = (None, timed(slider, repeat))
            out[label, "cook"] = first_and_median(cook)
            geo = cook()
            moved = geo.copy()
            moved.point["P"] = (geo.P * np.float32(1.01)).astype(np.float32)
            out[label, "view"] = first_and_median(view_new)
            preview._arrays_for(geo, full=True)
            out[label, "moved"] = (None, timed(view_moved, repeat))
    for key, case in (("cook", "engine: cook grid > noise > transform > normal"),
                      ("slider", "engine: slider edit on noise (recook 3 nodes)"),
                      ("view", "engine: viewport, new topology"), ("moved", "engine: viewport, positions only")):
        first_n, first_r = out["numba", key][0], out["rust", key][0]
        note = f"first call: numba {round(first_n * 1000)} ms, rust {round(first_r * 1000)} ms" if first_n is not None else ""
        row(case, side, None, out["numba", key][1], out["rust", key][1], first_n, note)


def point_normals_numpy(geo):
    with accel.disabled(), rust.disabled():
        return normal_node.normal(geo, "point").point["N"]


def point_normals_numba(geo):
    with rust.disabled():
        return normal_node.normal(geo, "point").point["N"]


def edges_numpy(geo):
    with accel.disabled(), rust.disabled():
        return topo.edges(geo)


def edges_numba(geo):
    with rust.disabled():
        return topo.edges(geo)


def machine():
    cpu = platform.processor() or ""
    try:
        for line in Path("/proc/cpuinfo").read_text(encoding="utf-8").splitlines():
            if line.startswith("model name"):
                cpu = line.split(":", 1)[1].strip()
                break
    except OSError:
        pass
    return {"cpu": cpu, "threads": os.cpu_count(), "python": platform.python_version(), "numpy": np.__version__,
            "numba": accel.status(), "rust": rust.version(), "platform": platform.platform()}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--sizes", default="1000,2000", help="grid sides in quads (1000 = 1M quads)")
    ap.add_argument("--repeat", type=int, default=5)
    ap.add_argument("--json", help="write the results here")
    ap.add_argument("--engine", action="store_true", help="also time the Python engine (cook + viewport) with Rust on and off")
    args = ap.parse_args()
    if not rust.available():
        sys.exit("the Rust core is not built: see rust/README.md")
    numba = accel.enabled()
    rows = []

    def row(case, size, numpy_s, numba_s, rust_s, jit_s=None, note=""):
        rows.append({"case": case, "quads": size * size, "numpy_s": numpy_s, "numba_s": numba_s, "rust_s": rust_s,
                     "jit_s": jit_s, "note": note})

    for side in (int(s) for s in args.sizes.split(",")):
        with accel.disabled(), rust.disabled():
            geo = py_cook(grid_graph(side, ("noise", {"amplitude": [0.2, 0.2, 1.0]})))
        if args.engine:
            engine_rows(side, args.repeat, row)
        jit = first_call(lambda: edges_numba(geo)) if numba else None
        row("edges", side, timed(lambda: edges_numpy(geo), args.repeat),
            timed(lambda: edges_numba(geo), args.repeat) if numba else None, timed(lambda: rust.edges(geo), args.repeat), jit)

        jit = first_call(lambda: point_normals_numba(geo)) if numba else None
        row("point normals", side, timed(lambda: point_normals_numpy(geo), args.repeat),
            timed(lambda: point_normals_numba(geo), args.repeat) if numba else None,
            timed(lambda: rust.point_normals(geo), args.repeat), jit)

        pos = np.ascontiguousarray(geo.P)
        def fbm3_numba():
            with rust.disabled():
                return fbm3(pos, 1, 4, 2.0, 0.5)
        jit = first_call(fbm3_numba) if numba else None
        row("fbm3 noise, 4 octaves", side, timed(lambda: fbm3_numpy(pos, 1, 4, 2.0, 0.5), args.repeat),
            timed(fbm3_numba, args.repeat) if numba else None,
            timed(lambda: rust.fbm3(pos, 1, 4, 2.0, 0.5), args.repeat), jit, "per point")

        graph = grid_graph(side, ("noise", {"amplitude": [0.2, 0.2, 1.0], "octaves": 4}), ("transform", {"rotate": [0, 0, 30]}),
                           ("normal", {}))

        def py_graph():
            with accel.disabled(), rust.disabled():
                py_cook(graph)

        def numba_graph():
            with rust.disabled():
                py_cook(graph)
        row("graph: grid > noise > transform > normal", side, timed(py_graph, args.repeat),
            timed(numba_graph, args.repeat) if numba else None,
            timed(lambda: rust.cook(graph), args.repeat), None, "whole cook, fresh evaluator; Rust includes building the Geometry")

    head = f"{'case':44} {'quads':>10} {'numpy':>9} {'numba':>9} {'rust':>9} {'rust vs best':>13}"
    print(head)
    print("-" * len(head))
    for r in rows:
        best = min(x for x in (r["numpy_s"], r["numba_s"]) if x is not None)
        if r["note"].startswith("first call"):
            print(f"{'':44} {'':>10} {r['note']}")
        fmt = lambda x: f"{x * 1000:8.1f}ms" if x is not None else f"{'-':>10}"            # noqa: E731
        print(f"{r['case']:44} {r['quads']:>10,} {fmt(r['numpy_s'])} {fmt(r['numba_s'])} {fmt(r['rust_s'])} "
              f"{best / r['rust_s']:12.1f}x")
    jits = [r for r in rows if r["jit_s"]]
    if jits:
        print("numba first call (compile + run): " + ", ".join(f"{r['case']} {r['jit_s']:.2f}s" for r in jits[: len(jits) // 2 or 1]))
    info = machine()
    print(f"\n{info['cpu']} | {info['threads']} threads | Python {info['python']} | numpy {info['numpy']} | {info['numba']}")
    if args.json:
        Path(args.json).write_text(json.dumps({"machine": info, "repeat": args.repeat, "results": rows}, indent=2), encoding="utf-8")


if __name__ == "__main__":
    main()
