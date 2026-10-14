"""Optional Numba acceleration. The engine is complete without it (numpy only); Numba just makes the hot
nodes (noise, scatter, copy_to_points) several times faster on many points.

Reliability rules
-----------------
* Numba is optional. Import problems of *any* kind leave ``AVAILABLE = False`` and the numpy code runs.
* ``MINIHOU_NO_NUMBA=1`` forces the numpy path.
* The backend is decided by availability only, never by timing (no "use numpy until the JIT is warm"),
  so a given graph gives the same output no matter when you cook it. Warm-up can run in the background
  (:func:`warmup_async`); a cook that arrives first simply waits for it.
* Numba's default ``workqueue`` threading layer aborts the process if two Python threads launch parallel
  kernels at once. All launches go through :func:`call`, which serialises them with a lock.
* If a kernel raises, :func:`call` logs it, disables acceleration for the rest of the process and returns
  :data:`FAILED`; the caller then runs its numpy implementation. A broken JIT cannot fail a cook.
"""
from __future__ import annotations

import contextlib
import logging
import os
import threading
from typing import Any, Callable, Optional

log = logging.getLogger("minihou.accel")

AVAILABLE = False
IMPORT_ERROR = ""
numba: Any = None

if os.environ.get("MINIHOU_NO_NUMBA", "").strip().lower() in ("1", "true", "yes", "on"):
    IMPORT_ERROR = "disabled by MINIHOU_NO_NUMBA"
else:
    try:
        import numba  # noqa: F401
        AVAILABLE = True
    except Exception as e:  # noqa: BLE001 - ImportError, but also numba/numpy version-mismatch errors
        IMPORT_ERROR = f"{type(e).__name__}: {e}"

FAILED = object()                  # sentinel returned by call() when the caller must use its numpy path

_lock = threading.RLock()
_disabled_reason: Optional[str] = None
_successful_calls = {}  # process-local diagnostics; never affects cooked output/cache keys
_forced_off = 0                    # >0 inside `disabled()`


def enabled() -> bool:
    return AVAILABLE and _disabled_reason is None and _forced_off == 0


def status() -> str:
    if not AVAILABLE:
        return f"numpy only ({IMPORT_ERROR})"
    if _disabled_reason:
        return f"numpy only (numba disabled after error: {_disabled_reason})"
    return f"numba {numba.__version__}"


@contextlib.contextmanager
def disabled():
    """Force the numpy path (tests, A/B comparisons)."""
    global _forced_off
    with _lock:
        _forced_off += 1
    try:
        yield
    finally:
        with _lock:
            _forced_off -= 1


def kernels():
    """The kernel module, or None if acceleration is off or the module cannot be imported."""
    global _disabled_reason
    if not enabled():
        return None
    try:
        from .nodes import _kernels
        return _kernels
    except Exception as e:  # noqa: BLE001
        _disabled_reason = f"{type(e).__name__}: {e}"
        log.warning("numba kernels unavailable (%s); using numpy", _disabled_reason)
        return None


def call(kernel: Callable, *args):
    """Run a Numba kernel under the launch lock. Returns :data:`FAILED` if acceleration is unavailable
    or the kernel raised (in which case acceleration is switched off for good)."""
    global _disabled_reason
    if not enabled():
        return FAILED
    with _lock:
        try:
            result = kernel(*args)
            name = getattr(kernel, "__name__", type(kernel).__name__)
            _successful_calls[name] = _successful_calls.get(name, 0) + 1
            return result
        except Exception as e:  # noqa: BLE001
            _disabled_reason = f"{type(e).__name__}: {e}"
            log.warning("numba kernel %s failed (%s); falling back to numpy for this process",
                        getattr(kernel, "__name__", kernel), _disabled_reason)
            return FAILED


def diagnostics() -> dict:
    """Read-only proof of the active backend and successful kernel execution."""
    with _lock:
        return {"available": AVAILABLE, "enabled": enabled(), "status": status(),
                "import_error": IMPORT_ERROR, "successful_calls": dict(_successful_calls)}


