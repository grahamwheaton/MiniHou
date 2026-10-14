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

import contextlib
import inspect
import math
import numbers
import re
import threading
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
    proxy: str = ""                 # how a proxy cook level scales this parameter (see minihou.proxy.KINDS); "" = not at all

    def __post_init__(self):
        if not _NAME_RE.match(self.name):
            raise ValueError(f"invalid parameter name {self.name!r}")
        if self.proxy:
            from .proxy import KINDS
            if self.proxy not in KINDS or self.kind not in ("int", "float"):
                raise ValueError(f"parameter {self.name!r}: proxy must be one of {sorted(KINDS)} on an int or float parameter")
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
        if self.proxy:
            d["proxy"] = self.proxy
        if self.doc:
            d["doc"] = self.doc
        return d


def Float(name, default=0.0, *, min=None, max=None, soft_min=None, soft_max=None, label="", doc="", ui="", proxy=""):
    return Param(name, "float", default, min, max, soft_min, soft_max, label, doc, (), ui, proxy)


def Int(name, default=0, *, min=None, max=None, soft_min=None, soft_max=None, label="", doc="", ui="", proxy=""):
    return Param(name, "int", default, min, max, soft_min, soft_max, label, doc, (), ui, proxy)


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


@dataclass(frozen=True)
class Action:
    """A button on a node (see :mod:`minihou.actions`): ``name`` is what the API calls it, ``label`` what the editor shows."""
    name: str
    label: str = ""
    doc: str = ""

    def __post_init__(self):
        if not _NAME_RE.match(self.name):
            raise ValueError(f"invalid action name {self.name!r}")

    def describe(self) -> dict:
        return {"name": self.name, "label": self.label or self.name.replace("_", " ").title(), "doc": self.doc}


# ---------------------------------------------------------------------------
# Viewport handles (draggable gizmos) declared on a node type
# ---------------------------------------------------------------------------
HANDLE_KINDS = ("arrow", "ring", "scale")
HANDLE_PLACES = ("center", "origin", "top", "bottom")
HANDLE_AXES = {"x": (1.0, 0.0, 0.0), "y": (0.0, 1.0, 0.0), "z": (0.0, 0.0, 1.0)}


@dataclass(frozen=True)
class Handle:
    """A draggable gizmo for one parameter, shown in the editor's 3-D view while the node is selected (cosmetic: not part of the
    cache key). ``kind``: "arrow" (drag along ``axis``: the parameter changes by ``scale`` per world unit), "ring" (a circle
    round ``axis``; for a parameter in degrees) or "scale" (a cube on ``axis``; dragging multiplies the parameter).

    ``axis``: "x" / "y" / "z", "-x"..., a 3-vector, or "@name" for a Vec3 parameter holding a direction. ``at``: where it
    stands: "center" (of the node's output bounds), "origin" (the world origin), "top" / "bottom" (middle of the top or bottom
    of the bounds), a 3-vector, or "@name" for a Vec3 parameter holding a point. ``reach=True`` puts an arrow's root at ``at + axis * value / scale``
    so it follows the dimension it edits (a radius, a length). ``comp`` picks one component of a Vec3 parameter."""
    param: str
    kind: str = "arrow"
    axis: Any = "z"
    at: Any = "center"
    comp: Optional[int] = None
    scale: float = 1.0
    reach: bool = False
    min: Optional[float] = None
    max: Optional[float] = None
    label: str = ""
    id: str = ""
    radius: Optional[float] = None            # rings: world radius (default: from the output's size)
    tool: str = ""                            # "move" | "rotate" | "scale": handles under the same tool buttons as transform's
    when: str = ""                            # show only while these hold: "flag" (a Bool parameter is on), "name=value", "name!=value", joined by "&"

    def key(self) -> str:
        return self.id or self.param + (f"_{'xyz'[self.comp]}" if self.comp is not None else "")


GUIDE_SHAPES = ("box", "sphere", "circle", "plane", "line", "bounds", "axes")


@dataclass(frozen=True)
class Guide:
    """A helper drawn in the 3-D view while the node is selected: an outline of a region the node works on (a bounding box, a
    radius, a cutting plane...). Not draggable and cosmetic (never in the cache key); pair it with ``Handle``s that edit the
    same parameters. ``shape``: "box" (``size`` is the full size), "sphere" (``size`` a radius, or a Vec3 of full sizes for an
    ellipsoid), "circle" (``size`` a radius, round ``axis``), "plane" (a square of side ``size`` with a normal tick), "line"
    (``size`` long along ``axis``), "bounds" (the bounding box of the node's output) or "axes" (three short lines).

    ``at`` is where it stands (as for ``Handle``), ``size`` is a number, "@name" for a parameter holding it, or None (fitted to
    the output). ``when``: "flag" (a Bool parameter is on), "name=value" or "name!=value", joined by "&", for the guide to show.
    ``color``: "x"/"y"/"z" or a hex colour."""
    shape: str
    at: Any = "center"
    axis: Any = "z"
    size: Any = None
    id: str = ""
    label: str = ""
    color: str = ""
    when: str = ""

    def key(self) -> str:
        return self.id or self.shape


