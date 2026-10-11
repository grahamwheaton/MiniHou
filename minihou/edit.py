"""Graph edit operations: one small JSON vocabulary shared by the web editor, the MCP server and tests.

A batch of ops is applied to a *copy* of the graph and only committed if every op succeeds, so a failing
batch leaves the graph untouched. Each op is a dict with an ``"op"`` key:

==============  =============================================================================================
op              fields
==============  =============================================================================================
``add``         ``type``; optional ``id`` (default: type + number), ``params``, ``inputs`` ({port: id or [ids]}),
                ``pos`` ([x, y]), ``display`` (true to make it the display node), ``comment``
``remove``      ``id`` (an id or a list of ids)
``rename``      ``id``, ``to``
``connect``     ``from``, ``to``; optional ``port`` (default: first free input), ``index`` (multi inputs)
``disconnect``  ``to``; optional ``port``, ``from`` (default: everything into ``to``/``port``)
``set``         ``id``, ``params`` ({name: value}; ``null`` resets a parameter to its default)
``display``     ``id``
``expose``      ``id``, ``param``; optional ``name`` (shown in Blender), ``label``
``unexpose``    ``name``, or ``id`` + ``param``
``move``        ``id`` + ``pos``, or ``positions`` ({id: [x, y]})
``meta``        ``id``, ``set`` ({key: value}; ``null`` deletes) - free editor data such as ``comment``
``replace``     ``graph`` (a whole graph JSON object)
``layout``      (no fields) tidy all node positions, like Houdini's L key
``loop``        optional ``from``, ``to``/``port``, ``iterations``, ``pos``; insert paired Begin, Transform and End
==============  =============================================================================================

Graphs may be *incomplete* while being edited (a node whose input is not wired yet); the cook reports that,
the edit is still accepted. Edits that can never be valid are rejected: unknown types/params/ports, bad values,
cycles, duplicate ids.
"""
from __future__ import annotations

import re
from typing import Any, Dict, List, Optional, Tuple

from .graph import EXPOSABLE_KINDS, Exposed, Graph, GraphError, NodeInstance
from .sdk import ParamError

_ID_RE = re.compile(r"^[A-Za-z_][A-Za-z0-9_]*$")
RESERVED_META = frozenset({"type", "params", "inputs"})
GRID_X, GRID_Y = 190, 110          # default node spacing used by the auto-layout


class EditError(ValueError):
    """An op in a batch failed; nothing in the batch was applied."""

    def __init__(self, index: int, op: Any, message: str):
        self.index, self.op, self.message = index, op, message
        kind = op.get("op") if isinstance(op, dict) else None
        super().__init__(f"op {index} ({kind or '?'}): {message}")


def apply_ops(graph: Graph, ops: List[dict]) -> Tuple[Graph, List[dict]]:
    """Apply ``ops`` atomically. Returns ``(new_graph, results)``; raises :class:`EditError`."""
    if not isinstance(ops, list):
        raise EditError(0, None, "ops must be a list")
    g = graph.copy()
    results: List[dict] = []
    for i, op in enumerate(ops):
        if not isinstance(op, dict) or not isinstance(op.get("op"), str):
            raise EditError(i, op, 'each op must be an object with an "op" field')
        fn = _OPS.get(op["op"])
        if fn is None:
            raise EditError(i, op, f"unknown op {op['op']!r}; known: {sorted(_OPS)}")
        try:
            out = fn(g, op)
            results.append(out or {})
            if op["op"] == "replace":
                g = out.pop("_graph")
        except EditError:
            raise
        except (GraphError, ParamError) as e:
            raise EditError(i, op, str(e)) from None
        except KeyError as e:
            msg = e.args[0] if e.args and isinstance(e.args[0], str) and " " in e.args[0] else f"missing field {e}"
            raise EditError(i, op, msg) from None
        except (TypeError, ValueError) as e:
            raise EditError(i, op, f"bad op: {e}") from None
    explicit = set()                                   # positions this batch set on purpose survive the auto-layout
    for op in ops:
        if op.get("op") == "add" and "pos" in op:
            explicit.add(op.get("id") or "")
        elif op.get("op") == "move":
            explicit.update(op.get("positions") or [op.get("id")])
        elif op.get("op") == "layout":
            explicit.clear()
    explicit.update(r["id"] for r, op in zip(results, ops) if op.get("op") == "add" and "pos" in op and "id" in r)
    layout_missing(g, keep=explicit)
    return g, results


# ---------------------------------------------------------------------------
# ops
# ---------------------------------------------------------------------------
def _need_node(g: Graph, nid: Any) -> NodeInstance:
    if not isinstance(nid, str) or nid not in g.nodes:
        raise GraphError(f"no such node {nid!r}")
    return g.nodes[nid]


