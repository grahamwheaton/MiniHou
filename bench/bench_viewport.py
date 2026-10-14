"""Viewport preparation time and payload for big grids: python bench/bench_viewport.py [columns ...]

Measures what the editor's preview request costs on the engine side (no GPU involved): a full-resolution view of a fresh
topology, the same after a deformer (positions only), and the bytes each sends."""
import sys
import time

from minihou import accel, preview
from minihou.evaluator import Evaluator
from minihou.graph import Graph
import minihou.nodes  # noqa: F401


def main(sizes):
    accel.warmup()
    for n in sizes:
        g = Graph.from_dict({"nodes": {"g": {"type": "grid", "params": {"columns": n, "rows": n}},
                                       "m": {"type": "noise", "inputs": {"geo": "g"}, "params": {}},
                                       "t": {"type": "transform", "inputs": {"geo": "m"}, "params": {"translate": [0, 0, 1]}}},
                             "display": "t"})
        ev = Evaluator()
        a, b = ev.cook(g, "m"), ev.cook(g, "t")
        preview._LAYOUTS.clear()
        t0 = time.perf_counter(); h, arr = preview._arrays_for(a, full=True); t1 = time.perf_counter() - t0
        t0 = time.perf_counter(); h2, arr2 = preview._arrays_for(b, full=True); t2 = time.perf_counter() - t0
        full_mb = sum(x.nbytes for x in arr.values()) / 1e6
        pos_mb = sum(x.nbytes for k, x in arr2.items() if k not in ("tri", "tri_prim", "edge")) / 1e6
        print(f"{n}x{n} grid ({a.nprims:,} quads): new topology {t1:.2f} s, {full_mb:.0f} MB, {h['tris']:,} triangles shown"
              f"{' (reduced)' if h['proxy']['active'] else ''}; positions only {t2:.2f} s, {pos_mb:.0f} MB")


if __name__ == "__main__":
    main([int(a) for a in sys.argv[1:]] or [1000, 2000])
