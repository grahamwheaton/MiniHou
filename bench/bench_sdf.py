"""Full sdf_boolean node timing: numpy versus Rust, identical inputs, ~1M/~8M voxels.

Run from the checkout: python bench/bench_sdf.py --repeat 3 --json bench/results/sdf.json
Both paths exclude source construction and include triangulation, fields and meshing.
"""
import argparse
import contextlib
import json
import platform
import statistics
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import numpy as np
from minihou import Evaluator, Graph, accel, rust, sdf
from minihou.nodes.volume import sdf_boolean, _tris


def inputs():
    out = []
    with rust.disabled(), accel.disabled():
        for center in ([0, 0, 0], [0.31, 0.23, 0.17]):
            out.append(Evaluator().cook(Graph.from_dict({"nodes": {"b": {"type": "box",
                "params": {"divisions": 8, "center": center}}}, "display": "b"})))
    return out


def run(repeat):
    a, b = inputs()
    allp = np.concatenate([_tris(g, "benchmark").reshape(-1, 3) for g in (a, b)])
    rows = []
    for target in (1_000_000, 8_000_000):
        lower, upper = 0.001, 0.1
        for _ in range(45):
            h = (lower + upper) / 2
            dims = np.floor((allp.max(0) - allp.min(0)) / h + 10).astype(int) + 2
            if int(np.prod(dims)) > target:
                lower = h
            else:
                upper = h
        h = upper
        _, dims = sdf.grid_for(allp.min(0), allp.max(0), h, 5 * h)
        row = {"voxel_size": h, "dims": dims.tolist(), "voxels": int(np.prod(dims))}
        outputs = []
        for name, context in (("numpy", rust.disabled), ("rust", contextlib.nullcontext)):
            with context(), accel.disabled():
                def cook():
                    return sdf_boolean(a, b, "union", h, 0.0, False)
                result = cook()
                runs = []
                for _ in range(repeat):
                    start = time.perf_counter()
                    result = cook()
                    runs.append(time.perf_counter() - start)
            outputs.append(result)
            row[name + "_seconds"] = statistics.median(runs)
            row[name + "_runs"] = runs
        np.testing.assert_array_equal(outputs[0].topology.vertex_point, outputs[1].topology.vertex_point)
        np.testing.assert_array_equal(outputs[0].topology.prim_offsets, outputs[1].topology.prim_offsets)
        np.testing.assert_allclose(outputs[0].P, outputs[1].P, atol=1e-6, rtol=0)
        row["speedup"] = row["numpy_seconds"] / row["rust_seconds"]
        row["points"] = outputs[1].npoints
        rows.append(row)
        print(json.dumps(row), flush=True)
    return {"python": platform.python_version(), "numpy": np.__version__, "platform": platform.platform(),
            "repeat": repeat, "rows": rows}


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repeat", type=int, default=3)
    parser.add_argument("--json", type=Path)
    args = parser.parse_args()
    if not rust.enabled():
        parser.error("build and install the Rust extension first")
    result = run(args.repeat)
    if args.json:
        args.json.parent.mkdir(parents=True, exist_ok=True)
        args.json.write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
