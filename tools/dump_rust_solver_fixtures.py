"""Refresh the Rust solver/goal/RBD-attribute unit-test oracles from the Python implementation (no Rust build).

Each case is a graph, a frame and a target; the Rust test (``nodes/solver_parity.rs``) cooks the same graph at the same
frame and compares topology, attributes and detail with the Python result. Run::

    py tools/dump_rust_solver_fixtures.py
"""
import json
import sys
from pathlib import Path

import numpy as np

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
from minihou import Evaluator, Graph, accel, rust  # noqa: E402
from minihou.sdk import REGISTRY  # noqa: E402


def encode(g):
    tables = {}
    for domain in ('point', 'vertex', 'prim'):
        tables[domain] = {name: {'width': 1 if a.ndim == 1 else a.shape[1],
                                 'dtype': 'float32' if a.dtype.kind == 'f' else 'int32',
                                 'data': a.reshape(-1).tolist()}
                          for name, a in g.table(domain).items()}
    return dict(vp=g.topology.vertex_point.tolist(), offsets=g.topology.prim_offsets.tolist(),
                closed=g.topology.closed_mask().tolist(), detail=json.loads(json.dumps(g.detail, default=list)), **tables)


def n(t, params=None, **inputs):
    return {"type": t, "params": params or {}, "inputs": inputs}


def main():
    cases = []

    def add(name, nodes, target, frame=1.0, error=False):
        cases.append(dict(name=name, graph={"nodes": nodes}, target=target, frame=frame, error=error))

    line = n("line", {"origin": [0, 0, 0], "direction": [1, 0, 0], "length": 4, "points": 9})
    grid = n("grid", {"columns": 5, "rows": 5, "size_x": 2, "size_y": 2, "center": [0, 0, 1]})
    target = n("grid", {"columns": 3, "rows": 3, "size_x": 4, "size_y": 4})
    # geometric goals
    add("cable", {"l": line, "len": n("goal_length", {"scale": 1.1}, geo="l"), "g": n("goal_gravity", geo="len"),
                  "pin": n("goal_pin", {"group": "0 8"}, geo="g"), "s": n("goal_solve", {"steps": 200, "iterations": 30}, geo="pin")}, "s")
    add("length_pin", {"l": line, "len": n("goal_length", {"scale": 0.5, "strength": 0.7}, geo="l"),
                       "pin": n("goal_pin", {"group": "0"}, geo="len"), "s": n("goal_solve", {"steps": 20, "iterations": 10}, geo="pin")}, "s")
    add("surface_collision", {"p": grid, "t": target, "surf": n("goal_surface", {"group": "0-12", "strength": 0.5}, geo="p", target="t"),
                              "c": n("goal_collision", {"radius": 0.3}, geo="surf"),
                              "s": n("goal_solve", {"steps": 30, "iterations": 6, "damping": 0.5}, geo="c")}, "s")
    add("goal_chain_detail", {"p": grid, "len": n("goal_length", geo="p"), "g": n("goal_gravity", {"acceleration": [0, 1, -2]}, geo="len"),
                              "pin": n("goal_pin", {"group": "0-4"}, geo="g")}, "pin")
    fabric = json.loads((ROOT / "examples" / "procedural_fabric.json").read_text(encoding="utf-8"))
    add("example_fabric", fabric["nodes"], "relax")
    # solver
    up = json.dumps({"translate": [0, 0, 0.5], "rotate": [0, 0, 10]})
    for frame, start, sub in ((1.0, 1, 1), (7.0, 1, 1), (7.6, 1, 3), (12.0, 10, 2)):
        add(f"solver_f{frame}_s{start}_x{sub}", {"p": grid, "s": n("solver", {"body": "transform", "params": up, "start_frame": start,
                                                                                "substeps": sub}, geo="p")}, "s", frame)
    # rbd attribute nodes
    box = n("box", {"center": [0, 0, 3]})
    add("rbd_configure", {"b": box, "c": n("rbd_configure", {"id_offset": 3, "mass": 2.5, "friction": 0.3, "velocity": [1, 0, 0],
                                                             "angular_velocity": [0, 0, 2], "compound": True}, geo="b")}, "c")
    add("rbd_constraint_spring", {"c": n("rbd_constraint", {"kind": "spring", "anchor": [0, 0, 1], "anchor_b": [0, 1, 3],
                                                            "limited": True, "lower": 0.5, "upper": 3, "strength": 7})}, "c")
    add("rbd_constraint_hinge", {"c": n("rbd_constraint", {"kind": "hinge", "world": True, "axis": [1, 0, 0], "limited": True,
                                                           "lower": -20, "upper": 30, "break_torque": 4})}, "c")
    links = {"a": n("rbd_constraint", {"strength": 10}), "b": n("rbd_constraint", {"kind": "pin", "body_a": 2, "body_b": 3, "strength": 4}),
             "m": {"type": "merge", "params": {}, "inputs": {"geos": ["a", "b"]}}}
    add("rbd_constraint_properties", dict(links, p=n("rbd_constraint_properties", {"mode": "scale", "strength": 3, "variation": 0.4,
                                                                                     "seed": 11, "group": "1", "outgrp_changed": True},
                                                       constraints="m")), "p")
    out = []
    with rust.disabled(), accel.disabled():
        for c in cases:
            g = Graph.from_dict(c["graph"])
            try:
                result = Evaluator().cook(g, c["target"], frame=c["frame"])
                result.validate()
                c["expected"] = encode(result)
            except Exception as exc:  # noqa: BLE001 - recorded for the Rust side
                if not c["error"]:
                    raise
                c["expected_error"] = str(exc)
            out.append(c)
    types = sorted({nd["type"] for c in cases for nd in c["graph"]["nodes"].values()})
    for t in types:
        assert REGISTRY.get(t) is not None, t
    dest = ROOT / "rust/crates/minihou-core/src/nodes/test_data/solver_parity.json"
    dest.parent.mkdir(exist_ok=True)
    dest.write_text(json.dumps(out, separators=(",", ":")) + "\n", encoding="utf-8")
    print(f"wrote {len(out)} Python solver oracles to {dest}")


if __name__ == "__main__":
    main()
