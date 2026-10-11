"""Wrangle snippets: a small, safe, VEX-flavoured expression language compiled to numpy.

Examples (run over points)::

    @P.z += sin(@P.x * 2) * 0.5
    v@Cd = set(@P.x, 0, 1 - @P.x)
    f@mask = 1 if @P.z > 0 else 0
    h = noise(@P * 0.3); @P += @N * h        # locals, several statements

Language
--------
* ``@name`` reads/writes a point attribute. Prefixes create new attributes with a type: ``f@`` float, ``v@`` vector,
  ``i@`` int. ``@P.x``/``.y``/``.z`` (also ``.r .g .b`` and ``[0]``-free) address vector components.
* ``@ptnum`` (point index) and ``@numpt`` (point count) are read-only.
* Operators: ``+ - * / // % **``, comparisons, ``and or not``, ``a if cond else b``; assignment and ``+= -= *= /=``.
* Functions: see :data:`FUNCTIONS` (``sin``, ``fit``, ``clamp``, ``length``, ``normalize``, ``dot``, ``cross``, ``noise``, ``rand`` ...).
* A float times a vector works (per-point scalars are kept as ``(n, 1)`` columns internally).

Safety
------
Snippets are parsed with :mod:`ast` and must consist only of whitelisted node types: numbers, names of locals or
whitelisted functions, attribute references, ``.x/.y/.z`` components, arithmetic, comparisons, conditionals and calls to
whitelisted functions by name. No attribute access, subscripts, strings, lambdas, comprehensions, imports or builtins are
reachable, so the compiled code can only compute on numpy arrays. Violations raise :class:`WrangleError` with the
snippet line and column.
"""
from __future__ import annotations

import ast
import functools
import re
from typing import Callable, Dict, List, Set, Tuple

import numpy as np


class WrangleError(ValueError):
    pass


_COMPONENTS = {"x": 0, "y": 1, "z": 2, "w": 3, "r": 0, "g": 1, "b": 2, "a": 3}
_PREFIX_TYPES = {"f": "float", "v": "vector", "i": "int", "": None}
_ATTR_RE = re.compile(r"(?<![A-Za-z0-9_])([fvi]?)@([A-Za-z_][A-Za-z0-9_]*)")
_MANGLE = "__attr_"
_READONLY = {"ptnum", "numpt"}


# ---------------------------------------------------------------------------
# Runtime functions (all operate on numpy arrays; scalars per point are (n, 1))
# ---------------------------------------------------------------------------
def _col(x):
    a = np.asarray(x)
    return a.reshape(-1, 1) if a.ndim == 1 else a


def _length(v):
    v = np.asarray(v, dtype=np.float32)
    return np.sqrt(np.sum(v * v, axis=-1, keepdims=True))


def _normalize(v):
    v = np.asarray(v, dtype=np.float32)
    n = _length(v)
    return v / np.where(n == 0, np.float32(1), n)


def _set(*parts):
    """Build a vector from scalars / per-point columns: set(1, 0, 0) or set(@P.x, 0, h)."""
    arrs = [np.asarray(p, dtype=np.float32) for p in parts]
    n = next((a.shape[0] for a in arrs if a.ndim >= 1 and a.size > 1 or a.ndim == 2), None)
    if n is None:
        return np.array([float(a.reshape(-1)[0]) for a in arrs], np.float32)
    cols = []
    for a in arrs:
        if a.ndim == 0 or a.size == 1 and a.ndim < 2:
            cols.append(np.full((n, 1), float(a.reshape(-1)[0]), np.float32))
        else:
            cols.append(a.reshape(n, -1))
    return np.concatenate(cols, axis=1)


def _fit(x, omin, omax, nmin, nmax):
    t = (np.asarray(x, np.float32) - omin) / np.where(np.asarray(omax - omin) == 0, 1, omax - omin)
    return nmin + np.clip(t, 0, 1) * (nmax - nmin)


def _noise(p, *rest):
    from .nodes._util import fbm3
    p = np.asarray(p, dtype=np.float32)
    if p.ndim == 1:
        p = p.reshape(1, -1)
    if p.shape[-1] != 3:
        raise WrangleError("noise() takes a vector position")
    seed = int(rest[0]) if rest else 0
    return fbm3(np.ascontiguousarray(p), seed, 1, 2.0, 0.5).reshape(-1, 1)


