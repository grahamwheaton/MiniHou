"""Buttons on a node: things a node does when asked, not when it cooks (Houdini's "Save to Disk" and "Reload" buttons).

A node type declares its buttons with ``node(..., actions=[Action("save", "Save to Disk", doc=...)])``; the editor shows them in the
parameter panel. The behaviour is a handler registered here with :func:`action`; it gets the live session, the node's id and its
effective parameters, and returns a JSON-able dict (the editor shows its ``message``). Cooking never runs an action, so a graph can
be cooked, cached and fuzzed without side effects.
"""
from __future__ import annotations

from typing import Callable, Dict, Tuple

HANDLERS: Dict[Tuple[str, str], Callable] = {}


def action(node_type: str, name: str):
    """Decorator: ``fn(session, node_id, params) -> dict`` runs when the button ``name`` of a ``node_type`` node is pressed."""
    def deco(fn: Callable) -> Callable:
        HANDLERS[(node_type, name)] = fn
        return fn
    return deco


def run(session, node_id: str, name: str) -> dict:
    """Press button ``name`` of node ``node_id``. Raises ``GraphError`` for an unknown node or button."""
    from .graph import GraphError
    with session.lock:
        graph = session.graph.copy()
    if node_id not in graph.nodes:
        raise GraphError(f"no such node {node_id!r}")
    ntype = graph.nodes[node_id].type
    nt = graph.registry.get(ntype)
    if name not in [a.name for a in getattr(nt, "actions", ())] or (ntype, name) not in HANDLERS:
        raise GraphError(f"node type {ntype!r} has no button {name!r}")
    return HANDLERS[(ntype, name)](session, node_id, graph.effective_params(node_id)) or {}