def warmup() -> float:
    """Compile (or load from the on-disk cache) every kernel by running it on tiny inputs. Returns seconds."""
    import time
    import numpy as np
    t0 = time.perf_counter()
    if not enabled():
        return 0.0
    k = kernels()
    if k is None:
        return 0.0
    p = np.zeros((4, 3), np.float32)
    call(k.fbm3, p, 0, 2, 2.0, 0.5, np.empty(4, np.float32))
    call(k.noise_vec, p, 1.0, np.zeros(3, np.float32), np.ones(3, np.float32), 0, 2, 2.0, 0.5, False, True,
         np.empty((4, 3), np.float32))
    vp = np.array([0, 1, 2, 3], np.int32)
    offs = np.array([0, 4], np.int32)
    counts = np.array([2], np.int64)
    first = np.array([0], np.int64)
    area = np.empty(2, np.float64)
    tri_prim = np.empty(2, np.int32)
    p[1, 0] = p[2, 0] = p[2, 1] = p[3, 1] = 1.0
    call(k.tri_areas, p, vp, offs, counts, first, area, tri_prim)
    r = np.array([0.5, 0.25], np.float64)
    call(k.sample_surface, p, vp, offs, first, tri_prim, np.cumsum(area), r, r, r, np.empty((2, 3), np.float32),
         np.empty((2, 3), np.float32), np.empty(2, np.int32))
    call(k.copy_points, p, p, p, True, np.ones(4, np.float32), True, np.empty((16, 3), np.float32))
    pn = np.empty((1, 3), np.float64)
    call(k.prim_normals, p, vp, offs, pn)
    call(k.accumulate_point_normals, vp, offs, pn, np.empty((4, 3), np.float32))
    one = np.ones(1)
    call(k.pcg_csr, np.array([0, 1], np.int64), np.zeros(1, np.int64), one, one, np.zeros(1), one, 1e-30, 10)
    tris = np.array([[[0, 0, 0], [1, 0, 0], [0, 1, 0]]], np.float64)
    one_i, zero_i = np.zeros(1, np.int64), np.zeros(1, np.int64)
    lo, hv, dims = np.zeros(3), np.ones(3), np.ones(3, np.int64)
    call(k.grid_closest, np.zeros((1, 3)), tris, zero_i, np.ones(1, np.int64), one_i, lo, hv, dims,
         np.full(1, 1.0), np.zeros(1, np.int64), np.zeros((1, 3)))
    call(k.grid_intersect, np.array([[0.2, 0.2, 1.0]]), np.array([[0.0, 0.0, -1.0]]), tris, zero_i,
         np.ones(1, np.int64), one_i, lo, hv, hv, dims, np.inf, np.empty(1), np.empty(1, np.int64),
         np.empty((1, 2)))
    vp4 = np.array([0, 1, 2, 3], np.int64)
    lo, hi = np.empty(4, np.int64), np.empty(4, np.int64)
    call(k.edges_halves, vp4, np.array([0, 4], np.int64), np.ones(1, np.uint8), lo, hi)
    r = call(k.edges_bucket, lo, hi, 4)
    if r is not FAILED:
        per = np.empty(4, np.int64)
        call(k.edges_sort_count, *r, 4, per)
        base = np.zeros(4, np.int64)
        np.cumsum(per[:-1], out=base[1:])
        call(k.edges_number, *r, 4, base, np.empty((int(per.sum()), 2), np.int64), np.empty(int(per.sum()), np.int64), np.full(4, -1, np.int64))
    tri3 = np.array([[0, 1, 2]], np.int64)
    code, order = np.empty(1, np.int64), np.empty(1, np.int64)
    call(k.tri_morton, p, tri3, np.zeros(3), np.ones(3), code)
    call(k.radix_order30, code, order)
    call(k.chunk_bounds, p, tri3, 8192, np.empty((1, 8), np.float32))
    call(k.fan_point_triangles, vp4, np.array([0, 4], np.int64), np.ones(1, np.uint8), np.zeros(1, np.int64),
         np.empty((2, 3), np.int64), np.empty(2, np.int64))
    call(k.gather_rows3, tri3, order, np.empty((1, 3), np.int64))
    call(k.gather1, code, order, np.empty(1, np.int64))
    return time.perf_counter() - t0


def warmup_async() -> Optional[threading.Thread]:
    """Warm up in a daemon thread (the server calls this at start-up so the first cook is not slow)."""
    if not enabled():
        return None

    def run():
        try:
            dt = warmup()
            log.info("numba kernels ready in %.1f s", dt)
        except Exception:  # noqa: BLE001
            log.exception("numba warm-up failed")

    t = threading.Thread(target=run, name="minihou-warmup", daemon=True)
    t.start()
    return t
