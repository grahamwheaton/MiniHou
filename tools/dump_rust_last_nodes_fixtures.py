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
from minihou.nodes.fracture_tools import convex_decomposition, mesh_volume


def l_prism():
    xy = [(0, 0), (2, 0), (2, 1), (1, 1), (1, 2), (0, 2)]
    points = np.array([[x, y, z] for z in (0, 1) for x, y in xy], np.float32)
    faces = [list(reversed(range(6))), list(range(6, 12))]
    faces += [[i, (i + 1) % 6, (i + 1) % 6 + 6, i + 6] for i in range(6)]
    return Geometry.from_arrays(points, np.array(sum(faces, []), np.int32),
                                np.cumsum([0] + [len(f) for f in faces]).astype(np.int32))


def main():
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
    path = Path(__file__).resolve().parents[1] / "rust/crates/minihou-core/src/nodes/test_data/last_nodes_parity.json"
    path.write_text(json.dumps(data, indent=2, sort_keys=True) + "\n")
    print(path)


if __name__ == "__main__":
    main()
