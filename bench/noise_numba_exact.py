"""Time the warmed Numba noise kernels at one and four million points."""
import time
import sys
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from minihou.nodes import _kernels as k


def measure(fn):
    times = []
    for _ in range(3):
        start = time.perf_counter()
        fn()
        times.append(time.perf_counter() - start)
    return min(times)


for size in (1_000_000, 4_000_000):
    rng = np.random.default_rng(13)
    pos = rng.uniform(-4, 4, (size, 3)).astype(np.float32)
    scalar = np.empty(size, np.float32)
    vector = np.empty_like(pos)
    offset = np.array([0.2, -0.4, 0.1], np.float32)
    amplitude = np.array([0.3, 0.7, 1.1], np.float32)
    fbm = lambda: k.fbm3(pos, 7, 3, 2.1, 0.45, scalar)
    noise = lambda: k.noise_vec(pos, 1.3, offset, amplitude, 7, 3, 2.1, 0.45, True, True, vector)
    fbm()
    noise()
    print(f"{size:,}\tfbm3 {measure(fbm):.6f}s\tnoise_vec {measure(noise):.6f}s", flush=True)
