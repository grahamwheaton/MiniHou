"""Blend: morph a shape towards other shapes with the same points (Houdini's Blend Shapes, Blender's shape keys)."""
import ast
import math
import re

import numpy as np

from .. import groups, timectx
from ..sdk import Float, Input, Str, node

_FUNCS = {"sin": math.sin, "cos": math.cos, "abs": abs, "min": min, "max": max, "sqrt": math.sqrt,
          "clamp": lambda x, lo, hi: max(lo, min(hi, x)),
          "smoothstep": lambda lo, hi, x: (lambda t: t * t * (3 - 2 * t))(max(0.0, min(1.0, (x - lo) / (hi - lo) if hi != lo else 0.0))),
          "fit": lambda x, a, b, c, d: c + (d - c) * ((x - a) / (b - a) if b != a else 0.0)}
_ARITH = {ast.Add: lambda a, b: a + b, ast.Sub: lambda a, b: a - b, ast.Mult: lambda a, b: a * b, ast.Div: lambda a, b: a / b,
          ast.Mod: lambda a, b: a % b, ast.Pow: lambda a, b: a ** b}


def eval_weight(text: str) -> float:
    """A number or a small expression: ``0.5``, ``sin(@Time * 3) * 0.5 + 0.5``. @Frame, @Time and @TimeInc are the
    current time (a weight that mentions them is recooked as the timeline moves); functions: sin cos abs min max sqrt clamp
    smoothstep fit; constant pi."""
    names = {"Frame": timectx.frame(), "Time": timectx.seconds(), "TimeInc": timectx.time_inc(), "pi": math.pi}
    src = re.sub(r"@(\w+)", r"\1", text.strip())
    try:
        tree = ast.parse(src, mode="eval")
    except SyntaxError as e:
        raise ValueError(f"weight {text!r}: {e.msg}") from None

    def ev(n):
        if isinstance(n, ast.Expression):
            return ev(n.body)
        if isinstance(n, ast.Constant) and isinstance(n.value, (int, float)) and not isinstance(n.value, bool):
            return float(n.value)
        if isinstance(n, ast.Name):
            if n.id not in names:
                raise ValueError(f"weight {text!r}: unknown name {n.id!r} (use @Frame, @Time, @TimeInc or pi)")
            return float(names[n.id])
        if isinstance(n, ast.UnaryOp) and isinstance(n.op, (ast.USub, ast.UAdd)):
            return -ev(n.operand) if isinstance(n.op, ast.USub) else ev(n.operand)
        if isinstance(n, ast.BinOp) and type(n.op) in _ARITH:
            a, b = ev(n.left), ev(n.right)
            try:
                return float(_ARITH[type(n.op)](a, b))
            except (ZeroDivisionError, OverflowError, ValueError):
                raise ValueError(f"weight {text!r}: cannot compute {a} and {b}") from None
        if isinstance(n, ast.Call) and isinstance(n.func, ast.Name) and n.func.id in _FUNCS and not n.keywords:
            return float(_FUNCS[n.func.id](*[ev(x) for x in n.args]))
        raise ValueError(f"weight {text!r}: only numbers, + - * / % **, @Frame/@Time/@TimeInc and the functions {', '.join(_FUNCS)} are allowed")

    try:
        return ev(tree)
    except TypeError:
        raise ValueError(f"weight {text!r}: wrong number of arguments to a function") from None


def _weights(text: str, count: int):
    parts = [p for p in (text or "").split(";") if p.strip()] or ["1"]
    w = [eval_weight(p) for p in parts]
    if len(w) == 1:
        w = w * count                                                  # one weight: the same for every target
    elif len(w) != count:
        raise ValueError(f"{len(w)} weights for {count} targets (give one, or one per target separated by ';')")
    return w


@node("blend", inputs=[Input("geo"), Input("targets", multi=True, optional=True)], category="deform", params=[
    Float("amount", 1.0, soft_min=0.0, soft_max=1.0, doc="Overall strength: 0 leaves the base shape, 1 reaches the target (more or "
                                                         "less than that overshoots or inverts)"),
    Str("weights", "1", doc="Weight of each target: one for all, or one per target separated by ';'. Numbers or expressions, e.g. "
                            "'sin(@Time * 3) * 0.5 + 0.5' (@Frame, @Time, @TimeInc; functions sin cos abs min max sqrt clamp "
                            "smoothstep fit)"),
    Str("attributes", "P", doc="Point attributes to blend, space separated; '*' for every float attribute the targets share with the base"),
    Str("mask", "", doc="A point attribute (0..1) that scales the blend per point, e.g. painted or noise-driven (empty: none)"),
    Str("group", "", doc="Only blend these points (empty: all)"),
])
def blend(geo, targets, amount, weights, attributes, mask, group):
    """Morph the base shape towards one or more targets that have the same number of points (Houdini's Blend Shapes /
    Blender's shape keys). Every blended attribute becomes ``base + amount * sum(weight_i * (target_i - base))``, so with
    a single target and weight 1 the amount runs from the base (0) to the target (1), and several targets add their offsets.
    Normals follow when ``N`` is listed; topology and everything else comes from the base."""
    if not targets:
        return geo
    w = [x * amount for x in _weights(weights, len(targets))]
    for i, t in enumerate(targets):
        if t.npoints != geo.npoints:
            raise ValueError(f"target {i + 1} has {t.npoints:,} points but the base has {geo.npoints:,}: blending needs the same points")
    names = attributes.split()
    if "*" in names:
        names = [n for n in geo.point.names() if geo.point[n].dtype.kind == "f" and all(n in t.point and t.point[n].shape == geo.point[n].shape for t in targets)]
    for n in names:
        if n not in geo.point:
            raise ValueError(f"the base has no point attribute {n!r}")
        for i, t in enumerate(targets):
            if n not in t.point or t.point[n].shape != geo.point[n].shape:
                raise ValueError(f"target {i + 1} lacks point attribute {n!r} with the base's shape")
        if geo.point[n].dtype.kind != "f":
            raise ValueError(f"point attribute {n!r} is not a float attribute; only floats can be blended")
    per_point = None
    if mask.strip():
        if mask.strip() not in geo.point:
            raise ValueError(f"the base has no point attribute {mask.strip()!r} to use as a mask")
        per_point = geo.point[mask.strip()].astype(np.float64)
        if per_point.ndim != 1:
            raise ValueError(f"mask attribute {mask.strip()!r} must be a single number per point")
    out = geo.copy()
    for n in names:
        base = geo.point[n].astype(np.float64)
        delta = sum(wi * (t.point[n].astype(np.float64) - base) for wi, t in zip(w, targets))
        if per_point is not None:
            delta = delta * per_point.reshape((-1,) + (1,) * (base.ndim - 1))
        out.point[n] = (base + delta).astype(np.float32)
    return groups.restrict(geo, out, "point", group)


