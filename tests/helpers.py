"""Shared test utilities."""
import numpy as np

import minihou  # noqa: F401  (registers built-in nodes)
from minihou import Geometry, Graph
from minihou.sdk import Float, Input, Registry, Str, node


def quad_geo(x0=0.0, y0=0.0, size=1.0):
    P = np.array([[x0, y0, 0], [x0 + size, y0, 0], [x0 + size, y0 + size, 0], [x0, y0 + size, 0]], np.float32)
    return Geometry.from_arrays(P, [0, 1, 2, 3], [0, 4])


def graph_from(nodes, display=None, exposed=None, registry=None):
    d = {"version": 1, "nodes": nodes}
    if display:
        d["display"] = display
    if exposed:
        d["exposed"] = exposed
    return Graph.from_dict(d, registry)


def counting_registry():
    """A private registry with nodes that record how many times they ran."""
    reg = Registry()
    calls = []

    @node("src", params=[Float("x", 0.0)], registry=reg)
    def src(x):
        calls.append(("src", x))
        return Geometry.from_arrays(np.array([[x, 0, 0]], np.float32))

    @node("add", inputs=["geo"], params=[Float("dx", 0.0)], registry=reg)
    def add(geo, dx):
        calls.append(("add", dx))
        out = geo.copy()
        out.point["P"] = geo.P + np.float32(dx)
        return out

    @node("join", inputs=[Input("a"), Input("b")], registry=reg)
    def join(a, b):
        calls.append(("join",))
        return Geometry.from_arrays(np.concatenate([a.P, b.P]))

    return reg, calls
