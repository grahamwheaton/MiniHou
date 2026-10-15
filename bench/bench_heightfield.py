"""Benchmark heightfield erosion: numpy reference vs Rust core at 512x512.

    python bench/bench_heightfield.py
"""
from __future__ import annotations

import argparse
import json
import statistics
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

import numpy as np  # noqa: E402

from minihou import Evaluator, Graph, rust  # noqa: E402
from minihou.nodes import heightfield  # noqa: E402


def timed(fn, repeat=1):
    fn()  # warm-up
    runs = []
    for _ in range(repeat):
        t0 = time.perf_counter()
        fn()
        runs.append(time.perf_counter() - t0)
    return statistics.median(runs)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--size", type=int, default=512, help="grid resolution (default: 512)")
    ap.add_argument("--repeat", type=int, default=1, help="timing repeats (default: 1)")
    ap.add_argument("--json", help="optional output json file")
    args = ap.parse_args()

    if not rust.available():
        sys.exit("the Rust core is not built: see rust/README.md")

    h, w = args.size, args.size
    rng = np.random.default_rng(12345)
    H_init = rng.uniform(0.0, 100.0, size=(h, w)).astype(np.float64)
    cell = (10.0 / h, 10.0 / w)

    droplets = 4096
    hydraulic_iters = 30
    thermal_iters = 20

    rows = []

    # 1. Thermal kernel
    t_numpy = timed(lambda: heightfield.thermal(H_init.copy(), cell, 35.0, 0.5, thermal_iters), args.repeat)
    t_rust = timed(lambda: rust.thermal(H_init.copy(), cell, 35.0, 0.5, thermal_iters), args.repeat)
    rows.append({
        "case": f"thermal ({thermal_iters} iterations)",
        "numpy_s": t_numpy,
        "rust_s": t_rust,
        "speedup": t_numpy / t_rust,
    })

    # 2. Hydraulic kernel
    t_numpy = timed(
        lambda: heightfield.hydraulic(
            H_init.copy(), cell, hydraulic_iters, rain=0.01, capacity=4.0,
            erode_rate=0.3, deposit_rate=0.3, evaporation=0.02,
            droplets=droplets, lifetime=48, inertia=0.1, seed=42
        ),
        args.repeat
    )
    t_rust = timed(
        lambda: rust.hydraulic(
            H_init.copy(), cell, hydraulic_iters, rain=0.01, capacity=4.0,
            erode_rate=0.3, deposit_rate=0.3, evaporation=0.02,
            droplets=droplets, lifetime=48, inertia=0.1, seed=42
        ),
        args.repeat
    )
    rows.append({
        "case": f"hydraulic ({hydraulic_iters} iters x {droplets} droplets)",
        "numpy_s": t_numpy,
        "rust_s": t_rust,
        "speedup": t_numpy / t_rust,
    })

    # 3. Whole node heightfield_erode
    g_dict = {
        "version": 1,
        "nodes": {
            "g": {"type": "grid", "params": {"columns": w, "rows": h, "size_x": 10.0, "size_y": 10.0}},
            "n": {"type": "noise", "inputs": {"geo": "g"}, "params": {"amplitude": [0, 0, 3.5], "frequency": 0.11, "octaves": 4, "seed": 4}},
            "erode": {
                "type": "heightfield_erode",
                "inputs": {"geo": "n"},
                "params": {
                    "iterations": hydraulic_iters,
                    "droplets": droplets,
                    "thermal_iterations": thermal_iters,
                    "seed": 42,
                }
            }
        },
        "display": "erode"
    }

    def run_node_numpy():
        with rust.disabled():
            return Evaluator().cook(Graph.from_dict(g_dict), "erode")

    def run_node_rust():
        return Evaluator().cook(Graph.from_dict(g_dict), "erode")

    t_numpy = timed(run_node_numpy, args.repeat)
    t_rust = timed(run_node_rust, args.repeat)
    rows.append({
        "case": f"node: heightfield_erode ({w}x{h}, {hydraulic_iters}x{droplets} hyd + {thermal_iters} thm)",
        "numpy_s": t_numpy,
        "rust_s": t_rust,
        "speedup": t_numpy / t_rust,
    })

    # Format table
    head = f"{'case':56} {'points':>10} {'numpy':>11} {'rust':>11} {'speedup':>10}"
    print(head)
    print("-" * len(head))
    for r in rows:
        fmt = lambda s: f"{s * 1000:8.1f} ms" if s < 1.0 else f"{s:8.2f} s "
        print(f"{r['case']:56} {h * w:>10,} {fmt(r['numpy_s']):>11} {fmt(r['rust_s']):>11} {r['speedup']:9.1f}x")

    if args.json:
        Path(args.json).write_text(json.dumps(rows, indent=2), encoding="utf-8")


if __name__ == "__main__":
    main()
