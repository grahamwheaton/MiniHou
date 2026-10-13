"""Graph model: a DAG of typed nodes, stored as JSON.

JSON layout (hand-editable)::

    {
      "version": 1,
      "nodes": {
        "grid1":  {"type": "grid",  "params": {"rows": 100}},
        "noise1": {"type": "noise", "params": {"amplitude": [0, 0, 0.5]}, "inputs": {"geo": "grid1"}},
        "out":    {"type": "output", "inputs": {"geo": "noise1"}}
      },
      "display": "out",
      "exposed": [{"name": "Height", "node": "noise1", "param": "amplitude"}]
    }

* ``inputs`` maps an input port to a node id (a list of ids for ``multi`` ports such as merge).
* ``params`` lists only values that differ from the node type's defaults (any subset is fine).
* ``display`` names the node to cook for the live link; defaults to the first ``output`` node.
* ``exposed`` lists parameters to surface as Blender modifier inputs.
"""
from __future__ import annotations

import copy
import json
import re
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Dict, List, Optional, Union

from .sdk import REGISTRY, ParamError, Registry

GRAPH_VERSION = 1
_ID_RE = re.compile(r"^[A-Za-z_][A-Za-z0-9_]*$")

InputRef = Union[str, List[str], None]


class GraphError(ValueError):
    """The graph is structurally invalid. ``errors`` lists every problem found; ``node_id`` names the
    node the first problem belongs to, when there is one (editors use it to flag the node)."""

    def __init__(self, errors: Union[str, List[str]], node_id: Optional[str] = None):
        self.errors = [errors] if isinstance(errors, str) else list(errors)
        self.node_id = node_id
        super().__init__("; ".join(self.errors))


@dataclass
class NodeInstance:
    id: str
    type: str
    params: Dict[str, Any] = field(default_factory=dict)      # user-set values only (coerced)
    inputs: Dict[str, InputRef] = field(default_factory=dict)
    meta: Dict[str, Any] = field(default_factory=dict)        # editor data (pos, comment, ...) round-tripped


@dataclass
class Exposed:
    name: str          # unique name shown in Blender
    node: str
    param: str
    label: str = ""


_BOX_COLOR = re.compile(r"^#[0-9a-fA-F]{6}$")


def clean_box(d: Any) -> Dict[str, Any]:
    """Validate frame geometry plus optional note text and collapsed/locked flags."""
    if not isinstance(d, dict):
        raise GraphError("a box is an object")
    bid = d.get("id")
    if not isinstance(bid, str) or not _ID_RE.match(bid):
        raise GraphError(f"invalid box id {bid!r} (use letters, digits, underscore)")

    def pair(key, lo=None):
        v = d.get(key)
        if (not isinstance(v, (list, tuple)) or len(v) != 2
                or not all(isinstance(c, (int, float)) and not isinstance(c, bool) and abs(c) < 1e7 for c in v)):
            raise GraphError(f"{key} must be [x, y]")
        out = [round(float(c), 1) for c in v]
        return out if lo is None else [max(lo, c) for c in out]

    color = d.get("color") or ""
    if color and not (isinstance(color, str) and _BOX_COLOR.match(color)):
        raise GraphError(f"color must be '#rrggbb', got {color!r}")
    result = {"id": bid, "title": str(d.get("title") or "")[:80], "color": color.lower(),
            "pos": pair("pos"), "size": pair("size", 40.0)}
    for key in ("collapsed", "locked", "note"):
        if key in d:
            if not isinstance(d[key], bool):
                raise GraphError(f"{key} must be a boolean")
            result[key] = d[key]
    if "text" in d:
        if not isinstance(d["text"], str) or len(d["text"]) > 10000:
            raise GraphError("note text must be a string of at most 10000 characters")
        result["text"] = d["text"]
    return result