def _rand(x, seed=0):
    """Deterministic per-element random in [0, 1) from a number or vector (hash, not a stream)."""
    a = np.asarray(x, dtype=np.float64)
    if a.ndim == 2 and a.shape[1] > 1:
        a = a @ np.array([12.9898, 78.233, 37.719, 4.581][: a.shape[1]])
        a = a.reshape(-1, 1)
    v = np.sin(a * 12.9898 + float(seed) * 78.233) * 43758.5453
    return (v - np.floor(v)).astype(np.float32)


FUNCTIONS: Dict[str, Callable] = {
    "sin": np.sin, "cos": np.cos, "tan": np.tan, "asin": np.arcsin, "acos": np.arccos, "atan": np.arctan,
    "atan2": np.arctan2, "sqrt": np.sqrt, "exp": np.exp, "log": np.log, "abs": np.abs, "floor": np.floor,
    "ceil": np.ceil, "frac": lambda x: x - np.floor(x), "pow": np.power, "sign": np.sign,
    "min": np.minimum, "max": np.maximum, "clamp": lambda x, lo, hi: np.clip(x, lo, hi),
    "lerp": lambda a, b, t: a + (b - a) * t, "fit": _fit, "smooth": lambda e0, e1, x: (
        lambda t: t * t * (3 - 2 * t))(np.clip((np.asarray(x, np.float32) - e0) / (e1 - e0), 0, 1)),
    "length": _length, "normalize": _normalize, "dot": lambda a, b: np.sum(np.asarray(a) * np.asarray(b), axis=-1, keepdims=True),
    "cross": lambda a, b: np.cross(a, b), "set": _set, "noise": _noise, "rand": _rand,
    "select": lambda c, a, b: np.where(c, a, b), "radians": np.radians, "degrees": np.degrees,
}
CONSTANTS = {"PI": np.float32(np.pi), "E": np.float32(np.e)}

# One line per function for references shown to people and AI tools (a test keeps this in sync with FUNCTIONS).
SIGNATURES: Dict[str, str] = {
    "sin": "sin(x)", "cos": "cos(x)", "tan": "tan(x)", "asin": "asin(x)", "acos": "acos(x)", "atan": "atan(x)",
    "atan2": "atan2(y, x)", "sqrt": "sqrt(x)", "exp": "exp(x)", "log": "log(x) natural log", "abs": "abs(x)",
    "floor": "floor(x)", "ceil": "ceil(x)", "frac": "frac(x) = x - floor(x)", "pow": "pow(x, y)", "sign": "sign(x)",
    "min": "min(a, b) per component", "max": "max(a, b) per component", "clamp": "clamp(x, lo, hi)",
    "lerp": "lerp(a, b, t) = a + (b - a) * t", "fit": "fit(x, omin, omax, nmin, nmax) remap with clamping",
    "smooth": "smooth(e0, e1, x) smoothstep 0..1", "length": "length(v) -> float", "normalize": "normalize(v) -> unit vector",
    "dot": "dot(a, b) -> float", "cross": "cross(a, b) -> vector", "set": "set(x, y, z) -> vector from floats",
    "noise": "noise(p, seed=0) -> float Perlin noise of a vector position, roughly -1..1",
    "rand": "rand(x, seed=0) -> float in [0, 1), deterministic hash of a number or vector (e.g. rand(@ptnum))",
    "select": "select(cond, a, b) per point", "radians": "radians(deg)", "degrees": "degrees(rad)",
}


def reference() -> str:
    """The snippet language in one page (for the editor's help and for AI tools)."""
    lang = __doc__.split("Language\n--------\n", 1)[1].split("Safety\n------", 1)[0].strip()
    funcs = "\n".join(f"* {SIGNATURES[name]}" for name in sorted(FUNCTIONS))
    examples = __doc__.split("Examples (run over points)::", 1)[1].split("Language\n", 1)[0].strip("\n")
    return ("Wrangle snippets run once over all points (vectorised; no loops needed or allowed).\n\nExamples:\n"
            f"{examples}\n\n{lang}\n\nFunctions:\n{funcs}\n\nConstants: {', '.join(sorted(CONSTANTS))}\n\n"
            "Statements are separated by newlines or ';'; '#' starts a comment. Errors name the line and column.")

_BINOPS = (ast.Add, ast.Sub, ast.Mult, ast.Div, ast.FloorDiv, ast.Mod, ast.Pow)
_CMPOPS = (ast.Eq, ast.NotEq, ast.Lt, ast.LtE, ast.Gt, ast.GtE)


