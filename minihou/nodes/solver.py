"""Solver: feed each frame's result into the next (Houdini's Solver SOP) -- simulations and growth over time."""
import json
import threading
import weakref
from collections import OrderedDict

from .. import timectx
from ..sdk import REGISTRY, Int, Str, node

MAX_FRAMES = 1000          # frames remembered per solver input (playback steps from the nearest one)
_MEMO: "weakref.WeakKeyDictionary" = weakref.WeakKeyDictionary()   # input geometry -> {settings: OrderedDict}
_LOCK = threading.Lock()


def _body(params):
    nt = REGISTRY.get(params["body"]) if params.get("body") in REGISTRY else None
    if nt is None:
        raise ValueError(f"no node type {params.get('body')!r} for the solver body")
    if not nt.inputs:
        raise ValueError(f"{params['body']!r} has no geometry input to feed the previous frame into")
    return nt


def _key(params) -> str:
    nt = REGISTRY.get(params["body"]) if params.get("body") in REGISTRY else None
    body = f"{nt.name}:{nt.version}:{nt.code_hash}" if nt is not None else "missing"
    return body + "|" + timectx.key()                     # a solver always depends on the frame


@node("solver", inputs=["geo"], category="time", key_extra=_key, params=[
    Str("body", "", doc="Node type applied once per step to the previous result: a system or e.g. 'wrangle'"),
    Str("params", "{}", doc='Parameters for the body as JSON, e.g. {"snippet": "@P.z += @TimeInc"}'),
    Int("start_frame", 1, doc="At and before this frame the solver outputs its input unchanged"),
    Int("substeps", 1, min=1, max=100, doc="Steps per frame (@TimeInc shrinks accordingly)"),
])
def solver(geo, body, params, start_frame, substeps):
    """Run 'body' on the previous frame's result, frame after frame, from start_frame to the current frame (Houdini's
    Solver). Deterministic: any frame gives the same result however you got there; playback is incremental (recent
    frames are remembered), jumping ahead steps from the nearest remembered frame, and going back before it restarts
    from the input. Inside the body, @Frame is the frame being solved and @TimeInc the step in seconds."""
    nt = _body({"body": body})
    try:
        overrides = json.loads(params or "{}")
    except ValueError as e:
        raise ValueError(f"params must be JSON: {e}") from None
    values = nt.coerce_params(overrides if isinstance(overrides, dict) else {})
    cur = timectx.frame()
    target = int(round(cur))
    if target <= start_frame:
        return geo
    settings = json.dumps([body, nt.code_hash, nt.version, sorted(values.items()), start_frame, substeps,
                           timectx.fps()], default=str)
    with _LOCK:
        per_input = _MEMO.setdefault(geo, {})
        frames = per_input.setdefault(settings, OrderedDict())
        start, state = start_frame, geo
        known = [f for f in frames if f <= target]
        if known:
            start = max(known)
            state = frames[start]
    dt = 1.0 / (timectx.fps() * substeps)
    extra = [None] * (len(nt.inputs) - 1)
    for f in range(start + 1, target + 1):
        for k in range(substeps):
            sub = f - 1 + (k + 1) / substeps
            with timectx.at(sub), timectx.step(dt):
                state = nt.fn(state, *extra, **values)
        state = state.freeze() if hasattr(state, "freeze") and not state.frozen else state
        with _LOCK:
            frames[f] = state
            while len(frames) > MAX_FRAMES:
                frames.popitem(last=False)
    return state
