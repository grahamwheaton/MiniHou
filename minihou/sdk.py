"""Node SDK: declare a node as a plain function plus a decorator.

    from minihou.sdk import node, Float, Vec3

    @node("noise", inputs=["geo"], params=[
        Float("frequency", 1.0, min=0.0, label="Frequency"),
        Vec3("amplitude", (0, 0, 1)),
    ])
    def noise(geo, frequency, amplitude):
        out = geo.copy()
        ...
        return out

Contract
--------
* The function receives one positional argument per declared input, in order
  (a ``list`` of geometries for ``multi=True`` inputs, ``None`` for an unconnected
  optional input) and one keyword argument per declared parameter.
* It returns a new :class:`~minihou.geometry.Geometry`. Inputs are frozen: copy before
  modifying (``geo.copy()`` is shallow, so untouched arrays stay shared).
* It must be a pure function of (inputs, params). The evaluator caches by hash of
  those, so hidden state (time, files, global RNG) would make the cache lie. Bump
  ``version=`` whenever you change a node's behaviour.

A future wrangle node fits this contract unchanged: one ``Str("snippet", kind="code")``
parameter (which is hashed into the cache key), one geometry input, and a function
that compiles the snippet (memoised on the source string) and runs it over the
attribute arrays.
"""
from __future__ import annotations

import inspect
import math
import numbers
import re
from dataclasses import dataclass, field
from typing import Any, Callable, Dict, List, Optional, Sequence, Tuple, Union

GEOMETRY = "geometry"

_NAME_RE = re.compile(r"^[A-Za-z_][A-Za-z0-9_]*$")


class ParamError(ValueError):
    """A parameter value is invalid for its declaration."""


# ---------------------------------------------------------------------------
# Ports
# ---------------------------------------------------------------------------
@dataclass(frozen=True)
class Input:
    name: str
    type: str = GEOMETRY
    multi: bool = False       # accepts any number of connections; function receives a list
    optional: bool = False
    doc: str = ""


@dataclass(frozen=True)
class Output:
    name: str = "geo"
    type: str = GEOMETRY
    doc: str = ""


# ---------------------------------------------------------------------------
# Parameters
# ---------------------------------------------------------------------------
@dataclass(frozen=True)
class Param:
    """Declaration of a node parameter.

    ``min``/``max`` are hard limits (violations raise :class:`ParamError`).
    ``soft_min``/``soft_max`` are UI slider hints only.
    """

    name: str
    kind: str                       # float | int | bool | str | vec3 | enum
    default: Any
    min: Optional[float] = None
    max: Optional[float] = None
    soft_min: Optional[float] = None
    soft_max: Optional[float] = None
    label: str = ""
    doc: str = ""
    choices: Tuple[str, ...] = ()
    ui: str = ""                    # free-form hint, e.g. "code" for expression/snippet text

    def __post_init__(self):
        if not _NAME_RE.match(self.name):
            raise ValueError(f"invalid parameter name {self.name!r}")
        object.__setattr__(self, "default", self.coerce(self.default))

    @property
    def display_label(self) -> str:
        return self.label or self.name.replace("_", " ").title()

    def coerce(self, value: Any) -> Any:
        """Validate ``value`` and return its canonical (JSON-friendly) form."""
        k = self.kind
        try:
            if k == "float":
                v = self._num(value)
                return self._check_range(v)
            if k == "int":
                if isinstance(value, bool):
                    raise ParamError("expected an int, got bool")
                if isinstance(value, float):
                    if not value.is_integer():
                        raise ParamError(f"expected an int, got {value}")
                    value = int(value)
                if not isinstance(value, numbers.Integral):
                    raise ParamError(f"expected an int, got {type(value).__name__}")
                return int(self._check_range(int(value)))
            if k == "bool":
                if isinstance(value, bool):
                    return value
                if isinstance(value, int) and value in (0, 1):
                    return bool(value)
                raise ParamError(f"expected a bool, got {value!r}")
            if k == "str":
                if not isinstance(value, str):
                    raise ParamError(f"expected a string, got {type(value).__name__}")
                return value
            if k == "vec3":
                if isinstance(value, (str, bytes)) or not hasattr(value, "__len__") or len(value) != 3:
                    raise ParamError(f"expected 3 numbers, got {value!r}")
                return tuple(self._check_range(self._num(c)) for c in value)
            if k == "enum":
                if value not in self.choices:
                    raise ParamError(f"expected one of {list(self.choices)}, got {value!r}")
                return value
        except ParamError as e:
            raise ParamError(f"parameter {self.name!r}: {e}") from None
        raise ParamError(f"parameter {self.name!r}: unknown kind {k!r}")  # pragma: no cover

    @staticmethod
    def _num(value: Any) -> float:
        if isinstance(value, bool) or not hasattr(value, "__float__"):
            raise ParamError(f"expected a number, got {value!r}")
        v = float(value)
        if not math.isfinite(v):
            raise ParamError(f"expected a finite number, got {v}")
        return v

    def _check_range(self, v):
        if self.min is not None and v < self.min:
            raise ParamError(f"{v} is below minimum {self.min}")
        if self.max is not None and v > self.max:
            raise ParamError(f"{v} is above maximum {self.max}")
        return v

    def to_json_value(self, v: Any) -> Any:
        return list(v) if self.kind == "vec3" else v

    def describe(self) -> dict:
        d = {"name": self.name, "kind": self.kind, "default": self.to_json_value(self.default),
             "label": self.display_label}
        for key in ("min", "max", "soft_min", "soft_max"):
            if getattr(self, key) is not None:
                d[key] = getattr(self, key)
        if self.choices:
            d["choices"] = list(self.choices)
        if self.ui:
            d["ui"] = self.ui
        if self.doc:
            d["doc"] = self.doc
        return d


