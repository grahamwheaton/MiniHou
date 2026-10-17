"""Small Python oracle summaries for the final Rust node ports.

Run from the repository root: python tools/dump_rust_last_nodes_fixtures.py
"""
import json
from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import numpy as np

from minihou import Evaluator, Graph
from minihou.geometry import Geometry
from minihou.nodes.fracture_tools import convex_decomposition, mesh_volume, rbd_material_fracture


def l_prism():
    xy = [(0, 0), (2, 0), (2, 1), (1, 1), (1, 2), (0, 2)]
    points = np.array([[x, y, z] for z in (0, 1) for x, y in xy], np.float32)
    faces = [list(reversed(range(6))), list(range(6, 12))]
    faces += [[i, (i + 1) % 6, (i + 1) % 6 + 6, i + 6] for i in range(6)]
    return Geometry.from_arrays(points, np.array(sum(faces, []), np.int32),
                                np.cumsum([0] + [len(f) for f in faces]).astype(np.int32))


def material_case(geo, **params):
    from time import perf_counter
    args = dict(material="concrete", density=1000, friction=.6, restitution=.1,
                glue_strength=1e7, pieces=2, fracture_level=1, sub_pieces=4, seed=2,
                surface_fraction=0, relax=0, gap=0, convex_parts=False,
                max_concavity=.05, interior_detail=True, detail_divisions=2,
                detail_amplitude=0, detail_frequency=4, glue=False, clusters=0,
                strength_scale=1, mass_scale=1, output="pieces",
                outgrp_inside=True, outgrp_inside_name="inside",
                outgrp_outside=False, outgrp_outside_name="outside",
                outgrp_cutpoints=True, outgrp_cutpoints_name="cutpoints",
                outgrp_cutedges=True, outgrp_cutedges_name="cutedges")
    args.update(params)
    start = perf_counter()
    out = rbd_material_fracture(geo, **args)
    ins = out.prim.get("group_inside", np.zeros(out.nprims, np.int32))
    uv = out.vertex.get("uv")
    return {"points": out.npoints, "prims": out.nprims,
            "pieces": np.bincount(out.prim["piece"]).tolist(),
            "inside": int(ins.sum()),
            "cutpoints": int(out.point.get("group_cutpoints", np.array([])).sum()),
            "materials": np.bincount(out.prim["material_index"]).tolist() if "material_index" in out.prim else [],
            "uv_finite_corners": int(np.isfinite(uv).all(axis=1).sum()) if uv is not None else None,
            "time_s": round(perf_counter() - start, 4)}


def main():
    if "--stress" in sys.argv:
        mesh = Evaluator().cook(Graph.from_dict({"nodes": {"s": {"type": "sphere", "params": {"rows": 36, "columns": 72}}}, "display": "s"}))
        print(json.dumps(material_case(mesh, pieces=24, surface_fraction=.15, relax=2,
                                       detail_amplitude=.02), indent=2))
        return
    graph = Graph.from_dict({"nodes": {
        "b": {"type": "box"},
        "f": {"type": "rbd_material_fracture", "inputs": {"geo": "b"},
              "params": {"pieces": 2, "seed": 2, "surface_fraction": 0, "relax": 0, "glue": False}},
    }, "display": "f"})
    material = Evaluator().cook(graph)
    box = Evaluator().cook(Graph.from_dict({"nodes": {"b": {"type": "box"}}, "display": "b"}))
    hull = convex_decomposition(box, .02, 32, .001, 16, "piece",
                                outgrp_inside=True, outgrp_inside_name="inside",
                                outgrp_outside=False, outgrp_outside_name="outside",
                                outgrp_cutpoints=False, outgrp_cutpoints_name="cutpoints",
                                outgrp_cutedges=False, outgrp_cutedges_name="cutedges")
    prism = l_prism()
    prism_out = convex_decomposition(prism, .02, 4, .001, 16, "piece",
                                     outgrp_inside=True, outgrp_inside_name="inside",
                                     outgrp_outside=False, outgrp_outside_name="outside",
                                     outgrp_cutpoints=False, outgrp_cutpoints_name="cutpoints",
                                     outgrp_cutedges=False, outgrp_cutedges_name="cutedges")
    data = {
        "material_box": {"points": material.npoints, "prims": material.nprims,
                         "pieces": np.bincount(material.prim["piece"]).tolist(),
                         "mass": [float(material.prim["rbd_mass"][0]), float(material.prim["rbd_mass"][-1])],
                         "prim_attributes": sorted(material.prim), "material": material.detail["rbd_material"]},
        "convex_box": {"points": hull.npoints, "prims": hull.nprims,
                       "pieces": np.bincount(hull.prim["piece"]).tolist(),
                       "inside": int(hull.prim["group_inside"].sum())},
        "l_prism": {"input_volume": mesh_volume(prism), "output_volume": mesh_volume(prism_out),
                    "python_pieces": int(prism_out.prim["piece"].max()) + 1},
    }
    painted = box.copy()
    painted.prim["material_index"] = np.array([0, 1, 0, 1, 0, 1], np.int32)
    painted.detail = dict(painted.detail, materials=["a", "b"])
    painted.vertex["uv"] = np.tile(np.array([[0, 0], [1, 0], [1, 1], [0, 1]], np.float32), (6, 1))
    sphere = Evaluator().cook(Graph.from_dict({"nodes": {"s": {"type": "sphere", "params": {"rows": 8, "columns": 12}}}, "display": "s"}))
    data["material_interior"] = {
        "box": material_case(box),
        "sphere": material_case(sphere),
        "l_prism": material_case(prism),
        "two_materials": material_case(painted),
        "box_convex": material_case(box, convex_parts=True, interior_detail=False),
        "l_prism_convex": material_case(prism, convex_parts=True, max_concavity=0,
                                        interior_detail=False),
        "l_prism_convex_detail": material_case(prism, convex_parts=True, max_concavity=0),
    }
    path = Path(__file__).resolve().parents[1] / "rust/crates/minihou-core/src/nodes/test_data/last_nodes_parity.json"
    path.write_text(json.dumps(data, indent=2, sort_keys=True) + "\n")
    print(path)


if __name__ == "__main__":
    main()
