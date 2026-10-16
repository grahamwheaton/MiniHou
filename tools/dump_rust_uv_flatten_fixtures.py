"""Dump uv_flatten parity fixtures from the Python implementation to test_data/uv_flatten_parity.json."""
import json
import os
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, ROOT)

import numpy as np
from minihou import Evaluator, Graph, NodeError
from minihou.geometry import Geometry
from minihou.sdk import REGISTRY


def encode(g):
    tables = {}
    for domain in ('point', 'vertex', 'prim'):
        tables[domain] = {name: {'width': 1 if a.ndim == 1 else a.shape[1],
                                'dtype': 'float32' if a.dtype.kind == 'f' else 'int32',
                                'data': a.reshape(-1).tolist()}
                          for name, a in g.table(domain).items()}
    return dict(vp=g.topology.vertex_point.tolist(), offsets=g.topology.prim_offsets.tolist(),
                closed=g.topology.closed_mask().tolist(), detail=dict(g.detail), **tables)


def cook_node(src, **params):
    spec = REGISTRY.get("uv_flatten")
    nodes = {"s": src, "u": {"type": "uv_flatten", "inputs": {"geo": "s"}, "params": params}, "out": {"type": "output", "inputs": {"geo": "u"}}}
    return Evaluator().cook(Graph.from_dict({"nodes": nodes, "display": "out"}))


def main():
    spec = REGISTRY.get("uv_flatten")
    defaults = {p.name: p.to_json_value(p.default) for p in spec.params}

    cases = []

    # 1. Grid 3x3
    src_grid = {"type": "grid", "params": {"columns": 3, "rows": 3, "size_x": 2.0, "size_y": 2.0}}
    g_in = Evaluator().cook(Graph.from_dict({"nodes": {"s": src_grid, "out": {"type": "output", "inputs": {"geo": "s"}}}, "display": "out"}))
    g_out = cook_node(src_grid)
    cases.append({
        "name": "grid_3x3",
        "type": "uv_flatten",
        "params": {},
        "defaults": defaults,
        "inputs": [encode(g_in)],
        "expected": encode(g_out)
    })

    # 2. Grid 4x2
    src_grid42 = {"type": "grid", "params": {"columns": 5, "rows": 3, "size_x": 4.0, "size_y": 2.0}}
    g_in = Evaluator().cook(Graph.from_dict({"nodes": {"s": src_grid42, "out": {"type": "output", "inputs": {"geo": "s"}}}, "display": "out"}))
    g_out = cook_node(src_grid42)
    cases.append({
        "name": "grid_4x2",
        "type": "uv_flatten",
        "params": {},
        "defaults": defaults,
        "inputs": [encode(g_in)],
        "expected": encode(g_out)
    })

    # 3. Box (divisions=1: 6 quads)
    src_box = {"type": "box", "params": {"divisions": 1}}
    g_in = Evaluator().cook(Graph.from_dict({"nodes": {"s": src_box, "out": {"type": "output", "inputs": {"geo": "s"}}}, "display": "out"}))
    g_out = cook_node(src_box)
    cases.append({
        "name": "box_div1",
        "type": "uv_flatten",
        "params": {},
        "defaults": defaults,
        "inputs": [encode(g_in)],
        "expected": encode(g_out)
    })

    # 4. Sphere (rows=4, columns=6)
    src_sphere = {"type": "sphere", "params": {"rows": 4, "columns": 6}}
    g_in = Evaluator().cook(Graph.from_dict({"nodes": {"s": src_sphere, "out": {"type": "output", "inputs": {"geo": "s"}}}, "display": "out"}))
    g_out = cook_node(src_sphere)
    cases.append({
        "name": "sphere_small",
        "type": "uv_flatten",
        "params": {},
        "defaults": defaults,
        "inputs": [encode(g_in)],
        "expected": encode(g_out)
    })

    # 5. Tube (caps=False, rows=3, columns=6)
    src_tube = {"type": "tube", "params": {"rows": 3, "columns": 6, "caps": False}}
    g_in = Evaluator().cook(Graph.from_dict({"nodes": {"s": src_tube, "out": {"type": "output", "inputs": {"geo": "s"}}}, "display": "out"}))
    g_out = cook_node(src_tube)
    cases.append({
        "name": "tube_open",
        "type": "uv_flatten",
        "params": {},
        "defaults": defaults,
        "inputs": [encode(g_in)],
        "expected": encode(g_out)
    })

    # 6. Split attribute on grid
    nodes_split = {
        "g": {"type": "grid", "params": {"columns": 3, "rows": 3}},
        "w": {"type": "wrangle", "inputs": {"geo": "g"}, "params": {"snippet": "i@side = @P.x > 0", "run_over": "primitives"}},
        "out": {"type": "output", "inputs": {"geo": "w"}}
    }
    g_split_in = Evaluator().cook(Graph.from_dict({"nodes": nodes_split, "display": "out"}))
    nodes_split_u = dict(nodes_split)
    nodes_split_u["u"] = {"type": "uv_flatten", "inputs": {"geo": "w"}, "params": {"split_attrib": "side"}}
    nodes_split_u["out"] = {"type": "output", "inputs": {"geo": "u"}}
    g_split_out = Evaluator().cook(Graph.from_dict({"nodes": nodes_split_u, "display": "out"}))
    cases.append({
        "name": "split_attrib",
        "type": "uv_flatten",
        "params": {"split_attrib": "side"},
        "defaults": defaults,
        "inputs": [encode(g_split_in)],
        "expected": encode(g_split_out)
    })

    # 7. Refusal: missing split attribute
    cases.append({
        "name": "refusal_missing_attrib",
        "type": "uv_flatten",
        "params": {"split_attrib": "nope"},
        "defaults": defaults,
        "inputs": [encode(g_in)],
        "error": "no primitive attribute 'nope' to split islands by"
    })

    # 8. Refusal: open curve input (helix)
    helix_src = {"type": "helix", "params": {}}
    g_helix = Evaluator().cook(Graph.from_dict({"nodes": {"s": helix_src, "out": {"type": "output", "inputs": {"geo": "s"}}}, "display": "out"}))
    cases.append({
        "name": "refusal_open_curves",
        "type": "uv_flatten",
        "params": {},
        "defaults": defaults,
        "inputs": [encode(g_helix)],
        "error": "uv_flatten works on polygons, but the input has 1 open curve(s); remove them first, e.g. blast with group '@closed == 0'"
    })

    target = os.path.join(ROOT, "rust", "crates", "minihou-core", "src", "nodes", "test_data", "uv_flatten_parity.json")
    with open(target, "w", encoding="utf-8") as f:
        json.dump(cases, f, indent=2)
    print(f"Wrote {len(cases)} cases to {target}")


if __name__ == "__main__":
    main()