def Float(name, default=0.0, *, min=None, max=None, soft_min=None, soft_max=None, label="", doc="", ui=""):
    return Param(name, "float", default, min, max, soft_min, soft_max, label, doc, (), ui)


def Int(name, default=0, *, min=None, max=None, soft_min=None, soft_max=None, label="", doc="", ui=""):
    return Param(name, "int", default, min, max, soft_min, soft_max, label, doc, (), ui)


def Bool(name, default=False, *, label="", doc="", ui=""):
    return Param(name, "bool", default, label=label, doc=doc, ui=ui)


def Str(name, default="", *, kind="", label="", doc=""):
    """String parameter. ``kind="code"`` hints editors to show a code box (wrangle snippets)."""
    return Param(name, "str", default, label=label, doc=doc, ui=kind)


def Vec3(name, default=(0.0, 0.0, 0.0), *, min=None, max=None, label="", doc=""):
    return Param(name, "vec3", default, min, max, label=label, doc=doc)


def Enum(name, choices: Sequence[str], default=None, *, label="", doc=""):
    choices = tuple(choices)
    if not choices:
        raise ValueError("Enum needs at least one choice")
    return Param(name, "enum", choices[0] if default is None else default, label=label, doc=doc, choices=choices)


# ---------------------------------------------------------------------------
# Node types and registry
# ---------------------------------------------------------------------------
@dataclass(frozen=True)
class NodeType:
    name: str
    fn: Callable
    inputs: Tuple[Input, ...]
    params: Tuple[Param, ...]
    outputs: Tuple[Output, ...]
    version: int = 1
    category: str = ""
    doc: str = ""
    _param_index: Dict[str, Param] = field(default_factory=dict, repr=False, compare=False)

    def __post_init__(self):
        object.__setattr__(self, "_param_index", {p.name: p for p in self.params})

    def param(self, name: str) -> Param:
        try:
            return self._param_index[name]
        except KeyError:
            raise ParamError(f"node type {self.name!r} has no parameter {name!r}") from None

    def input_names(self) -> List[str]:
        return [i.name for i in self.inputs]

    def default_params(self) -> Dict[str, Any]:
        return {p.name: p.default for p in self.params}

    def coerce_params(self, given: Optional[Dict[str, Any]]) -> Dict[str, Any]:
        """Fill defaults, validate, and return params in declaration order."""
        given = dict(given or {})
        unknown = set(given) - set(self._param_index)
        if unknown:
            raise ParamError(f"node type {self.name!r} has no parameter(s) {sorted(unknown)}")
        return {p.name: (p.coerce(given[p.name]) if p.name in given else p.default) for p in self.params}

    def describe(self) -> dict:
        return {"name": self.name, "version": self.version, "category": self.category, "doc": self.doc,
                "inputs": [i.__dict__ for i in self.inputs],
                "outputs": [o.__dict__ for o in self.outputs],
                "params": [p.describe() for p in self.params]}


class Registry:
    def __init__(self):
        self._types: Dict[str, NodeType] = {}

    def register(self, nt: NodeType, *, replace: bool = False) -> NodeType:
        if nt.name in self._types and not replace:
            raise ValueError(f"node type {nt.name!r} is already registered")
        self._types[nt.name] = nt
        return nt

    def get(self, name: str) -> NodeType:
        try:
            return self._types[name]
        except KeyError:
            raise KeyError(f"unknown node type {name!r}; known: {sorted(self._types)}") from None

    def __contains__(self, name: str) -> bool:
        return name in self._types

    def names(self) -> List[str]:
        return sorted(self._types)

    def __iter__(self):
        return iter(self._types.values())


REGISTRY = Registry()


def node(name: str, *, inputs: Sequence[Union[str, Input]] = (), params: Sequence[Param] = (),
         outputs: Optional[Sequence[Union[str, Output]]] = None, version: int = 1, category: str = "",
         doc: str = "", registry: Optional[Registry] = None, replace: bool = False):
    """Decorator: turn a function into a registered node type."""
    ins = tuple(Input(i) if isinstance(i, str) else i for i in inputs)
    outs = tuple(Output(o) if isinstance(o, str) else o for o in (outputs if outputs is not None else ["geo"]))
    params = tuple(params)

    def deco(fn: Callable) -> Callable:
        if not _NAME_RE.match(name):
            raise ValueError(f"invalid node type name {name!r}")
        if len(outs) != 1:
            raise ValueError(f"node {name!r}: exactly one output port is supported for now")
        names = [i.name for i in ins] + [p.name for p in params]
        if len(set(names)) != len(names):
            raise ValueError(f"node {name!r}: duplicate input/parameter names in {names}")
        sig = inspect.signature(fn)
        has_var_kw = any(p.kind is inspect.Parameter.VAR_KEYWORD for p in sig.parameters.values())
        if not has_var_kw:
            missing = [n for n in names if n not in sig.parameters]
            if missing:
                raise ValueError(f"node {name!r}: function {fn.__name__} lacks argument(s) {missing}")
        nt = NodeType(name=name, fn=fn, inputs=ins, params=params, outputs=outs, version=version,
                      category=category, doc=doc or (inspect.getdoc(fn) or ""))
        (registry or REGISTRY).register(nt, replace=replace)
        fn.node_type = nt  # type: ignore[attr-defined]
        return fn

    return deco