def _check_when(where: str, when: str, by: Dict[str, Param]) -> None:
    for term in filter(None, (t.strip() for t in when.split("&"))):
        name = term.split("!=")[0].split("=")[0].strip()
        if name.startswith("_"):                  # a value the viewer adds (e.g. "_inputs": how many inputs are connected)
            continue
        if name not in by:
            raise ValueError(f"{where}: when={when!r} names no parameter {name!r}")
        if "=" not in term and by[name].kind != "bool":
            raise ValueError(f"{where}: when={when!r}: {name!r} is not a Bool parameter (write {name}=value)")


def holds(when: str, params: Dict[str, Any]) -> bool:
    """Whether every term of a ``when`` condition holds for the given parameter values."""
    for term in filter(None, (t.strip() for t in when.split("&"))):
        if "!=" in term:
            name, value = (s.strip() for s in term.split("!=", 1))
            ok = str(params.get(name)) != value
        elif "=" in term:
            name, value = (s.strip() for s in term.split("=", 1))
            ok = str(params.get(name)) == value
        else:
            ok = bool(params.get(term))
        if not ok:
            return False
    return True


def _check_guide(node_name: str, g: "Guide", params: Sequence[Param]) -> None:
    by = {p.name: p for p in params}
    where = f"node {node_name!r} guide {g.key()!r}"
    if g.shape not in GUIDE_SHAPES:
        raise ValueError(f"{where}: shape must be one of {GUIDE_SHAPES}")
    for what, v in (("at", g.at), ("axis", g.axis), ("size", g.size)):
        if isinstance(v, str) and v.startswith("@") and v[1:] not in by:
            raise ValueError(f"{where}: {what} {v!r} names no parameter")
    _check_when(where, g.when, by)


def _check_handle(node_name: str, h: "Handle", params: Sequence[Param]) -> None:
    by = {p.name: p for p in params}
    where = f"node {node_name!r} handle {h.key()!r}"
    _check_when(where, h.when, by)
    if h.param not in by:
        raise ValueError(f"{where}: no parameter {h.param!r}")
    p = by[h.param]
    if h.kind not in HANDLE_KINDS:
        raise ValueError(f"{where}: kind must be one of {HANDLE_KINDS}")
    if p.kind not in ("float", "int", "vec3"):
        raise ValueError(f"{where}: {h.param!r} is a {p.kind} parameter; handles edit numbers and vectors")
    if p.kind == "vec3" and h.comp not in (0, 1, 2):
        raise ValueError(f"{where}: {h.param!r} is a vector: say which component (comp=0, 1 or 2)")
    if p.kind != "vec3" and h.comp is not None:
        raise ValueError(f"{where}: comp only applies to vector parameters")
    for what, v in (("axis", h.axis), ("at", h.at)):
        if isinstance(v, str):
            if v.startswith("@"):
                if v[1:] not in by or by[v[1:]].kind != "vec3":
                    raise ValueError(f"{where}: {what} {v!r} must name a Vec3 parameter")
            elif not ((what == "axis" and v.lstrip("-") in HANDLE_AXES) or (what == "at" and v in HANDLE_PLACES)):
                raise ValueError(f"{where}: {what} {v!r} is not a keyword ({'x, y, z, -x...' if what == 'axis' else ', '.join(HANDLE_PLACES)}) "
                                 "or an @parameter")
        else:
            try:
                ok = len(tuple(v)) == 3
            except TypeError:
                ok = False
            if not ok:
                raise ValueError(f"{where}: {what} must be a keyword, an @Vec3-parameter or three numbers")
    if h.kind == "ring" and h.scale == 1.0:
        object.__setattr__(h, "scale", 180.0 / 3.141592653589793)       # a ring turns a parameter in degrees


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
    source: str = ""              # file a node library loaded it from ("" for built-ins)
    code_hash: str = ""           # hash of that file: part of the cache key, so editing the code never serves stale results
    key_extra: Optional[Callable[[Dict[str, Any]], str]] = None   # extra cache-key material from outside the graph
    handles: Tuple["Handle", ...] = ()                              # viewport gizmos (cosmetic: never in the cache key)
    guides: Tuple["Guide", ...] = ()                                # viewport helpers (outlines of regions; cosmetic too)
    actions: Tuple["Action", ...] = ()                              # buttons (cosmetic too; see minihou.actions)
    _param_index: Dict[str, Param] = field(default_factory=dict, repr=False, compare=False)
    proxy_params: Tuple[Param, ...] = field(default=(), repr=False, compare=False)    # the parameters a proxy cook level scales

    def __post_init__(self):
        object.__setattr__(self, "_param_index", {p.name: p for p in self.params})
        object.__setattr__(self, "proxy_params", tuple(p for p in self.params if p.proxy))

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
        d = {"name": self.name, "version": self.version, "category": self.category, "doc": self.doc,
             "inputs": [i.__dict__ for i in self.inputs],
             "outputs": [o.__dict__ for o in self.outputs],
             "params": [p.describe() for p in self.params]}
        if self.actions:
            d["actions"] = [a.describe() for a in self.actions]
        if self.source:
            d["source"] = self.source
        from . import menu
        d.update(menu.info(self.name, self.category, self.source))
        return d