# ---------------------------------------------------------------------------
# Compilation
# ---------------------------------------------------------------------------
class Program:
    """A compiled snippet: which attributes it reads/writes, and the function that runs it."""

    def __init__(self, source: str, reads: Set[str], writes: Dict[str, str], fn: Callable):
        self.source, self.reads, self.writes, self.fn = source, reads, writes, fn

    def run(self, attrs: Dict[str, np.ndarray], n: int) -> Dict[str, np.ndarray]:
        """Execute over ``n`` elements. ``attrs`` maps name -> array; returns the written attributes (new arrays)."""
        missing = sorted(self.reads - set(attrs) - set(self.writes))
        if missing:
            raise WrangleError(f"unknown attribute(s) {', '.join('@' + m for m in missing)}; "
                               f"available: {', '.join('@' + a for a in sorted(attrs)) or 'none'}")
        env: Dict[str, np.ndarray] = {}
        kinds: Dict[str, str] = {}
        for name, declared in self.writes.items():
            if name in attrs:
                a = np.asarray(attrs[name])
                kinds[name] = declared or _kind_of(a)
                if declared and declared != _kind_of(a):
                    raise WrangleError(f"@{name} exists as {_kind_of(a)}, not {declared}")
            else:
                kinds[name] = declared                      # '' = infer from the assigned value
                if declared:
                    env[name] = np.zeros((n, 3) if declared == "vector" else (n, 1),
                                         np.int32 if declared == "int" else np.float32)
        for name in self.reads | set(self.writes):
            if name in attrs:
                a = np.asarray(attrs[name])
                a = a.astype(np.float32) if a.dtype.kind == "f" else a
                env[name] = _col(a).copy() if name in self.writes else _col(a)
        env["ptnum"] = np.arange(n, dtype=np.int32).reshape(-1, 1)
        env["numpt"] = np.int32(n)
        try:
            with np.errstate(all="ignore"):
                self.fn(env, n)
        except WrangleError:
            raise
        except KeyError as e:
            raise WrangleError(f"@{e.args[0]} is read before it is assigned") from None
        except Exception as e:  # noqa: BLE001 - shape mismatches etc. become user errors
            raise WrangleError(f"runtime error: {type(e).__name__}: {e}") from None
        out = {}
        for name in self.writes:
            v = np.asarray(env[name])
            if v.ndim == 0:
                v = v.reshape(1, 1)
            if v.ndim == 1:
                v = v.reshape(1, -1) if v.shape[0] in (3, 4) and kinds[name] == "vector" else v.reshape(-1, 1)
            if v.shape[0] != n:
                v = np.broadcast_to(v, (n, v.shape[1]))
            kind = kinds[name] or ("vector" if v.shape[1] == 3 else "int" if v.dtype.kind in "iu" else "float")
            if kind == "vector":
                if v.shape[1] == 1:
                    v = np.broadcast_to(v, (n, 3))
                if v.shape[1] != 3:
                    raise WrangleError(f"@{name} is a vector but the snippet assigned {v.shape[1]} components")
                v = v.astype(np.float32)
            else:
                if v.shape[1] != 1:
                    raise WrangleError(f"@{name} is a {kind} but the snippet assigned a vector")
                v = v[:, 0].astype(np.int32 if kind == "int" else np.float32)
            out[name] = np.ascontiguousarray(v)
        return out


def _kind_of(a: np.ndarray) -> str:
    if a.ndim == 2 and a.shape[1] == 3:
        return "vector"
    if a.ndim == 1:
        return "int" if a.dtype.kind in "iub" else "float"
    return f"{a.shape[1]}-component"


def _err(node, msg) -> WrangleError:
    return WrangleError(f"line {getattr(node, 'lineno', '?')}, column {getattr(node, 'col_offset', 0) + 1}: {msg}")