class Graph:
    def __init__(self, registry: Optional[Registry] = None):
        self.registry = registry or REGISTRY
        self.nodes: Dict[str, NodeInstance] = {}
        self.display: Optional[str] = None
        self.exposed: List[Exposed] = []
        self.doc: str = ""                    # what the graph does (systems show it as their node doc)
        self.boxes: List[Dict[str, Any]] = []  # network boxes: coloured, titled rectangles behind the nodes (cosmetic)

    # -- editing -----------------------------------------------------------
    def add_node(self, node_id: str, type: str, params: Optional[dict] = None,
                 inputs: Optional[Dict[str, InputRef]] = None, **meta) -> NodeInstance:
        if not _ID_RE.match(node_id):
            raise GraphError(f"invalid node id {node_id!r} (use letters, digits, underscore)")
        if node_id in self.nodes:
            raise GraphError(f"duplicate node id {node_id!r}")
        nt = self.registry.get(type)
        inst = NodeInstance(node_id, type, nt.coerce_params(params) if params else {}, {}, dict(meta))
        # Keep only explicitly-given params (coerce_params fills defaults for the rest).
        inst.params = {k: v for k, v in inst.params.items() if k in (params or {})}
        self.nodes[node_id] = inst
        for port, src in (inputs or {}).items():
            self.connect(node_id, port, src)
        return inst

    def remove_node(self, node_id: str) -> None:
        del self.nodes[node_id]
        for n in self.nodes.values():
            for port, ref in list(n.inputs.items()):
                if isinstance(ref, list):
                    n.inputs[port] = [r for r in ref if r != node_id]
                elif ref == node_id:
                    del n.inputs[port]
        self.exposed = [e for e in self.exposed if e.node != node_id]
        if self.display == node_id:
            self.display = None

    def connect(self, node_id: str, port: str, source: Union[str, List[str]]) -> None:
        inst = self._node(node_id)
        nt = self.registry.get(inst.type)
        ports = {i.name: i for i in nt.inputs}
        if port not in ports:
            raise GraphError(f"node {node_id!r} ({inst.type}) has no input {port!r}; has {list(ports)}")
        if ports[port].multi:
            inst.inputs[port] = [source] if isinstance(source, str) else list(source)
        else:
            if not isinstance(source, str):
                raise GraphError(f"input {node_id}.{port} takes a single node id")
            inst.inputs[port] = source

    def set_param(self, node_id: str, param: str, value: Any) -> Any:
        """Validate and set a parameter. Returns the canonical stored value."""
        inst = self._node(node_id)
        v = self.registry.get(inst.type).param(param).coerce(value)
        inst.params[param] = v
        return v

    def get_param(self, node_id: str, param: str) -> Any:
        inst = self._node(node_id)
        nt = self.registry.get(inst.type)
        return inst.params[param] if param in inst.params else nt.param(param).default

    def effective_params(self, node_id: str) -> Dict[str, Any]:
        inst = self._node(node_id)
        return self.registry.get(inst.type).coerce_params(inst.params)

    def _node(self, node_id: str) -> NodeInstance:
        try:
            return self.nodes[node_id]
        except KeyError:
            raise GraphError(f"no such node {node_id!r}") from None

    # -- structure ---------------------------------------------------------
    def input_ids(self, node_id: str) -> List[str]:
        """All upstream node ids referenced by a node, in port order."""
        out: List[str] = []
        for ref in self.nodes[node_id].inputs.values():
            if isinstance(ref, list):
                out.extend(ref)
            elif ref is not None:
                out.append(ref)
        return out

    def _main_output(self) -> Optional[str]:
        if self.display:
            return self.display
        outs = [nid for nid, n in self.nodes.items() if n.type == "output"]
        for nid in outs:                                    # the main output is one without its own Blender object
            if not self.nodes[nid].params.get("object"):
                return nid
        return outs[0] if outs else None

    def extra_outputs(self) -> Dict[str, str]:
        """``{object name: node id}`` for ``output`` nodes that publish to their own Blender object (the display node is
        always the main output, whatever its ``object`` says)."""
        main = self._main_output()
        out: Dict[str, str] = {}
        for nid, n in self.nodes.items():
            name = str(n.params.get("object") or "").strip()[:63] if n.type == "output" else ""
            if name and nid != main and name not in out:
                out[name] = nid
        return out

    def scene_sinks(self) -> Dict[str, str]:
        """``{node id: node id}`` of the ``scene_objects`` and ``image_out`` nodes: results that become real Blender objects / images."""
        return {nid: nid for nid, n in self.nodes.items() if n.type in ("scene_objects", "image_out")}

    def output_node(self) -> str:
        main = self._main_output()
        if main is None:
            raise GraphError("graph has no display node and no 'output' node")
        return main

    def upstream_order(self, target: str, stop=()) -> List[str]:
        """Node ids needed to cook ``target``, dependencies first, ``target`` last (iterative DFS). Nodes in ``stop`` are
        included but not descended into (their result is supplied from outside, e.g. a system's inputs)."""
        self._node(target)
        order: List[str] = []
        state: Dict[str, int] = {}  # 1 = in progress, 2 = done
        kids = lambda n: iter(()) if n in stop else iter(self.input_ids(n))  # noqa: E731
        stack = [(target, kids(target))]
        state[target] = 1
        while stack:
            nid, it = stack[-1]
            for child in it:
                if child not in self.nodes:
                    raise GraphError(f"node {nid!r} references missing node {child!r}", nid)
                st = state.get(child)
                if st == 1:
                    raise GraphError(f"cycle detected through {child!r}", child)
                if st is None:
                    state[child] = 1
                    stack.append((child, kids(child)))
                    break
            else:
                state[nid] = 2
                order.append(nid)
                stack.pop()
        return order

    def downstream(self, node_id: str) -> List[str]:
        """Node ids that depend (transitively) on ``node_id``, excluding itself."""
        consumers: Dict[str, List[str]] = {}
        for nid in self.nodes:
            for src in self.input_ids(nid):
                consumers.setdefault(src, []).append(nid)
        seen, stack = set(), [node_id]
        while stack:
            for c in consumers.get(stack.pop(), []):
                if c not in seen:
                    seen.add(c)
                    stack.append(c)
        return [n for n in self.nodes if n in seen]

    def validate(self) -> List[str]:
        """Return a list of human-readable problems (empty if the graph is sound)."""
        return [msg for _, msg in self.problems()]

    def problems(self, only: Optional[List[str]] = None) -> List[tuple]:
        """``[(node_id or None, message)]`` for every problem. With ``only``, just the node-level checks for
        those nodes (what a cook of one target needs; an unfinished branch elsewhere doesn't block it)."""
        errs: List[tuple] = []
        for nid in (only if only is not None else list(self.nodes)):
            inst = self.nodes[nid]
            if not _ID_RE.match(nid):
                errs.append((nid, f"invalid node id {nid!r}"))
            if inst.type not in self.registry:
                errs.append((nid, f"node {nid!r}: unknown node type {inst.type!r}"))
                continue
            nt = self.registry.get(inst.type)
            try:
                nt.coerce_params(inst.params)
            except ParamError as e:
                errs.append((nid, f"node {nid!r}: {e}"))
            ports = {i.name: i for i in nt.inputs}
            for port in inst.inputs:
                if port not in ports:
                    errs.append((nid, f"node {nid!r}: unknown input port {port!r}"))
            for port, spec in ports.items():
                ref = inst.inputs.get(port)
                if ref in (None, []):
                    if not spec.optional and not spec.multi:
                        errs.append((nid, f"node {nid!r}: input {port!r} is not connected"))
                    elif spec.multi and not spec.optional:
                        errs.append((nid, f"node {nid!r}: input {port!r} needs at least one connection"))
                    continue
                for src in ([ref] if isinstance(ref, str) else ref):
                    if src not in self.nodes:
                        errs.append((nid, f"node {nid!r}: input {port!r} references missing node {src!r}"))
        if only is not None:
            return errs
        # cycles (Kahn)
        indeg = {n: 0 for n in self.nodes}
        consumers: Dict[str, List[str]] = {n: [] for n in self.nodes}
        for nid in self.nodes:
            for src in self.input_ids(nid):
                if src in self.nodes:
                    indeg[nid] += 1
                    consumers[src].append(nid)
        queue = [n for n, d in indeg.items() if d == 0]
        order = []
        seen = 0
        while queue:
            n = queue.pop()
            order.append(n)
            seen += 1
            for c in consumers[n]:
                indeg[c] -= 1
                if indeg[c] == 0:
                    queue.append(c)
        if seen != len(self.nodes):
            stuck = sorted(n for n, d in indeg.items() if d > 0)
            errs.append((stuck[0], f"graph contains a cycle involving {stuck}"))
        elif not errs and any(n.type in {"loop_begin", "loop_end"} for n in self.nodes.values()):
            from .loops import analyze
            try:
                analyze(self, order, self.display)
            except GraphError as exc:
                errs.append((exc.node_id, str(exc)))
        if self.display is not None and self.display not in self.nodes:
            errs.append((None, f"display node {self.display!r} does not exist"))
        names = set()
        for e in self.exposed:
            if e.name in names:
                errs.append((e.node, f"exposed parameter name {e.name!r} is used twice"))
            names.add(e.name)
            problem = self.exposed_problem(e)
            if problem:
                errs.append((e.node if e.node in self.nodes else None, problem))
        return errs

    def exposed_problem(self, e: "Exposed") -> Optional[str]:
        if e.node not in self.nodes:
            return f"exposed {e.name!r}: node {e.node!r} does not exist"
        inst = self.nodes[e.node]
        if inst.type not in self.registry:
            return f"exposed {e.name!r}: node {e.node!r} has unknown type {inst.type!r}"
        try:
            p = self.registry.get(inst.type).param(e.param)
        except ParamError as ex:
            return f"exposed {e.name!r}: {ex}"
        if p.kind not in EXPOSABLE_KINDS:
            return (f"exposed {e.name!r}: parameter kind {p.kind!r} cannot be exposed "
                    f"(supported: {sorted(EXPOSABLE_KINDS)})")
        return None

    def check(self) -> "Graph":
        probs = self.problems()
        if probs:
            raise GraphError([m for _, m in probs], probs[0][0])
        return self

    # -- exposed parameters ------------------------------------------------
    def find_exposed(self, name: str) -> Exposed:
        for e in self.exposed:
            if e.name == name:
                return e
        raise GraphError(f"no exposed parameter named {name!r}")

    def interface(self) -> List[dict]:
        """Description of exposed parameters (for the Blender node-group interface)."""
        out = []
        for e in self.exposed:
            if self.exposed_problem(e):
                continue                      # an unchecked (editor-saved) graph may hold a stale entry
            inst = self.nodes[e.node]
            p = self.registry.get(inst.type).param(e.param)
            d = p.describe()
            d.update(name=e.name, label=e.label or e.name, node=e.node, param=e.param,
                     value=p.to_json_value(self.get_param(e.node, e.param)))
            out.append(d)
        return out

    def set_exposed(self, name: str, value: Any) -> Any:
        e = self.find_exposed(name)
        return self.set_param(e.node, e.param, value)

    # -- (de)serialisation -------------------------------------------------
    def copy(self) -> "Graph":
        g = Graph(self.registry)
        g.nodes = {k: NodeInstance(v.id, v.type, dict(v.params),
                                   {p: (list(r) if isinstance(r, list) else r) for p, r in v.inputs.items()},
                                   copy.deepcopy(v.meta))
                   for k, v in self.nodes.items()}
        g.display = self.display
        g.exposed = [Exposed(e.name, e.node, e.param, e.label) for e in self.exposed]
        g.doc = self.doc
        g.boxes = copy.deepcopy(self.boxes)
        return g

    def to_dict(self) -> dict:
        nodes = {}
        for nid, inst in self.nodes.items():
            nt = self.registry.get(inst.type) if inst.type in self.registry else None
            d: Dict[str, Any] = {"type": inst.type}
            if inst.params:
                d["params"] = ({k: _tidy(nt.param(k).to_json_value(v)) for k, v in inst.params.items()} if nt
                               else dict(inst.params))
            if inst.inputs:
                d["inputs"] = {k: (list(v) if isinstance(v, list) else v) for k, v in inst.inputs.items()}
            d.update(inst.meta)
            if isinstance(d.get("pos"), list):
                d["pos"] = _tidy(d["pos"])
            nodes[nid] = d
        out: Dict[str, Any] = {"version": GRAPH_VERSION}
        if self.doc:
            out["doc"] = self.doc
        out["nodes"] = nodes
        if self.boxes:
            out["boxes"] = copy.deepcopy(self.boxes)
        if self.display:
            out["display"] = self.display
        if self.exposed:
            out["exposed"] = [{k: v for k, v in (("name", e.name), ("node", e.node), ("param", e.param),
                                                 ("label", e.label)) if v} for e in self.exposed]
        return out

    @classmethod
    def from_dict(cls, data: dict, registry: Optional[Registry] = None, *, check: bool = True) -> "Graph":
        if not isinstance(data, dict) or not isinstance(data.get("nodes"), dict):
            raise GraphError("graph JSON must be an object with a 'nodes' object")
        version = data.get("version", GRAPH_VERSION)
        if version != GRAPH_VERSION:
            raise GraphError(f"unsupported graph version {version!r} (this build reads {GRAPH_VERSION})")
        g = cls(registry)
        errs: List[str] = []
        for nid, nd in data["nodes"].items():
            if not isinstance(nd, dict) or "type" not in nd:
                errs.append(f"node {nid!r}: expected an object with a 'type'")
                continue
            meta = {k: v for k, v in nd.items() if k not in ("type", "params", "inputs")}
            inputs = {}
            for port, ref in (nd.get("inputs") or {}).items():
                inputs[port] = list(ref) if isinstance(ref, list) else ref
            if nd["type"] not in g.registry:
                if check:
                    errs.append(f"node {nid!r}: unknown node type {nd['type']!r}")
                    continue
                # Unchecked (engine, editor): keep it as written. Its node library may be mid-edit; cooks through it
                # report the missing type, and saving round-trips it unchanged.
                g.nodes[nid] = NodeInstance(nid, nd["type"], dict(nd.get("params") or {}), inputs, meta)
                continue
            nt = g.registry.get(nd["type"])
            try:
                params = nt.coerce_params(nd.get("params"))
                params = {k: v for k, v in params.items() if k in (nd.get("params") or {})}
            except ParamError as e:
                errs.append(f"node {nid!r}: {e}")
                params = {}
            if not _ID_RE.match(nid):
                errs.append(f"invalid node id {nid!r}")
            g.nodes[nid] = NodeInstance(nid, nd["type"], params, inputs, meta)
        g.display = data.get("display")
        g.doc = str(data.get("doc") or "")
        for bd in data.get("boxes", []) or []:
            try:
                g.boxes.append(clean_box(bd))
            except GraphError as e:
                errs.append(f"bad network box {bd!r}: {e}")
        for ed in data.get("exposed", []) or []:
            try:
                g.exposed.append(Exposed(ed["name"], ed["node"], ed["param"], ed.get("label", "")))
            except (KeyError, TypeError):
                errs.append(f"bad exposed entry {ed!r}: needs 'name', 'node' and 'param'")
        if errs:
            raise GraphError(errs)
        if check:
            g.check()
        return g

    @classmethod
    def from_json(cls, text: str, registry: Optional[Registry] = None, *, check: bool = True) -> "Graph":
        try:
            data = json.loads(text)
        except json.JSONDecodeError as e:
            raise GraphError(f"invalid JSON: {e}") from None
        return cls.from_dict(data, registry, check=check)

    def to_json(self, indent: int = 2) -> str:
        """Pretty JSON for hand-editing and diffs: short arrays (vectors, positions, input lists) stay on one line."""
        return _FLAT_ARRAY.sub(lambda m: "[" + ", ".join(x.strip() for x in m.group(1).split(",\n")) + "]",
                               json.dumps(self.to_dict(), indent=indent)) + "\n"

    @classmethod
    def load(cls, path: Union[str, Path], registry: Optional[Registry] = None, *, check: bool = True) -> "Graph":
        return cls.from_json(Path(path).read_text(encoding="utf-8"), registry, check=check)

    def save(self, path: Union[str, Path]) -> None:
        with open(path, "w", encoding="utf-8", newline="\n") as f:
            f.write(self.to_json())


def _tidy(v: Any) -> Any:
    """Whole-number floats as ints (``10`` stays ``10`` in a saved file, not ``10.0``); loading coerces them back."""
    if isinstance(v, float) and v.is_integer() and abs(v) < 2 ** 53:
        return int(v)
    if isinstance(v, list):
        return [_tidy(x) for x in v]
    return v


# An array of scalars spread over several lines by json.dumps(indent=...): no nested arrays/objects. JSON strings
# cannot contain raw newlines, so splitting the items on ",\n" is safe.
_FLAT_ARRAY = re.compile(r"\[\n\s*((?:[^\[\]{}\n]+,\n\s*)*[^\[\]{}\n]+)\n\s*\]")


# Parameter kinds that can become Blender modifier inputs.
EXPOSABLE_KINDS = frozenset({"float", "int", "bool", "vec3"})
