"""SDF kernels through the Python evaluator, including backend markers and attributes."""
import unittest
import numpy as np
from minihou import Evaluator, Graph, accel, rust
from tests.test_rust_parity import assert_same_geometry


def boolean_graph(operation="union", smooth=0, transfer=True, h=0.16):
    return {"nodes": {
        "a": {"type": "box", "params": {"divisions": 2, "add_vertex_normals": True}},
        "b": {"type": "box", "params": {"center": [0.31, 0.23, 0.17], "divisions": 2,
                                          "add_vertex_normals": True}},
        "ac": {"type": "color", "inputs": {"geo": "a"}, "params": {"mode": "random", "seed": 3}},
        "bc": {"type": "color", "inputs": {"geo": "b"}, "params": {"mode": "random", "seed": 7}},
        "out": {"type": "sdf_boolean", "inputs": {"a": "ac", "b": "bc"},
                "params": {"operation": operation, "smooth": smooth, "transfer_attribs": transfer,
                           "voxel_size": h}}}, "display": "out"}


@unittest.skipUnless(rust.available(), "Rust extension unavailable")
class SdfNodeTests(unittest.TestCase):
    def compare(self, data, marker="rust"):
        graph = Graph.from_dict(data)
        ev = Evaluator()
        with accel.disabled():
            actual = ev.cook(graph)
            with rust.disabled():
                expected = Evaluator().cook(graph)
        assert_same_geometry(self, actual, expected, atol=1e-6)
        np.testing.assert_allclose(actual.P, expected.P, atol=1e-6, rtol=0)
        self.assertEqual(ev.last_stats.backend[graph.output_node()], marker)
        return actual

    def test_boolean_operations_smoothing_and_transfer(self):
        for operation in ("union", "subtract", "intersect"):
            for smooth in (0, 0.23):
                for transfer in (False, True):
                    with self.subTest(operation=operation, smooth=smooth, transfer=transfer):
                        result = self.compare(boolean_graph(operation, smooth, transfer))
                        self.assertEqual("Cd" in result.point, transfer)

    def test_remesh(self):
        for smooth, offset in ((0, 0), (2, 0.07), (1, -0.05)):
            self.compare({"nodes": {"src": {"type": "box"},
                "out": {"type": "remesh", "inputs": {"geo": "src"},
                        "params": {"smooth": smooth, "offset": offset, "voxel_size": 0.17,
                                   "transfer": "*"}}}, "display": "out"})

    def test_points_to_surface(self):
        for smooth in (0, 2):
            self.compare({"nodes": {"src": {"type": "box", "params": {"primitive_type": "points"}},
                "colored": {"type": "color", "inputs": {"geo": "src"}, "params": {"mode": "random"}},
                "out": {"type": "points_to_surface", "inputs": {"points": "colored"},
                        "params": {"smooth": smooth, "radius": 0.4, "voxel_size": 0.13,
                                   "transfer": "*"}}}, "display": "out"})

    def test_all_analytic_shapes(self):
        from minihou.nodes.volume import SDF_SHAPES
        for shape in SDF_SHAPES:
            with self.subTest(shape=shape):
                self.compare({"nodes": {"out": {"type": "sdf_shape", "params": {
                    "shape": shape, "voxel_size": 0.14, "smooth": 1}}}, "display": "out"})

    def test_empty_fallback_markers(self):
        for kind, port in (("points_to_surface", "points"), ("sdf_boolean", "a")):
            nodes = {"src": {"type": "box", "flags": {"bypass": True}},
                     "out": {"type": kind, "inputs": {port: "src"}}}
            if kind == "sdf_boolean":
                nodes["out"]["inputs"]["b"] = "src"
            self.compare({"nodes": nodes, "display": "out"}, "fallback")

    def test_shipped_volume_graph(self):
        from pathlib import Path
        graph = Graph.load(Path(__file__).resolve().parents[1] / "examples/graphs/volumes.json")
        with accel.disabled():
            actual = Evaluator().cook(graph)
            with rust.disabled():
                expected = Evaluator().cook(graph)
        assert_same_geometry(self, actual, expected, atol=1e-6)
        np.testing.assert_array_equal(actual.P, expected.P)
        self.assertEqual(actual.topology.digest, expected.topology.digest)
        for domain in ("point", "vertex", "prim"):
            for name in expected.table(domain).names():
                self.assertEqual(actual.table(domain)[name].tobytes(), expected.table(domain)[name].tobytes())

    def test_seeded_boolean_parameters(self):
        rng = np.random.default_rng(141)
        for seed in range(8):
            data = boolean_graph(str(rng.choice(["union", "subtract", "intersect"])),
                                 float(rng.uniform(0, 0.3)), bool(seed % 2), float(rng.uniform(.13, .21)))
            data["nodes"]["b"]["params"]["center"] = rng.uniform(-.4, .4, 3).tolist()
            with self.subTest(seed=seed):
                self.compare(data)

    def test_malformed_kernel_arrays_raise(self):
        import minihou_rs as rs
        for call in (
            lambda: rs.sdf_surface_nets(np.zeros(3), [2, 2, 2], [0., 0., 0.], 1.),
            lambda: rs.sdf_blur(np.zeros(3, np.float32), [2, 2, 2], 1),
            lambda: rs.sdf_flood(np.zeros(3, bool), [2, 2, 2]),
            lambda: rs.sdf_from_mesh(np.zeros(8), [0., 0., 0.], [2, 2, 2], 1., 3.),
            lambda: rs.sdf_from_points(np.zeros(6), np.ones(1), 1., 1.),
            lambda: rs.sdf_smooth_min_f32(np.zeros(3, np.float32), np.zeros(2, np.float32), 1.),
        ):
            with self.assertRaises(ValueError):
                call()


if __name__ == "__main__":
    unittest.main()
