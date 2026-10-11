"""Deterministic RBD wall workload, with cold build and warm incremental timing."""
import json
from pathlib import Path
import statistics
import sys
import time

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
from minihou import Evaluator, Graph
from tools.make_rbd_examples import wall


def benchmark(columns, rows):
    graph = Graph.from_dict(wall(columns, rows))
    ev = Evaluator()
    t = time.perf_counter()
    ev.cook(graph, frame=1)
    cold = (time.perf_counter()-t)*1000
    times = []
    for frame in range(2, 121):
        t = time.perf_counter()
        ev.cook(graph, frame=frame)
        times.append((time.perf_counter()-t)*1000)
    world = ev.rbd.worlds['sim'][1]
    report = dict(bricks=columns*rows, dynamic_bodies=columns*rows+1, frames=120,
                  cold_ms=round(cold, 2), median_ms=round(statistics.median(times), 2),
                  p95_ms=round(sorted(times)[int(.95*len(times))], 2),
                  total_seconds=round((cold+sum(times))/1000, 3),
                  pose_cache_bytes=world.cache_bytes, geometry_cache_bytes=ev.cache.total_bytes)
    ev.close()
    return report


if __name__ == '__main__':
    result = dict(backend='mujoco 3.15.0', python=sys.version, workloads=[benchmark(5, 5), benchmark(10, 10)])
    print(json.dumps(result, indent=2))
    if len(sys.argv) > 1:
        Path(sys.argv[1]).write_text(json.dumps(result, indent=2), encoding='utf8')
