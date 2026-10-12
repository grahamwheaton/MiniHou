"""The python node: a few lines of Python inside the graph, with geometry and variables piped in.

Unlike a wrangle (a sandboxed expression language) this runs *real Python* with numpy, so it can do anything a node library
file can, without leaving the editor. Because a graph file can then carry code that runs on your machine, python nodes are
**off by default**: start the engine with ``--allow-python`` (or set ``MINIHOU_PYTHON_NODES=1``) for graphs you trust.
Nothing a graph says can turn it on.
"""
from __future__ import annotations

import ast
import os
import traceback

import numpy as np

from .. import groups, timectx
from ..geometry import Geometry
from ..sdk import Bool, Float, Input, Int, Str, Vec3, node

_ENABLED = None          # set by set_allowed(); None: ask the environment
_TRUE = ("1", "true", "yes", "on")
RESERVED = {"geo", "input1", "input2", "input3", "inputs", "out", "np", "Geometry", "groups", "Frame", "Time", "TimeInc",
            "a", "b", "c", "d", "i", "j", "v", "s", "flag"}

DEFAULT_CODE = '''# geo, input1, input2, input3: Geometry (or None when nothing is wired). They are read-only: copy before changing.
# Variables from the parameters: a b c d (floats)  i j (ints)  v (x, y, z)  s (text)  flag (bool), and any `name = value`
# lines of the Variables field. Frame, Time, TimeInc give the time (tick "animated" so the node recooks on every frame).
# Set `out` to the result Geometry; leave it out to pass `geo` through.
P = geo.P.astype(np.float64)
P[:, 2] += a * np.sin(P[:, 0] * b)
out = geo.copy()
out.point["P"] = P.astype(np.float32)
'''


def allowed() -> bool:
    if _ENABLED is not None:
        return _ENABLED
    return os.environ.get("MINIHOU_PYTHON_NODES", "").strip().lower() in _TRUE


def set_allowed(value) -> None:
    """The engine's switch (``--allow-python``). Graphs cannot reach it."""
    global _ENABLED
    _ENABLED = None if value is None else bool(value)


def parse_variables(text: str) -> dict:
    """``name = value`` lines (value: a Python literal: number, text, list, dict, True/None...) -> {name: value}."""
    out = {}
    try:
        tree = ast.parse(str(text or ""))
    except SyntaxError as e:
        raise ValueError(f"Variables line {e.lineno}: write 'name = value' ({e.msg})") from None
    for stmt in tree.body:
        if not (isinstance(stmt, ast.Assign) and len(stmt.targets) == 1 and isinstance(stmt.targets[0], ast.Name)):
            raise ValueError(f"Variables line {stmt.lineno}: write 'name = value'")
        name = stmt.targets[0].id
        if name in RESERVED:
            raise ValueError(f"Variables line {stmt.lineno}: {name!r} is already a variable of the python node")
        try:
            out[name] = ast.literal_eval(stmt.value)
        except (ValueError, SyntaxError):
            raise ValueError(f"Variables line {stmt.lineno}: the value of {name!r} must be a number, text, list, dict, "
                             f"True/False or None") from None
    return out


def _fail(code_name: str, e: BaseException) -> ValueError:
    """A readable error naming the line of the node's own code."""
    line = None
    if isinstance(e, SyntaxError):
        line = e.lineno
    else:
        for fs in reversed(traceback.extract_tb(e.__traceback__)):
            if fs.filename == code_name:
                line = fs.lineno
                break
    hint = ""
    if "read-only" in str(e):
        hint = " (the input geometry is read-only: use geo.copy() and P = geo.P.copy() before changing it)"
    return ValueError(f"python node{f', line {line}' if line else ''}: {type(e).__name__}: {e}{hint}")


@node("python", inputs=[Input("geo", optional=True), Input("input1", optional=True), Input("input2", optional=True),
                        Input("input3", optional=True)], category="modify", version=1, params=[
    Str("code", DEFAULT_CODE, kind="code", doc="Python run on every cook. See the comment at the top of the default code."),
    Float("a", 0.0, soft_min=-10.0, soft_max=10.0, doc="Variable a (float): sliders you can expose to Blender"),
    Float("b", 1.0, soft_min=-10.0, soft_max=10.0, doc="Variable b (float)"),
    Float("c", 0.0, soft_min=-10.0, soft_max=10.0, doc="Variable c (float)"),
    Float("d", 0.0, soft_min=-10.0, soft_max=10.0, doc="Variable d (float)"),
    Int("i", 0, soft_min=0, soft_max=100, doc="Variable i (int)"),
    Int("j", 0, soft_min=0, soft_max=100, doc="Variable j (int)"),
    Vec3("v", (0.0, 0.0, 0.0), doc="Variable v: a tuple (x, y, z)"),
    Str("s", "", doc="Variable s: text"),
    Bool("flag", False, doc="Variable flag: a bool"),
    Str("variables", "", doc="Named values, one `name = value` per line (numbers, text, lists, True/False): "
                             "e.g. radius = 2.5 or names = ['a', 'b']"),
    Bool("animated", False, doc="Recook when the frame changes (the code reads Frame, Time or TimeInc)"),
])
def python(geo=None, input1=None, input2=None, input3=None, *, code, a, b, c, d, i, j, v, s, flag, variables, animated):
    """Run Python on the geometry you wire in (numpy as np, Geometry, groups). Variables a b c d i j v s flag, any named
    values from Variables, Frame/Time/TimeInc and the inputs geo, input1..3 are in scope; set `out` to the result (a
    Geometry, or an (n, 3) array of points), or leave it out to pass `geo` through. Inputs are read-only: copy before
    editing. Disabled unless the engine runs with --allow-python (or MINIHOU_PYTHON_NODES=1), because a graph file can then
    run code; treat the node as a pure function of its inputs and parameters (results are cached)."""
    if not allowed():
        raise ValueError("python nodes are switched off. Start the engine with --allow-python (or set MINIHOU_PYTHON_NODES=1) "
                         "to run them: they execute the code stored in the graph, so enable them only for graphs you trust")
    ns = {"np": np, "Geometry": Geometry, "groups": groups, "geo": geo, "input1": input1, "input2": input2,
          "input3": input3, "inputs": [geo, input1, input2, input3], "a": a, "b": b, "c": c, "d": d, "i": i, "j": j,
          "v": tuple(v), "s": s, "flag": flag, "Frame": timectx.frame(), "Time": timectx.seconds(),
          "TimeInc": timectx.time_inc(), "out": None}
    ns.update(parse_variables(variables))
    name = "<python node>"
    try:
        compiled = compile(code, name, "exec")
    except SyntaxError as e:
        raise _fail(name, e) from None
    try:
        exec(compiled, ns)                                   # noqa: S102 - the point of the node; gated by allowed()
    except ValueError as e:
        raise _fail(name, e) from None
    except Exception as e:  # noqa: BLE001 - user code: report it with its line, whatever it raised
        raise _fail(name, e) from None
    result = ns.get("out")
    if result is None:
        if geo is None:
            raise ValueError("python node: nothing wired in and `out` was not set")
        return geo
    if isinstance(result, Geometry):
        return result
    try:
        arr = np.asarray(result, np.float32)
    except (TypeError, ValueError):
        arr = None
    if arr is not None and arr.ndim == 2 and arr.shape[1] == 3:
        return Geometry.points_only(arr)
    raise ValueError(f"python node: `out` must be a Geometry or an (n, 3) array of points, not {type(result).__name__}")
