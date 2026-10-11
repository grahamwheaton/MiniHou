"""Structural repeat-block validation. Pairing is an ordinary DAG edge, not metadata.

Scopes propagate down geometry edges. External inputs may enter a deeper scope,
but sibling scopes cannot mix and End can close only the innermost Begin.
"""
from contextlib import contextmanager
from contextvars import ContextVar

from .graph import GraphError

MAX_DEPTH = 16
MAX_TOTAL_ITERATIONS = 10000
_budget = ContextVar("repeat_budget", default=None)


@contextmanager
def cook_budget():
    token = _budget.set([MAX_TOTAL_ITERATIONS]) if _budget.get() is None else None
    try:
        yield
    finally:
        if token is not None:
            _budget.reset(token)


def consume_iteration(nid):
    remaining = _budget.get()
    if remaining[0] <= 0:
        raise GraphError(f"Loop End {nid!r}: repeat budget exceeded ({MAX_TOTAL_ITERATIONS} total iterations per cook)", nid)
    remaining[0] -= 1


def analyze(graph, order, target=None):
    scopes, pairs = {}, {}
    for nid in order:
        node = graph.nodes[nid]
        incoming = [scopes.get(src, ()) for src in graph.input_ids(nid)]
        scope = max(incoming, key=len, default=())
        if any(scope[:len(s)] != s for s in incoming):
            raise GraphError(f"node {nid!r}: crossed loop blocks; close each block before combining results", nid)
        if node.type == "loop_begin":
            if not isinstance(node.inputs.get("geo"), str):
                raise GraphError(f"Loop Begin {nid!r}: geo input must be a single initial-geometry node", nid)
            scope = (*scope, nid)
            if len(scope) > MAX_DEPTH:
                raise GraphError(f"loop nesting exceeds {MAX_DEPTH} blocks", nid)
        elif node.type == "loop_end":
            begin = node.inputs.get("begin")
            if not isinstance(begin, str) or begin not in graph.nodes or graph.nodes[begin].type != "loop_begin":
                raise GraphError(f"Loop End {nid!r}: begin input must connect directly to a Loop Begin", nid)
            if begin in pairs:
                raise GraphError(f"Loop Begin {begin!r} is paired with multiple Ends ({pairs[begin]!r}, {nid!r})", nid)
            body = node.inputs.get("geo")
            if not isinstance(body, str):
                raise GraphError(f"Loop End {nid!r}: geo input must be a single body node", nid)
            body_scope = scopes.get(body, ())
            if not body_scope or body_scope[-1] != begin or body_scope != scopes.get(begin):
                raise GraphError(f"Loop End {nid!r}: crossed or disconnected body; close innermost Begin {begin!r} first", nid)
            pairs[begin] = nid
            scope = body_scope[:-1]
        scopes[nid] = scope
    for nid in order:
        if graph.nodes[nid].type == "loop_begin" and nid not in pairs:
            raise GraphError(f"Loop Begin {nid!r} is unpaired; connect a Loop End and display the End, not the body", nid)
        if graph.nodes[nid].type == "output" and scopes[nid]:
            raise GraphError(f"output {nid!r} escapes a loop body; connect the paired Loop End instead", nid)
    if target is not None and scopes.get(target):
        raise GraphError("display is inside a loop body; display the paired Loop End to evaluate the repeat", target)
    return pairs
