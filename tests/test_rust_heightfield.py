"""Parity and integration tests for heightfield_erode kernels in the Rust core."""
import unittest

import numpy as np

import minihou  # noqa: F401
from minihou import Evaluator, Geometry, Graph, rust
from minihou.nodes import heightfield
from minihou.geometry import Topology


@unittest.skipUnless(rust.available(), "the Rust core is not built (see rust/README.md)")
class RustHeightfieldTests(unittest.TestCase):
    def test_thermal_parity(self):
        cases = [
            # (rows, cols, cell, talus, rate, iterations, seed)
            (2, 2, (1.0, 1.0), 35.0, 0.5, 1, 1),
            (3, 7, (0.5, 2.0), 30.0, 0.3, 3, 2),
            (7, 3, (2.0, 0.5), 45.0, 0.7, 2, 3),
            (16, 24, (1.2, 0.8), 35.0, 0.5, 0, 4),  # 0 iterations
            (16, 24, (1.2, 0.8), 35.0, 0.5, 1, 5),  # 1 iteration
            (16, 24, (1.2, 0.8), 35.0, 0.5, 5, 6),  # 5 iterations
            (64, 64, (1.0, 1.0), 40.0, 0.6, 3, 7),
            (257, 129, (0.25, 0.5), 33.0, 0.4, 2, 8),
        ]
        for rows, cols, cell, talus, rate, iterations, seed in cases:
            with self.subTest(size=(rows, cols), iters=iterations, seed=seed):
                rng = np.random.default_rng(seed)
                H = rng.uniform(0.0, 100.0, size=(rows, cols)).astype(np.float64)
                h_py = heightfield.thermal(H.copy(), cell, talus, rate, iterations)
                h_rs = rust.thermal(H.copy(), cell, talus, rate, iterations)
                np.testing.assert_array_equal(h_py, h_rs)

    def test_hydraulic_parity(self):
        cases = [
            # (rows, cols, droplets, iterations, inertia, capacity, seed)
            (2, 2, 1, 1, 0.1, 4.0, 10),
            (3, 7, 5, 2, 0.0, 2.0, 20),
            (7, 3, 10, 2, 0.9, 8.0, 30),
            (16, 16, 100, 0, 0.1, 4.0, 40),  # 0 iterations
            (16, 16, 1, 1, 0.0, 4.0, 41),    # 1 droplet, inertia 0
            (16, 16, 100, 1, 0.9, 4.0, 42),  # 100 droplets, inertia 0.9
            (16, 16, 100, 5, 0.1, 0.001, 43),  # tiny capacity
            (16, 16, 100, 5, 0.1, 50.0, 44),   # large capacity
            (64, 64, 100, 2, 0.1, 4.0, 50),
            (64, 64, 4096, 1, 0.1, 4.0, 51),
            (257, 129, 100, 1, 0.15, 3.5, 60),
        ]
        for rows, cols, droplets, iterations, inertia, capacity, seed in cases:
            with self.subTest(size=(rows, cols), droplets=droplets, iters=iterations, inertia=inertia, cap=capacity):
                rng = np.random.default_rng(seed)
                H = rng.uniform(0.0, 50.0, size=(rows, cols)).astype(np.float64)
                cell = (1.5, 0.75)
                py_res = heightfield.hydraulic(
                    H.copy(), cell, iterations, rain=0.01, capacity=capacity,
                    erode_rate=0.3, deposit_rate=0.3, evaporation=0.02,
                    droplets=droplets, lifetime=16, inertia=inertia, seed=seed
                )
                rs_res = rust.hydraulic(
                    H.copy(), cell, iterations, rain=0.01, capacity=capacity,
                    erode_rate=0.3, deposit_rate=0.3, evaporation=0.02,
                    droplets=droplets, lifetime=16, inertia=inertia, seed=seed
                )
                for name, a, b in zip(["H", "flow", "eroded", "deposited"], py_res, rs_res):
                    np.testing.assert_array_equal(a, b, err_msg=f"{name} mismatch for {rows}x{cols}")

    def test_heightfield_erode_node_parity(self):
        # Test full node through Evaluator under various configurations
        subtests = [
            ("no_mask", False, 2, 3),
            ("with_mask", True, 2, 3),
            ("thermal_only", False, 0, 5),
            ("hydraulic_only", False, 3, 0),
        ]
        for name, use_mask, iters, thermal_iters in subtests:
            with self.subTest(case=name):
                nodes = {
                    "grid1": {
                        "type": "grid",
                        "params": {"rows": 32, "columns": 32, "size_x": 10.0, "size_y": 10.0}
                    },
                    "erode1": {
                        "type": "heightfield_erode",
                        "inputs": {"geo": "grid1"},
                        "params": {
                            "iterations": iters,
                            "droplets": 200,
                            "thermal_iterations": thermal_iters,
                            "seed": 99,
                        }
                    }
                }
                if use_mask:
                    # Mask node that sets mask point attribute
                    nodes["noise_mask"] = {
                        "type": "noise",
                        "inputs": {"geo": "grid1"},
                        "params": {"attrib": "mask", "mode": "scalar", "frequency": 0.5, "seed": 1}
                    }
                    nodes["erode1"]["inputs"]["mask"] = "noise_mask"

                g = Graph.from_dict({"version": 1, "nodes": nodes, "display": "erode1"})

                ev_rust = Evaluator()
                geo_rust = ev_rust.cook(g, "erode1")
                if iters > 0 or thermal_iters > 0:
                    self.assertEqual(ev_rust.last_stats.backend.get("erode1"), "rust")

                with rust.disabled():
                    ev_py = Evaluator()
                    geo_py = ev_py.cook(g, "erode1")

                for attr in ["P", "flow", "erosion", "deposit"]:
                    a = geo_rust.point[attr]
                    b = geo_py.point[attr]
                    np.testing.assert_array_equal(a, b, err_msg=f"Node output attribute {attr} mismatch in {name}")

    def test_not_a_grid_refused(self):
        # A geometry that is not a grid must raise the exact ValueError
        non_grid = Geometry(4, Topology(np.array([0, 1, 2, 3], np.int32), np.array([0, 4], np.int32)))
        non_grid.point["P"] = np.zeros((4, 3), np.float32)

        with self.assertRaises(ValueError) as cm_rust:
            heightfield.heightfield_erode(non_grid, iterations=1, droplets=10, rain=0.01, capacity=4.0,
                                          erode_rate=0.3, deposit_rate=0.3, evaporation=0.02, inertia=0.1,
                                          seed=0, thermal_iterations=0, talus_angle=35.0, thermal_rate=0.5)

        with rust.disabled():
            with self.assertRaises(ValueError) as cm_py:
                heightfield.heightfield_erode(non_grid, iterations=1, droplets=10, rain=0.01, capacity=4.0,
                                              erode_rate=0.3, deposit_rate=0.3, evaporation=0.02, inertia=0.1,
                                              seed=0, thermal_iterations=0, talus_angle=35.0, thermal_rate=0.5)

        self.assertIn("heightfield nodes need a grid", str(cm_rust.exception))
        self.assertEqual(str(cm_rust.exception), str(cm_py.exception))


if __name__ == "__main__":
    unittest.main()