def _new_id(g: Graph, base: str) -> str:
    base = re.sub(r"[^A-Za-z0-9_]", "_", base) or "node"
    if base[0].isdigit():
        base = "n" + base
    n = 1
    while f"{base}{n}" in g.nodes:
        n += 1
    return f"{base}{n}"


def _pos(v: Any) -> List[float]:
    if (not isinstance(v, (list, tuple)) or len(v) != 2
            or not all(isinstance(c, (int, float)) and not isinstance(c, bool) for c in v)):
        raise GraphError(f"pos must be [x, y], got {v!r}")
    return [round(float(c), 1) for c in v]


def op_add(g: Graph, op: dict) -> dict:
    t = op["type"]
    if t not in g.registry:
        raise GraphError(f"unknown node type {t!r}; known: {g.registry.names()}")
    nid = op.get("id") or _new_id(g, t)
    meta: Dict[str, Any] = {}
    if "pos" in op:
        meta["pos"] = _pos(op["pos"])
    if op.get("comment"):
        meta["comment"] = str(op["comment"])
    g.add_node(nid, t, op.get("params") or None, None, **meta)
    for port, src in (op.get("inputs") or {}).items():
        for s in ([src] if isinstance(src, str) else list(src)):
            _connect(g, s, nid, port, None)
    if op.get("display"):
        g.display = nid
    return {"id": nid}


def op_remove(g: Graph, op: dict) -> dict:
    ids = op["id"] if isinstance(op["id"], list) else [op["id"]]
    for nid in ids:
        _need_node(g, nid)
        g.remove_node(nid)
    return {"removed": ids}


def op_loop(g: Graph, op: dict) -> dict:
    """One undoable insertion. Pairing uses an input edge, so rename/save/duplicate need no side registry."""
    source = op.get("from")
    if source is not None:
        _need_node(g, source)
    x, y = _pos(op.get("pos", [0, 0]))
    begin = _new_id(g, "loop_begin")
    g.add_node(begin, "loop_begin", inputs={"geo": source} if source else {}, pos=[x, y])
    body = _new_id(g, "transform")
    g.add_node(body, "transform", inputs={"geo": begin}, pos=[x, y + GRID_Y])
    end = _new_id(g, "loop_end")
    g.add_node(end, "loop_end", params={"iterations": op.get("iterations", 3)},
               inputs={"geo": body, "begin": begin}, pos=[x, y + 2 * GRID_Y])
    if op.get("to"):
        _connect(g, end, op["to"], op.get("port"), None)
    return {"id": end, "begin": begin, "body": body, "end": end}


def op_rename(g: Graph, op: dict) -> dict:
    old, new = op["id"], op["to"]
    _need_node(g, old)
    if not isinstance(new, str) or not _ID_RE.match(new):
        raise GraphError(f"invalid node id {new!r} (use letters, digits, underscore; not starting with a digit)")
    if new == old:
        return {"id": new}
    if new in g.nodes:
        raise GraphError(f"a node called {new!r} already exists")
    g.nodes = {(new if k == old else k): v for k, v in g.nodes.items()}
    g.nodes[new].id = new
    for n in g.nodes.values():
        for port, ref in list(n.inputs.items()):
            n.inputs[port] = [new if r == old else r for r in ref] if isinstance(ref, list) else (new if ref == old else ref)
    if g.display == old:
        g.display = new
    for e in g.exposed:
        if e.node == old:
            e.node = new
    return {"id": new}


def _connect(g: Graph, src: str, dst: str, port: Optional[str], index: Optional[int]) -> str:
    _need_node(g, src)
    inst = _need_node(g, dst)
    nt = g.registry.get(inst.type)
    if not nt.inputs:
        raise GraphError(f"node {dst!r} ({inst.type}) has no inputs")
    if port is None:
        free = [i.name for i in nt.inputs if inst.inputs.get(i.name) in (None, [])]
        port = free[0] if free else nt.inputs[0].name
    spec = {i.name: i for i in nt.inputs}.get(port)
    if spec is None:
        raise GraphError(f"node {dst!r} ({inst.type}) has no input {port!r}; inputs: {nt.input_names()}")
    if src == dst or dst in g.upstream_order(src):
        raise GraphError(f"connecting {src!r} -> {dst!r} would create a cycle")
    if spec.multi:
        cur = [r for r in (inst.inputs.get(port) or []) if r != src]
        cur.insert(len(cur) if index is None else max(0, min(int(index), len(cur))), src)
        inst.inputs[port] = cur
    else:
        inst.inputs[port] = src
    return port


def op_connect(g: Graph, op: dict) -> dict:
    port = _connect(g, op["from"], op["to"], op.get("port"), op.get("index"))
    return {"port": port}


