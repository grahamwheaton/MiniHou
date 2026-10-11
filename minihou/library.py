"""Node libraries: Python files of ``@node`` functions that the engine loads and hot-reloads.

Put files in ``<workspace>/nodes/`` (or pass ``serve --nodes DIR``). Each ``*.py`` file (names starting with ``_`` or
``.`` are skipped) is executed and its ``@node`` functions become node types, available in the editor's Tab menu and to
AI tools like built-in nodes. When a file changes it is re-executed:

* its node types are replaced as a unit (nodes removed from the file disappear);
* every node type carries a hash of its file, which is part of the cache key, so results never go stale;
* if the new code fails (syntax error, exception at import, a clash with an existing node name), the previous version
  stays loaded and the error (with file and line) is reported to the editor and AI tools.

Library files are ordinary Python running inside the engine, with the same trust as any script you run: only put files
you trust in a node folder. Opening a graph never executes code; graphs refer to node types by name.
"""
from __future__ import annotations

import hashlib
import json
import logging
import re
import threading
import traceback
import types
import uuid
import weakref
from dataclasses import dataclass, field
from pathlib import Path
from typing import Dict, Iterable, List, Optional

from . import sdk
from .sdk import REGISTRY, Registry

log = logging.getLogger("minihou.library")

_LOAD_LOCK = threading.Lock()              # sdk._LOADING is process-wide

TEMPLATE = '''"""{name}: describe what this family of nodes does."""
import numpy as np

from minihou.sdk import Float, Int, Vec3, node


@node("{name}", inputs=["geo"], category="custom", params=[
    Float("amount", 1.0, soft_min=0.0, soft_max=10.0, doc="How strong the effect is"),
])
def {name}(geo, amount):
    """One line on what the node does (shown in the editor and to AI tools)."""
    out = geo.copy()                                   # inputs are read-only: copy, then replace arrays
    P = geo.P.copy()
    P[:, 2] += amount
    out.point["P"] = P
    return out
'''


AUTHORING_GUIDE = """\
Writing a node: a Python file in the node folder (one file can hold several nodes). It is loaded within a second of
saving and re-loaded on every save; errors are reported with file and line, and the previous version stays active.

    import numpy as np
    from minihou.sdk import node, Float, Int, Bool, Vec3, Enum, Str, Input
    from minihou.geometry import Geometry, Topology

    @node("twist", inputs=["geo"], category="deform", params=[
        Float("angle", 90.0, soft_min=-360, soft_max=360, doc="Degrees of twist over the height"),
        Vec3("axis_center", (0, 0, 0)),
    ])
    def twist(geo, angle, axis_center):
        \"\"\"Twist points around Z, proportional to height.\"\"\"
        out = geo.copy()                      # shallow copy; untouched arrays stay shared (cheap, not re-sent)
        P = geo.P.astype(np.float64)          # (n, 3) float32, read-only: never modify inputs in place
        ...
        out.point["P"] = P.astype(np.float32)
        return out

Rules
* Pure function of inputs and params (the cache depends on it): no time, files, global random state.
* One positional argument per input (in order); a list for Input("geos", multi=True); None for an unconnected
  Input("x", optional=True). One keyword argument per param. Return a new Geometry.
* Vectorise with numpy (no Python loops over points): graphs often carry 100k-1M points.
* Params: Float/Int(name, default, min=, max=, soft_min=, soft_max=, doc=), Bool, Vec3, Enum(name, choices), Str(name,
  default, kind="code"). Float/Int/Bool/Vec3 params can be exposed to Blender.
* Node names: lower_case, unique, not an existing type. Category is free text (shown in the Tab menu).

Geometry API
* geo.P -> (n, 3) float32 positions; geo.npoints, geo.nprims, geo.nvertices
* geo.point / geo.vertex / geo.prim: attribute tables, e.g. geo.point["N"] (n, 3), out.point["Cd"] = arr (float32/int32,
  shape (n,) or (n, k)); "name" in geo.point; geo.point.names(); geo.detail is a plain dict
* geo.topology.vertex_point (vertex -> point), geo.topology.prim_offsets (CSR: prim i owns vertices
  offsets[i]:offsets[i+1]); geo.topology.prim_sizes()
* New geometry: Geometry.from_arrays(P, vertex_point, prim_offsets) or Geometry.points_only(P);
  Topology.from_polys([[0, 1, 2, 3], ...]) for small cases; out.set_topology(t) resets vertex/prim tables
* Faces are counter-clockwise when seen from outside (normals point out). Z is up; units are Blender metres.
The built-in nodes in minihou/nodes/*.py are complete examples (transform.py and noise.py are short).

Systems (no Python): a graph file saved in the node folder, e.g. nodes/rock_scatter.json, is also a node type, named after
the file. Inside it, `input` nodes (params: index, name) are the system's inputs (wire a stand-in into an input's `preview`
port to build it); its exposed parameters become the system node's parameters; its display node is the output; the graph's
top-level "doc" is the node's description. Build one with open_graph("nodes/<name>.json", create=true) + edit_graph, then
open the graph that should use it and add a node of type <name>. Systems can use other systems and library nodes.
"""


