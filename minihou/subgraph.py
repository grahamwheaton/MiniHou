"""Turn part of a graph into a reusable system (a graph file in the node library) and swap the selection for it.

``extract`` builds the system's graph: the selected nodes, an ``input`` node for every outside node they read, one
``output`` fed by the node that leads out. ``replace_ops`` gives the edit operations that remove the selection and put the new
node, wired to the same neighbours, in its place (one batch, so one undo).
"""
from __future__ import annotations

import re
from typing import Dict, List, Optional, Tuple

from .graph import Graph, GraphError

NAME = re.compile(r"^[A-Za-z_][A-Za-z0-9_]*$")
_FORBIDDEN = {"rbd_solver": "RBD Solver must stay top-level (a system cannot own the simulation), so it cannot go inside a system",
              "blender_object": "a Blender Object node cannot be inside a system: give the system an input and connect the "
                                "Blender Object to it in the main graph",
              "input": "the selection contains an input node of this graph, which belongs to the graph itself",
              "output": "the selection contains the output node, which belongs to the graph itself"}


def port_name(src: str, taken: set) -> str:
    """A port name for the outside node ``src``: its id, made valid and unique."""
    base = re.sub(r"[^A-Za-z0-9_]+", "_", src).strip("_") or "geo"
    if base[0].isdigit():
        base = "in_" + base
    name, k = base, 2
    while name in taken:
        name, k = f"{base}{k}", k + 1
    taken.add(name)
    return name


def analyse(graph: Graph, ids: List[str]) -> dict:
    """What leaves and enters the selection: the outside sources it reads (in a stable order), the inside node that feeds the rest of
    the graph, and the places it feeds (``(consumer, port, was a list)``). Raises GraphError for a selection that cannot be a system."""
    chosen = [i for i in ids if i in graph.nodes]
    if not chosen:
        raise GraphError("select at least one node")
    S = set(chosen)
    for nid in chosen:
        t = graph.nodes[nid].type
        if t in _FORBIDDEN:
            raise GraphError(f"{nid}: {_FORBIDDEN[t]}")
    for nid in chosen:                                              # a loop only works with both its ends inside
        inst = graph.nodes[nid]
        if inst.type == "loop_end" and inst.inputs.get("begin") not in S:
            raise GraphError(f"{nid}: its Loop Begin is not in the selection; select the whole loop")
        if inst.type == "loop_begin" and not any(n.type == "loop_end" and n.inputs.get("begin") == nid for i, n in graph.nodes.items() if i in S):
            raise GraphError(f"{nid}: its Loop End is not in the selection; select the whole loop")
    sources: List[str] = []
    for nid in chosen:                                              # outside nodes read by the selection, in selection order
        for ref in graph.nodes[nid].inputs.values():
            for r in (ref if isinstance(ref, list) else [ref]):
                if r and r not in S and r not in sources:
                    sources.append(r)
    feeds: List[Tuple[str, str, int]] = []                          # (consumer, port, position in a list input or -1)
    leaving_set = set()
    for cid, c in graph.nodes.items():
        if cid in S:
            continue
        for port, ref in c.inputs.items():
            for k, r in enumerate(ref if isinstance(ref, list) else [ref]):
                if r in S:
                    feeds.append((cid, port, k if isinstance(ref, list) else -1))
                    leaving_set.add(r)
    leaving = sorted(leaving_set)
    if len(leaving) > 1:
        raise GraphError(f"the selection feeds the rest of the graph from {len(leaving)} different nodes ({', '.join(leaving)}); a "
                         f"system has one output: select so that only one node leads out, or merge them first")
    if leaving:
        out = leaving[0]
    else:                                                           # nothing reads it: the display node, or the one node nothing inside reads
        used = {r for nid in chosen for ref in graph.nodes[nid].inputs.values() for r in (ref if isinstance(ref, list) else [ref]) if r in S}
        tails = [n for n in chosen if n not in used]
        if graph.display in S:
            out = graph.display
        elif len(tails) == 1:
            out = tails[0]
        else:
            raise GraphError(f"cannot tell which node is the result ({', '.join(tails)} all end the selection): select a chain with one end")
    return {"chosen": chosen, "sources": sources, "out": out, "feeds": feeds}