class _Checker(ast.NodeVisitor):
    """Whitelist validation + collects attribute reads/writes and locals."""

    def __init__(self, declared: Dict[str, str]):
        self.declared = declared          # attr name -> declared kind from prefix ('' if none)
        self.reads: Set[str] = set()
        self.writes: Set[str] = set()
        self.locals: Set[str] = set()

    def generic_visit(self, node):
        raise _err(node, f"'{type(node).__name__}' is not allowed in wrangle snippets")

    def visit_Module(self, node):
        for s in node.body:
            self.visit(s)

    def _target(self, t, aug: bool):
        if isinstance(t, ast.Name):
            if t.id.startswith(_MANGLE):
                name = t.id[len(_MANGLE):]
                if name in _READONLY:
                    raise _err(t, f"@{name} is read-only")
                self.writes.add(name)
                if aug:
                    self.reads.add(name)
            else:
                if t.id in FUNCTIONS or t.id in CONSTANTS:
                    raise _err(t, f"cannot assign to the built-in '{t.id}'")
                if aug and t.id not in self.locals:
                    raise _err(t, f"'{t.id}' is used before it is assigned")
                self.locals.add(t.id)
        elif isinstance(t, ast.Attribute) and isinstance(t.value, ast.Name) and t.value.id.startswith(_MANGLE):
            if t.attr not in _COMPONENTS:
                raise _err(t, f"unknown component '.{t.attr}' (use .x .y .z)")
            name = t.value.id[len(_MANGLE):]
            if name in _READONLY:
                raise _err(t, f"@{name} is read-only")
            self.writes.add(name)
            self.reads.add(name)           # writing one component keeps the others
        else:
            raise _err(t, "can only assign to @attributes, their .x/.y/.z components, or local names")

    def visit_Assign(self, node):
        if len(node.targets) != 1:
            raise _err(node, "chained assignment is not supported")
        self.visit(node.value)
        self._target(node.targets[0], aug=False)

    def visit_AugAssign(self, node):
        if not isinstance(node.op, _BINOPS):
            raise _err(node, "unsupported operator")
        self.visit(node.value)
        self._target(node.target, aug=True)

    def visit_Expr(self, node):
        raise _err(node, "a statement must assign something (e.g. @P.z += 1)")

    def visit_Name(self, node):
        if node.id.startswith(_MANGLE):
            self.reads.add(node.id[len(_MANGLE):])
        elif node.id in CONSTANTS:
            pass
        elif node.id in FUNCTIONS:
            raise _err(node, f"'{node.id}' is a function; call it like {node.id}(...)")
        elif node.id not in self.locals:
            raise _err(node, f"unknown name '{node.id}' (attributes need @, e.g. @{node.id})")

    def visit_Attribute(self, node):
        if not (isinstance(node.value, ast.Name) and node.value.id.startswith(_MANGLE)):
            raise _err(node, "'.' is only allowed for vector components of @attributes (e.g. @P.x)")
        if node.attr not in _COMPONENTS:
            raise _err(node, f"unknown component '.{node.attr}' (use .x .y .z)")
        self.visit(node.value)

    def visit_Constant(self, node):
        if isinstance(node.value, bool) or not isinstance(node.value, (int, float)):
            raise _err(node, "only numbers are allowed as literals")

    def visit_BinOp(self, node):
        if not isinstance(node.op, _BINOPS):
            raise _err(node, "unsupported operator")
        self.visit(node.left)
        self.visit(node.right)

    def visit_UnaryOp(self, node):
        if not isinstance(node.op, (ast.USub, ast.UAdd, ast.Not)):
            raise _err(node, "unsupported operator")
        self.visit(node.operand)

    def visit_BoolOp(self, node):
        for v in node.values:
            self.visit(v)

    def visit_Compare(self, node):
        if not all(isinstance(o, _CMPOPS) for o in node.ops):
            raise _err(node, "unsupported comparison")
        self.visit(node.left)
        for c in node.comparators:
            self.visit(c)

    def visit_IfExp(self, node):
        self.visit(node.test)
        self.visit(node.body)
        self.visit(node.orelse)

    def visit_Call(self, node):
        if not isinstance(node.func, ast.Name) or node.func.id not in FUNCTIONS:
            name = node.func.id if isinstance(node.func, ast.Name) else type(node.func).__name__
            raise _err(node, f"unknown function '{name}'; available: {', '.join(sorted(FUNCTIONS))}")
        if node.keywords:
            raise _err(node, "keyword arguments are not supported")
        for a in node.args:
            if isinstance(a, ast.Starred):
                raise _err(a, "'*' arguments are not supported")
            self.visit(a)


