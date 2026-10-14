"""What the first cook and the first viewport of a big mesh cost right after the engine starts: each backend in its own fresh
process (numpy only, numba, Rust), so nothing is compiled, cached or already mapped.

    python bench/bench_first_call.py              # 1M and 4M quads
    python bench/bench_first_call.py --sizes 2000

numba's kernels are compiled ahead by ``accel.warmup()`` on tiny arrays, as the engine does when it starts; the time that
leaves for the first real call is what an artist waits for.
"""
from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

CHILD = r"""
import json, sys, time
t0 = time.perf_counter()
from minihou import accel, preview, rust
from minihou.evaluator import Evaluator
from minihou.graph import Graph
accel.warmup()
start = time.perf_counter() - t0
side = int(sys.argv[1])
g = Graph.from_dict({"nodes": {"g": {"type": "grid", "params": {"columns": side + 1, "rows": side + 1}},
     "n0": {"type": "noise", "inputs": {"geo": "g"}, "params": {"amplitude": [0.2, 0.2, 1.0], "octaves": 4}},
     "n1": {"type": "transform", "inputs": {"geo": "n0"}, "params": {"rotate": [0, 0, 30]}},
     "n2": {"type": "normal", "inputs": {"geo": "n1"}}}, "display": "n2"})
t = time.perf_counter(); geo = Evaluator().cook(g, "n2"); cook = time.perf_counter() - t
t = time.perf_counter(); preview._arrays_for(geo, full=True); view = time.perf_counter() - t
print(json.dumps({"start": start, "cook": cook, "view": view, "numba": accel.status(), "rust": rust.status()}))
"""

BACKENDS = {"numpy": {"MINIHOU_NO_RUST": "1", "MINIHOU_NO_NUMBA": "1"}, "numba": {"MINIHOU_NO_RUST": "1"}, "rust": {}}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--sizes", default="1000,2000")
    ap.add_argument("--json")
    args = ap.parse_args()
    rows = []
    for side in (int(s) for s in args.sizes.split(",")):
        for name, env in BACKENDS.items():
            out = subprocess.run([sys.executable, "-c", CHILD, str(side)], cwd=str(ROOT), capture_output=True, text=True,
                                 encoding="utf-8", env={**os.environ, "PYTHONPATH": str(ROOT), **env}, check=True)
            r = json.loads(out.stdout.strip().splitlines()[-1])
            r.update(backend=name, quads=side * side)
            rows.append(r)
            print(f"{side * side:>10,} quads  {name:6}  start+warm-up {r['start']:5.2f} s   first cook {r['cook']:6.2f} s   "
                  f"first viewport {r['view']:6.2f} s")
    if args.json:
        Path(args.json).write_text(json.dumps(rows, indent=2), encoding="utf-8")


if __name__ == "__main__":
    main()