def op_disconnect(g: Graph, op: dict) -> dict:
    inst = _need_node(g, op["to"])
    port, src = op.get("port"), op.get("from")
    ports = [port] if port else list(inst.inputs)
    if port and port not in g.registry.get(inst.type).input_names():
        raise GraphError(f"node {op['to']!r} has no input {port!r}")
    for p in ports:
        ref = inst.inputs.get(p)
        if ref is None:
            continue
        if src is None:
            del inst.inputs[p]
        elif isinstance(ref, list):
            inst.inputs[p] = [r for r in ref if r != src]
            if not inst.inputs[p]:
                del inst.inputs[p]
        elif ref == src:
            del inst.inputs[p]
    return {}


def op_set(g: Graph, op: dict) -> dict:
    inst = _need_node(g, op["id"])
    params = op["params"]
    if not isinstance(params, dict):
        raise GraphError("params must be an object {name: value}")
    nt = g.registry.get(inst.type)
    out = {}
    for name, value in params.items():
        p = nt.param(name)
        if value is None:
            inst.params.pop(name, None)
            out[name] = p.to_json_value(p.default)
        else:
            out[name] = p.to_json_value(g.set_param(op["id"], name, value))
    return {"params": out}


def op_display(g: Graph, op: dict) -> dict:
    _need_node(g, op["id"])
    g.display = op["id"]
    return {}


def op_expose(g: Graph, op: dict) -> dict:
    nid, pname = op["id"], op["param"]
    inst = _need_node(g, nid)
    p = g.registry.get(inst.type).param(pname)
    if p.kind not in EXPOSABLE_KINDS:
        raise GraphError(f"{inst.type}.{pname} is a {p.kind} parameter; Blender can take {sorted(EXPOSABLE_KINDS)}")
    existing = next((e for e in g.exposed if e.node == nid and e.param == pname), None)
    name = op.get("name") or (existing.name if existing else p.display_label)
    taken = {e.name for e in g.exposed if e is not existing}
    if name in taken:
        if op.get("name"):
            raise GraphError(f"exposed name {name!r} is already used")
        name = f"{nid} {p.display_label}"
        k = 2
        while name in taken:
            name, k = f"{nid} {p.display_label} {k}", k + 1
    if existing:
        existing.name, existing.label = name, op.get("label", existing.label)
    else:
        g.exposed.append(Exposed(name, nid, pname, op.get("label", "")))
    return {"name": name}


def op_unexpose(g: Graph, op: dict) -> dict:
    before = len(g.exposed)
    if "name" in op:
        g.exposed = [e for e in g.exposed if e.name != op["name"]]
    else:
        g.exposed = [e for e in g.exposed if not (e.node == op["id"] and e.param == op["param"])]
    if len(g.exposed) == before:
        raise GraphError("no such exposed parameter")
    return {}


def op_move(g: Graph, op: dict) -> dict:
    positions = op.get("positions") or {op["id"]: op["pos"]}
    for nid, pos in positions.items():
        _need_node(g, nid).meta["pos"] = _pos(pos)
    return {}


def op_meta(g: Graph, op: dict) -> dict:
    inst = _need_node(g, op["id"])
    for k, v in dict(op["set"]).items():
        if k in RESERVED_META or not isinstance(k, str):
            raise GraphError(f"meta key {k!r} is reserved")
        if v is None:
            inst.meta.pop(k, None)
        else:
            inst.meta[k] = v
    return {}


def op_replace(g: Graph, op: dict) -> dict:
    return {"_graph": Graph.from_dict(op["graph"], g.registry, check=False)}


_OPS = {"add": op_add, "loop": op_loop, "remove": op_remove, "rename": op_rename, "connect": op_connect,
        "disconnect": op_disconnect, "set": op_set, "display": op_display, "expose": op_expose,
        "unexpose": op_unexpose, "move": op_move, "meta": op_meta, "replace": op_replace}
OP_NAMES = sorted(_OPS)


# ---------------------------------------------------------------------------
# layout
# ---------------------------------------------------------------------------
def layout_missing(g: Graph, keep=()) -> bool:
    """Give every node without ``pos`` a position. Returns True if anything changed.

    Mostly new nodes (a hand-written file, or a system an AI just built): tidy the whole graph.
    A few new nodes: each goes below its first input (or to the right of everything)."""
    missing = [nid for nid, n in g.nodes.items() if "pos" not in n.meta]
    if not missing:
        return False
    if 2 * len(missing) >= len(g.nodes):
        kept = {nid: list(g.nodes[nid].meta["pos"]) for nid in keep if nid in g.nodes and "pos" in g.nodes[nid].meta}
        layout_all(g)
        for nid, p in kept.items():
            g.nodes[nid].meta["pos"] = p
        return True
    placed = {nid: tuple(n.meta["pos"]) for nid, n in g.nodes.items() if "pos" in n.meta}
    occupied = set(placed.values())
    for nid in _topo(g):
        if nid in placed:
            continue
        srcs = [s for s in g.input_ids(nid) if s in placed]
        if srcs:
            x, y = placed[srcs[0]][0], placed[srcs[0]][1] + GRID_Y
        else:
            x = max(p[0] for p in placed.values()) + GRID_X
            y = min(p[1] for p in placed.values())
        while (x, y) in occupied:
            x += GRID_X
        placed[nid] = (x, y)
        occupied.add((x, y))
        g.nodes[nid].meta["pos"] = [float(x), float(y)]
    return True