def extract(graph: Graph, ids: List[str], doc: str = "") -> Tuple[dict, List[Tuple[str, str]]]:
    """``(system graph as a dict, [(port name, outside source id)])`` for the selection."""
    a = analyse(graph, ids)
    S = set(a["chosen"])
    taken: set = set()
    ports = {src: port_name(src, taken) for src in a["sources"]}
    nodes: Dict[str, dict] = {}
    xs = [graph.nodes[n].meta.get("pos", [0, 0])[0] for n in a["chosen"]]
    ys = [graph.nodes[n].meta.get("pos", [0, 0])[1] for n in a["chosen"]]
    x0, y0 = (min(xs), min(ys)) if xs else (0, 0)
    for nid in a["chosen"]:
        inst = graph.nodes[nid]
        d: dict = {"type": inst.type}
        if inst.params:
            d["params"] = dict(inst.params)
        ins = {}
        for port, ref in inst.inputs.items():
            if isinstance(ref, list):
                ins[port] = [("in_" + ports[r]) if r not in S else r for r in ref]
            elif ref:
                ins[port] = ("in_" + ports[ref]) if ref not in S else ref
        if ins:
            d["inputs"] = ins
        pos = inst.meta.get("pos", [x0, y0])
        d["pos"] = [round(pos[0] - x0, 1), round(pos[1] - y0 + 110, 1)]
        if inst.meta.get("comment"):
            d["comment"] = inst.meta["comment"]
        nodes[nid] = d
    for k, (src, port) in enumerate(ports.items()):
        nodes["in_" + port] = {"type": "input", "params": {"name": port, "index": k}, "pos": [round(k * 190.0, 1), 0.0]}
    nodes["result"] = {"type": "output", "inputs": {"geo": a["out"]}, "pos": [0.0, round(max((n["pos"][1] for n in nodes.values()), default=0) + 110, 1)]}
    data = {"version": 1, "nodes": nodes, "display": "result"}
    exposed = [{k: v for k, v in (("name", e.name), ("node", e.node), ("param", e.param), ("label", e.label)) if v}
               for e in graph.exposed if e.node in S]
    if exposed:
        data["exposed"] = exposed
    if doc:
        data["doc"] = doc
    return data, [(port, src) for src, port in ports.items()]


def replace_ops(graph: Graph, ids: List[str], system_type: str, new_id: Optional[str] = None) -> List[dict]:
    """Edit operations that swap the selection for one ``system_type`` node wired to the same neighbours."""
    a = analyse(graph, ids)
    S = set(a["chosen"])
    _data, ports = extract(graph, ids)
    xs = [graph.nodes[n].meta.get("pos", [0, 0])[0] for n in a["chosen"]]
    ys = [graph.nodes[n].meta.get("pos", [0, 0])[1] for n in a["chosen"]]
    nid = new_id or _free_id(graph, system_type)
    add = {"op": "add", "type": system_type, "id": nid, "pos": [round(sum(xs) / len(xs), 1), round(sum(ys) / len(ys), 1)],
           "inputs": {port: src for port, src in ports}}
    if graph.display in S:
        add["display"] = True
    ops: List[dict] = [add]
    for cid, port, k in a["feeds"]:
        ops.append({"op": "disconnect", "to": cid, "port": port, "from": a["out"]})
        ops.append({"op": "connect", "from": nid, "to": cid, "port": port, **({"index": k} if k >= 0 else {})})
    ops.append({"op": "remove", "id": list(a["chosen"])})
    return ops


def _free_id(graph: Graph, base: str) -> str:
    if base not in graph.nodes:
        return base
    k = 1
    while f"{base}{k}" in graph.nodes:
        k += 1
    return f"{base}{k}"