@dataclass
class LibraryFile:
    path: Path
    stamp: Optional[tuple] = None
    code_hash: str = ""
    types: List[str] = field(default_factory=list)
    error: Optional[str] = None


class NodeLibrary:
    def __init__(self, dirs: Iterable, registry: Registry = REGISTRY):
        self.dirs = [Path(d).resolve() for d in dirs]
        self.registry = registry
        self.files: Dict[Path, LibraryFile] = {}

    # -- scanning ----------------------------------------------------------------------------------
    def scan(self) -> List[Path]:
        """Python node files, then system graphs (*.json), per folder."""
        out = []
        for pattern in ("*.py", "*.json"):
            for d in self.dirs:
                if d.is_dir():
                    out.extend(sorted(p for p in d.glob(pattern) if not p.name.startswith(("_", "."))))
        return out

    def refresh(self) -> bool:
        """Load new or changed files, drop deleted ones. Returns True if node types or errors changed."""
        changed = systems_changed = False
        current = self.scan()
        for path in [p for p in self.files if p not in current]:
            systems_changed |= path.suffix == ".json"
            for name in self.files[path].types:
                self.registry.unregister(name)
            del self.files[path]
            log.info("node library: removed %s", path.name)
            changed = True
        for path in current:
            try:
                st = path.stat()
                stamp = (st.st_mtime_ns, st.st_size)
            except OSError:
                continue
            entry = self.files.get(path)
            if entry is not None and entry.stamp == stamp:
                continue
            if path.suffix == ".json":
                systems_changed = True                  # (re)built together below, in dependency order
                continue
            try:
                text = path.read_text(encoding="utf-8")
            except (OSError, UnicodeDecodeError) as e:
                entry = self.files.setdefault(path, LibraryFile(path))
                entry.stamp, entry.error = stamp, f"cannot read: {e}"
                changed = True
                continue
            h = hashlib.blake2b(text.encode("utf-8"), digest_size=8).hexdigest()
            if entry is not None and entry.code_hash == h:
                entry.stamp = stamp                     # touched, not changed
                continue
            self._load(path, text, h, stamp)
            changed = True
        if changed or systems_changed:
            # A system's cache hash covers the code of every node type inside it, so any change re-derives them all
            # (cheap: small JSON files). Unchanged results keep their keys and stay cached.
            self._load_systems([p for p in current if p.suffix == ".json"])
            changed = True
        return changed

    # -- loading -----------------------------------------------------------------------------------
    def _load(self, path: Path, text: str, h: str, stamp) -> None:
        entry = self.files.setdefault(path, LibraryFile(path))
        old = {n: self.registry.get(n) for n in entry.types if n in self.registry}
        for n in old:
            self.registry.unregister(n)
        src = str(path)
        with _LOAD_LOCK:
            sdk._LOADING = {"file": src, "hash": h}
            try:
                module = types.ModuleType(f"minihou_nodes_{path.stem}")
                module.__file__ = src
                exec(compile(text, src, "exec"), module.__dict__)          # noqa: S102 - user's own node library
                new = [nt.name for nt in self.registry if nt.source == src]
                error = None if new else "defines no @node functions"
            except Exception as e:  # noqa: BLE001 - report, keep the previous version
                for nt in [nt for nt in self.registry if nt.source == src]:
                    self.registry.unregister(nt.name)
                for nt in old.values():
                    self.registry.register(nt)
                new, error = list(old), _describe_error(e, src)
            finally:
                sdk._LOADING = None
        entry.stamp, entry.code_hash, entry.types, entry.error = stamp, h, new, error
        if error:
            log.warning("node library %s: %s", path.name, error)
        else:
            log.info("node library: loaded %s (%s)", path.name, ", ".join(new))

    # -- systems: graph files as node types ----------------------------------------------------------------
    def _load_systems(self, paths: List[Path]) -> None:
        from .graph import Graph, GraphError
        for path in paths:                                   # unregister every system first; rebuilt below
            for n in self.files.get(path, LibraryFile(path)).types:
                if n in self.registry and self.registry.get(n).source == str(path):
                    self.registry.unregister(n)
        texts: Dict[str, tuple] = {}
        for path in paths:
            entry = self.files.setdefault(path, LibraryFile(path))
            entry.types, entry.error = [], None
            try:
                st = path.stat()
                entry.stamp = (st.st_mtime_ns, st.st_size)
                text = path.read_text(encoding="utf-8")
                entry.code_hash = hashlib.blake2b(text.encode("utf-8"), digest_size=8).hexdigest()
                data = json.loads(text)
            except (OSError, UnicodeDecodeError, ValueError) as e:
                entry.error = f"cannot read: {e}"
                continue
            name = path.stem
            if not _SYSTEM_NAME.match(name):
                entry.error = f"file name {name!r} must be a valid node name (letters, digits, _)"
            elif name in self.registry:
                other = self.registry.get(name).source
                entry.error = f"node type {name!r} already exists ({'in ' + other if other else 'built in'})"
            elif not isinstance(data, dict) or not isinstance(data.get("nodes"), dict):
                entry.error = "not a graph file (needs a 'nodes' object)"
            else:
                texts[name] = (path, text, data)
        # dependency order between systems; a system that uses itself (directly or not) is an error
        deps = {n: {nd.get("type") for nd in d["nodes"].values() if isinstance(nd, dict)} & set(texts)
                for n, (_, _, d) in texts.items()}
        order, state = [], {}

        def visit(n, chain):
            if state.get(n) == 2:
                return True
            if state.get(n) == 1:
                for m in chain[chain.index(n):]:
                    self.files[texts[m][0]].error = f"system uses itself: {' -> '.join(chain[chain.index(n):] + [n])}"
                return False
            state[n] = 1
            ok = all(visit(m, chain + [n]) for m in sorted(deps[n]))
            state[n] = 2
            if ok:
                order.append(n)
            return ok

        for n in sorted(texts):
            visit(n, [])
        for name in order:
            path, text, data = texts[name]
            entry = self.files[path]
            if entry.error:
                continue
            try:
                graph = Graph.from_dict(data, self.registry, check=False)
                nt = _system_type(name, path, graph, text, self.registry)
            except (GraphError, ValueError, KeyError) as e:
                entry.error = str(e)
                continue
            self.registry.register(nt)
            entry.types = [name]
            log.info("node library: system %s (%s inputs, %d params)", path.name, len(nt.inputs), len(nt.params))
        for path in paths:
            if self.files[path].error:
                log.warning("node library %s: %s", path.name, self.files[path].error)

    def status(self) -> dict:
        return {"dirs": [str(d) for d in self.dirs],
                "files": [{"file": str(f.path), "nodes": list(f.types), "error": f.error}
                          for f in sorted(self.files.values(), key=lambda f: str(f.path))]}

    def unload(self) -> None:
        for entry in self.files.values():
            for name in entry.types:
                self.registry.unregister(name)
        self.files.clear()