class _Rewriter(ast.NodeTransformer):
    """Turn the validated AST into numpy code over an ``__env`` dict."""

    def _env(self, name, ctx):
        return ast.Subscript(value=ast.Name("__env", ast.Load()), slice=ast.Constant(name), ctx=ctx)

    def visit_Name(self, node):
        if node.id.startswith(_MANGLE):
            return ast.copy_location(self._env(node.id[len(_MANGLE):], node.ctx), node)
        return node

    def visit_Attribute(self, node):
        name = node.value.id[len(_MANGLE):]
        i = _COMPONENTS[node.attr]
        sl = ast.Tuple(elts=[ast.Slice(), ast.Slice(ast.Constant(i), ast.Constant(i + 1))], ctx=ast.Load())
        return ast.copy_location(ast.Subscript(value=self._env(name, ast.Load()), slice=sl, ctx=node.ctx), node)

    def visit_BoolOp(self, node):
        self.generic_visit(node)
        fn = "logical_and" if isinstance(node.op, ast.And) else "logical_or"
        expr = node.values[0]
        for v in node.values[1:]:
            expr = ast.Call(func=ast.Name(f"__np_{fn}", ast.Load()), args=[expr, v], keywords=[])
        return ast.copy_location(expr, node)

    def visit_UnaryOp(self, node):
        self.generic_visit(node)
        if isinstance(node.op, ast.Not):
            return ast.copy_location(ast.Call(func=ast.Name("__np_logical_not", ast.Load()), args=[node.operand],
                                              keywords=[]), node)
        return node

    def visit_Compare(self, node):
        self.generic_visit(node)
        if len(node.ops) == 1:
            return node
        parts, left = [], node.left
        for op, right in zip(node.ops, node.comparators):
            parts.append(ast.Compare(left=left, ops=[op], comparators=[right]))
            left = right
        expr = parts[0]
        for p in parts[1:]:
            expr = ast.Call(func=ast.Name("__np_logical_and", ast.Load()), args=[expr, p], keywords=[])
        return ast.copy_location(expr, node)

    def visit_IfExp(self, node):
        self.generic_visit(node)
        return ast.copy_location(ast.Call(func=ast.Name("__np_where", ast.Load()),
                                          args=[node.test, node.body, node.orelse], keywords=[]), node)

    def visit_AugAssign(self, node):
        self.generic_visit(node)
        if isinstance(node.target, ast.Subscript) and isinstance(node.target.slice, ast.Tuple):
            # component write: env["P"][:, 2:3] = env["P"][:, 2:3] op value  (explicit, broadcasts)
            load = ast.Subscript(value=node.target.value, slice=node.target.slice, ctx=ast.Load())
            return ast.copy_location(ast.Assign(targets=[node.target],
                                                value=ast.BinOp(left=load, op=node.op, right=node.value)), node)
        load = (ast.Subscript(value=node.target.value, slice=node.target.slice, ctx=ast.Load())
                if isinstance(node.target, ast.Subscript) else ast.Name(node.target.id, ast.Load()))
        return ast.copy_location(ast.Assign(targets=[node.target], value=ast.BinOp(left=load, op=node.op, right=node.value)),
                                 node)


@functools.lru_cache(maxsize=256)
def compile_snippet(source: str) -> Program:
    """Validate and compile a snippet (memoised on the source text)."""
    declared: Dict[str, str] = {}

    def mangle(m):
        prefix, name = m.group(1), m.group(2)
        kind = _PREFIX_TYPES[prefix]
        if kind:
            prev = declared.get(name)
            if prev and prev != kind:
                raise WrangleError(f"@{name} is declared both as {prev} and {kind}")
            declared[name] = kind
        return _MANGLE + name

    statements = [line.strip() for line in source.replace(";", "\n").splitlines()]
    text = _ATTR_RE.sub(mangle, "\n".join(line for line in statements if line and not line.startswith("#")))
    if "@" in text:
        raise WrangleError("'@' must be followed by an attribute name, e.g. @P")
    try:
        tree = ast.parse(text, mode="exec")
    except SyntaxError as e:
        raise WrangleError(f"line {e.lineno}, column {e.offset}: syntax error: {e.msg}") from None
    if not tree.body:
        raise WrangleError("empty snippet")
    chk = _Checker(declared)
    chk.visit(tree)
    tree = _Rewriter().visit(tree)
    func = ast.FunctionDef(name="__wrangle", args=ast.arguments(posonlyargs=[], args=[ast.arg("__env"), ast.arg("__n")],
                                                                     kwonlyargs=[], kw_defaults=[], defaults=[]),
                           body=tree.body, decorator_list=[], returns=None, type_params=[])
    mod = ast.Module(body=[func], type_ignores=[])
    ast.fix_missing_locations(mod)
    ns = {"__builtins__": {}, "__np_where": np.where, "__np_logical_and": np.logical_and,
          "__np_logical_or": np.logical_or, "__np_logical_not": np.logical_not}
    ns.update(FUNCTIONS)
    ns.update(CONSTANTS)
    exec(compile(mod, "<wrangle>", "exec"), ns)  # noqa: S102 - AST validated against a whitelist above
    writes = {}
    for name in chk.writes:
        writes[name] = declared.get(name) or ""
    return Program(source, set(chk.reads) - _READONLY - set(), writes, ns["__wrangle"])