def layout_all(g: Graph) -> None:
    """Tidy every node: layered top-to-bottom layout (Houdini's L), anchored where the graph already was."""
    old = [n.meta["pos"] for n in g.nodes.values() if isinstance(n.meta.get("pos"), list)]
    pos = _layered(g)
    if not pos:
        return
    dx = dy = 0.0
    if old:
        dx = min(p[0] for p in old) - min(p[0] for p in pos.values())
        dy = min(p[1] for p in old) - min(p[1] for p in pos.values())
    for nid, (x, y) in pos.items():
        g.nodes[nid].meta["pos"] = [float(round(x + dx)), float(round(y + dy))]


def op_layout(g: Graph, op: dict) -> dict:
    layout_all(g)
    return {}


_OPS["layout"] = op_layout
OP_NAMES = sorted(_OPS)


def _layered(g: Graph) -> Dict[str, Tuple[float, float]]:
    order = _topo(g)
    if not order:
        return {}
    ins: Dict[str, List[Tuple[str, float]]] = {}            # node -> [(source, x offset of the port)]
    outs: Dict[str, List[Tuple[str, float]]] = {}           # source -> [(consumer, port offset)]
    for nid in order:
        nt = g.registry.get(g.nodes[nid].type) if g.nodes[nid].type in g.registry else None
        names = nt.input_names() if nt else list(g.nodes[nid].inputs)
        for i, port in enumerate(names):
            ref = g.nodes[nid].inputs.get(port)
            off = (i + 1) / (len(names) + 1) * 160 - 80
            for src in ([ref] if isinstance(ref, str) else (ref or [])):
                if src in g.nodes:
                    ins.setdefault(nid, []).append((src, off))
                    outs.setdefault(src, []).append((nid, off))
    depth: Dict[str, int] = {}
    for nid in order:                                        # longest path from a source...
        depth[nid] = 1 + max((depth[s] for s, _ in ins.get(nid, ()) if s in depth), default=-1)
    for nid in reversed(order):                              # ...then pull side chains down next to their consumer
        if outs.get(nid):
            depth[nid] = max(depth[nid], min(depth[c] for c, _ in outs[nid]) - 1)
    rows: Dict[int, List[str]] = {}
    for nid in order:
        rows.setdefault(depth[nid], []).append(nid)
    x: Dict[str, float] = {}
    for d, ids in rows.items():
        for k, nid in enumerate(ids):
            x[nid] = k * GRID_X

    def place(ids: List[str], want: Dict[str, float]) -> None:
        ids.sort(key=lambda n: want[n])
        xs: List[float] = []
        for nid in ids:
            v = want[nid]
            if xs and v < xs[-1] + GRID_X:
                v = xs[-1] + GRID_X
            xs.append(v)
        shift = sum(want[n] - v for n, v in zip(ids, xs)) / len(ids)
        for nid, v in zip(ids, xs):
            x[nid] = v + shift

    levels = sorted(rows)
    for sweep in ("down", "up", "down"):                     # a few barycentre sweeps: fewer crossings, straight chains
        for d in (levels[1:] if sweep == "down" else reversed(levels[:-1])):
            ids = rows[d]
            want = {}
            for nid in ids:
                links = ins.get(nid, ()) if sweep == "down" else outs.get(nid, ())
                pts = [x[o] - off if sweep == "down" else x[o] + off for o, off in links if o in x]
                want[nid] = sum(pts) / len(pts) if pts else x[nid]
            place(ids, want)
    return {nid: (x[nid], depth[nid] * GRID_Y) for nid in order}


def _topo(g: Graph) -> List[str]:
    """Node ids, inputs before consumers where possible (ignores missing refs, tolerates cycles)."""
    out: List[str] = []
    seen = set()
    for root in g.nodes:
        if root in seen:
            continue
        seen.add(root)
        stack = [(root, iter(g.input_ids(root)))]
        while stack:                                   # iterative DFS: graphs can be deep
            nid, it = stack[-1]
            for s in it:
                if s in g.nodes and s not in seen:
                    seen.add(s)
                    stack.append((s, iter(g.input_ids(s))))
                    break
            else:
                out.append(nid)
                stack.pop()
    return out