_SYSTEM_NAME = re.compile(r"^[A-Za-z_][A-Za-z0-9_]*$")
_SYSTEM_LOCK = threading.RLock()              # one inner cache for all systems; re-entrant for nested systems
_SYSTEM_STACK = threading.local()
_TOKENS: "weakref.WeakKeyDictionary" = weakref.WeakKeyDictionary()


def _token(geo) -> str:
    """A cache key for geometry handed to a system: geometry is immutable once cooked, so identity is enough."""
    try:
        t = _TOKENS.get(geo)
        if t is None:
            t = _TOKENS[geo] = "in:" + uuid.uuid4().hex
        return t
    except TypeError:                                       # not weak-referenceable: never reuse
        return "in:" + uuid.uuid4().hex


def _system_evaluator(registry: Registry):
    """One evaluator (and bounded cache) per registry for the inside of systems; call with _SYSTEM_LOCK held."""
    ev = _SYS_EVALS.get(id(registry))
    if ev is None:
        from .evaluator import Cache, Evaluator
        ev = _SYS_EVALS[id(registry)] = Evaluator(registry, Cache(512 * 1024 ** 2))
    return ev


_SYS_EVALS: Dict[int, object] = {}


def _ident(label: str) -> str:
    s = re.sub(r"[^A-Za-z0-9_]+", "_", label.strip()).strip("_").lower() or "param"
    return "p_" + s if s[0].isdigit() else s


