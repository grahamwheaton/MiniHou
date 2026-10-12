"""Time: the current frame a graph cooks at (Houdini's $F / $T), read by wrangles as ``@Frame`` and ``@Time``.

The evaluator sets the frame for the duration of a cook (a context variable, so nested system cooks on the same thread
see it and concurrent cooks on other threads do not interfere). Nodes whose string parameters mention ``@Frame`` or
``@Time`` get the frame in their cache key, so scrubbing the timeline recooks exactly what depends on time and nothing
else. ``Time`` is in seconds with frame 1 at time 0, like Houdini.
"""
from __future__ import annotations

import contextlib
import re
from contextvars import ContextVar
from typing import Iterator, Optional, Tuple

DEFAULT_FPS = 24.0
_STATE: ContextVar[Tuple[float, float]] = ContextVar("minihou_time", default=(1.0, DEFAULT_FPS))
_STEP: ContextVar[Optional[float]] = ContextVar("minihou_timeinc", default=None)
USES_TIME = re.compile(r"@(Frame|Time|TimeInc)\b")


def frame() -> float:
    return _STATE.get()[0]


def fps() -> float:
    return _STATE.get()[1]


def seconds() -> float:
    f, rate = _STATE.get()
    return (f - 1.0) / rate


def time_inc() -> float:
    """Seconds per step: 1 / fps, or a solver substep's share of it (Houdini's @TimeInc)."""
    step = _STEP.get()
    return step if step is not None else 1.0 / _STATE.get()[1]


@contextlib.contextmanager
def step(seconds_per_step: float) -> Iterator[None]:
    token = _STEP.set(float(seconds_per_step))
    try:
        yield
    finally:
        _STEP.reset(token)


@contextlib.contextmanager
def at(frame_: Optional[float], fps_: Optional[float] = None) -> Iterator[None]:
    """Cook at a frame (None keeps the current one)."""
    if frame_ is None and fps_ is None:
        yield
        return
    cur = _STATE.get()
    token = _STATE.set((float(cur[0] if frame_ is None else frame_), float(cur[1] if fps_ is None else fps_)))
    try:
        yield
    finally:
        _STATE.reset(token)


def key() -> str:
    """Cache-key material for time-dependent nodes."""
    f, rate = _STATE.get()
    return f"t{f!r}@{rate!r}/{_STEP.get()!r}"


def params_use_time(params: dict) -> bool:
    """True if a string parameter mentions @Frame/@Time/@TimeInc, or the node says it is ``animated`` (the python node)."""
    return params.get("animated") is True or any(isinstance(v, str) and USES_TIME.search(v) for v in params.values())