class Registry:
    """Node types by name. Readers always see a complete table: ``batch()`` stages many changes (a library reload that
    unregisters and rebuilds types) in a private copy that is published in one step, so a cook running on another
    thread never sees a type missing halfway through."""

    def __init__(self):
        self._types: Dict[str, NodeType] = {}
        self._tls = threading.local()

    @property
    def _cur(self) -> Dict[str, NodeType]:
        pending = getattr(self._tls, "pending", None)
        return self._types if pending is None else pending

    @contextlib.contextmanager
    def batch(self):
        """Changes made by this thread inside the block stay private until it ends (nested blocks join the outer one)."""
        if getattr(self._tls, "pending", None) is not None:
            yield
            return
        self._tls.pending = dict(self._types)
        try:
            yield
        finally:
            staged, self._tls.pending = self._tls.pending, None
            self._types = staged                           # one reference assignment: atomic for readers

    def register(self, nt: NodeType, *, replace: bool = False) -> NodeType:
        cur = self._cur
        if nt.name in cur and not replace:
            raise ValueError(f"node type {nt.name!r} is already registered")
        cur[nt.name] = nt
        return nt

    def unregister(self, name: str) -> Optional[NodeType]:
        return self._cur.pop(name, None)

    def get(self, name: str) -> NodeType:
        types = self._cur
        try:
            return types[name]
        except KeyError:
            raise KeyError(f"unknown node type {name!r}; known: {sorted(types)}") from None

    def __contains__(self, name: str) -> bool:
        return name in self._cur

    def names(self) -> List[str]:
        return sorted(self._cur)

    def __iter__(self):
        return iter(list(self._cur.values()))


REGISTRY = Registry()

# Set by minihou.library while it executes a node-library file: nodes defined there are stamped with the file and its
# hash, and may not take the name of an existing (e.g. built-in) node type.
_LOADING: Optional[Dict[str, str]] = None


def node(name: str, *, inputs: Sequence[Union[str, Input]] = (), params: Sequence[Param] = (),
         outputs: Optional[Sequence[Union[str, Output]]] = None, version: int = 1, category: str = "",
         doc: str = "", registry: Optional[Registry] = None, replace: bool = False,
         key_extra: Optional[Callable[[Dict[str, Any]], str]] = None, handles: Sequence["Handle"] = (),
         guides: Sequence["Guide"] = (),
         actions: Sequence["Action"] = ()):
    """Decorator: turn a function into a registered node type.

    ``key_extra(params) -> str`` is for nodes whose result depends on data outside the graph (e.g. a Blender object
    streamed over the link): its value joins the cache key, so a new version of that data recooks the node."""
    ins = tuple(Input(i) if isinstance(i, str) else i for i in inputs)
    outs = tuple(Output(o) if isinstance(o, str) else o for o in (outputs if outputs is not None else ["geo"]))
    params = tuple(params)
    handles = tuple(handles)
    guides = tuple(guides)
    actions = tuple(actions)

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
        src = _LOADING
        reg = registry or REGISTRY
        if src is not None and name in reg:
            other = reg.get(name).source
            raise ValueError(f"node type {name!r} already exists ({'in ' + other if other else 'built in'}); "
                             "choose another name")
        for h in handles:
            _check_handle(name, h, params)
        for g in guides:
            _check_guide(name, g, params)
        gkeys = [g.key() for g in guides]
        if len(set(gkeys)) != len(gkeys):
            raise ValueError(f"node {name!r}: guide ids must be unique (give two guides of one shape an id=)")
        keys = [h.key() for h in handles]
        if len(set(keys)) != len(keys):
            raise ValueError(f"node {name!r}: handle ids must be unique (give two handles on one parameter an id=)")
        nt = NodeType(name=name, fn=fn, inputs=ins, params=params, outputs=outs, version=version, handles=handles, guides=guides,
                      category=category or ("custom" if src else ""), doc=doc or (inspect.getdoc(fn) or ""),
                      source=src["file"] if src else "", code_hash=src["hash"] if src else "", key_extra=key_extra, actions=actions)
        reg.register(nt, replace=replace)
        fn.node_type = nt  # type: ignore[attr-defined]
        return fn

    return deco