def _system_type(name: str, path: Path, graph, text: str, registry: Registry):
    """Build the NodeType for a system graph: inputs from its ``input`` nodes, params from its exposed parameters."""
    from dataclasses import replace
    from .sdk import GEOMETRY, Input, NodeType, Output
    ins = sorted((inst.params.get("index", 0), nid) for nid, inst in graph.nodes.items() if inst.type == "input")
    input_ids, inputs, seen = [], [], set()
    for idx, nid in ins:
        pname = str(graph.nodes[nid].params.get("name") or f"geo{idx}")
        if not _SYSTEM_NAME.match(pname) or pname in seen:
            raise ValueError(f"input node {nid!r}: port name {pname!r} is invalid or used twice")
        seen.add(pname)
        input_ids.append(nid)
        inputs.append(Input(pname, GEOMETRY, optional=True, doc=f"feeds input node {nid!r} (unconnected: its preview)"))
    params, pmap = [], {}
    for e in graph.exposed:
        if graph.exposed_problem(e):
            raise ValueError(f"exposed {e.name!r}: {graph.exposed_problem(e)}")
        inner = registry.get(graph.nodes[e.node].type).param(e.param)
        pname = _ident(e.name)
        while pname in seen or pname in pmap:
            pname += "_"
        params.append(replace(inner, name=pname, label=e.label or e.name, default=graph.get_param(e.node, e.param)))
        pmap[pname] = (e.node, e.param)
    used = sorted({inst.type for inst in graph.nodes.values()})
    dep = ",".join(f"{t}:{registry.get(t).code_hash or registry.get(t).version}" if t in registry else f"{t}:missing"
                   for t in used)
    code_hash = hashlib.blake2b((text + "|" + dep).encode("utf-8"), digest_size=8).hexdigest()

    def run(*geos, **values):
        stack = getattr(_SYSTEM_STACK, "names", None)
        if stack is None:
            stack = _SYSTEM_STACK.names = []
        if name in stack:
            raise RuntimeError(f"system {name!r} uses itself: {' -> '.join(stack + [name])}")
        stack.append(name)
        try:
            g = graph.copy()
            for pname, (nid, param) in pmap.items():
                g.set_param(nid, param, values[pname])
            feeds = {nid: (_token(geo), geo) for nid, geo in zip(input_ids, geos) if geo is not None}
            with _SYSTEM_LOCK:
                return _system_evaluator(registry).cook(g, feeds=feeds)
        finally:
            stack.pop()

    doc = graph.doc or f"System defined by {path.name}."
    return NodeType(name=name, fn=run, inputs=tuple(inputs), params=tuple(params), outputs=(Output("geo"),),
                    category="system", doc=doc, source=str(path), code_hash=code_hash)


def _describe_error(e: BaseException, src: str) -> str:
    if isinstance(e, SyntaxError) and e.lineno:
        return f"SyntaxError: {e.msg} (line {e.lineno})"
    line = next((fr.lineno for fr in reversed(traceback.extract_tb(e.__traceback__)) if fr.filename == src), None)
    return f"{type(e).__name__}: {e}" + (f" (line {line})" if line else "")
