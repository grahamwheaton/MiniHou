"""Exact float32 parity for the noise node and its erosion input."""
import json
import unittest
from pathlib import Path

import numpy as np

import minihou  # noqa: F401
from minihou import Evaluator, Graph, accel, rust
from minihou.nodes._util import fbm3, fbm3_numpy


def stage_probe(P, params):
    """Report the first divergent float32 stage of the fused Rust noise pass."""
    frequency = np.float32(params.get("frequency", 1))
    offset = np.asarray(params.get("offset", [0, 0, 0]), np.float32)
    amplitude = np.asarray(params.get("amplitude", [0, 0, 1]), np.float32)
    seed = params.get("seed", 0)
    octaves = params.get("octaves", 1)
    lacunarity = params.get("lacunarity", 2)
    gain = params.get("gain", 0.5)
    vector = params.get("mode", "scalar") == "vector"
    pos = P * frequency + offset
    seeds = [seed + 1013 * c for c in range(3)] if vector else [seed]
    raw_numpy = np.stack([fbm3_numpy(pos, s, octaves, lacunarity, gain) for s in seeds], axis=1)
    raw_rust = np.stack([rust.fbm3(pos, s, octaves, lacunarity, gain) for s in seeds], axis=1)
    if not vector:
        raw_numpy = np.broadcast_to(raw_numpy, P.shape)
        raw_rust = np.broadcast_to(raw_rust, P.shape)
    stages = [("fbm3", raw_numpy, raw_rust)]
    ones = np.ones(3, np.float32)
    fused_raw = rust.noise_vec(P, frequency, offset, ones, seed, octaves, lacunarity, gain, vector, False)
    stages.append(("sample position / fused fbm3", raw_numpy, fused_raw))
    scaled_numpy = raw_numpy * amplitude
    scaled_rust = rust.noise_vec(P, frequency, offset, amplitude, seed, octaves, lacunarity, gain, vector, False)
    stages.append(("amplitude multiplication", scaled_numpy, scaled_rust))
    added_numpy = P + scaled_numpy
    added_rust = rust.noise_vec(P, frequency, offset, amplitude, seed, octaves, lacunarity, gain, vector, True)
    stages.append(("position addition", added_numpy, added_rust))
    for name, expected, actual in stages:
        if expected.tobytes() != actual.tobytes():
            different = np.count_nonzero(expected.view(np.uint32) != actual.view(np.uint32))
            return name, float(np.max(np.abs(expected - actual))), int(different)
    return "none", 0.0, 0


@unittest.skipUnless(rust.available(), "the Rust core is not built")
class RustNoiseExactTests(unittest.TestCase):
    def test_terrain_stage_probe(self):
        data = json.loads((Path(__file__).resolve().parents[1] / "examples/graphs/eroded_terrain.json").read_text())
        graph = Graph.from_dict(data)
        with rust.disabled(), accel.disabled():
            source = Evaluator().cook(graph, "land")
            expected = Evaluator().cook(graph, "hills")
        params = data["nodes"]["hills"]["params"]
        stage = stage_probe(source.P, params)
        actual = Evaluator().cook(graph, "hills")
        with rust.disabled():
            numba_result = Evaluator().cook(graph, "hills")
        print(f"terrain first divergent stage: {stage}; P max diff: "
              f"{float(np.max(np.abs(expected.P - actual.P)))}")
        print(f"terrain Rust vs Rust-disabled ({accel.status()}): "
              f"{float(np.max(np.abs(numba_result.P - actual.P)))}; "
              f"NumPy vs Rust-disabled: {float(np.max(np.abs(expected.P - numba_result.P)))}")
        pos = source.P * np.float32(params["frequency"])
        with rust.disabled():
            numba_raw = fbm3(pos, params["seed"], params["octaves"], 2, 0.5)
        numpy_raw = fbm3_numpy(pos, params["seed"], params["octaves"], 2, 0.5)
        if numba_raw.tobytes() != numpy_raw.tobytes():
            print(f"Numba first divergent stage: fbm3; max diff: "
                  f"{float(np.max(np.abs(numba_raw - numpy_raw)))}")
        self.assertEqual(stage[0], "none")
        np.testing.assert_array_equal(actual.P, expected.P)

    def test_noise_node_matches_numpy_on_all_attributes(self):
        rng = np.random.default_rng(781)
        cases = [
            {"seed": 4, "octaves": 6, "frequency": 0.11, "amplitude": [0, 0, 3.5]},
            {"seed": -17, "octaves": 1, "frequency": 3.3, "offset": [0.25, -1.2, 2.1],
             "amplitude": [-0.4, 0.7, 1.3], "mode": "vector"},
        ]
        for _ in range(4):
            cases.append({
                "seed": int(rng.integers(-10000, 10000)),
                "octaves": int(rng.integers(1, 9)),
                "frequency": float(rng.uniform(0.03, 4)),
                "offset": rng.uniform(-3, 3, 3).tolist(),
                "amplitude": rng.uniform(-2, 2, 3).tolist(),
                "lacunarity": float(rng.uniform(1, 3.5)),
                "gain": float(rng.uniform(0.1, 0.95)),
                "mode": "vector" if rng.integers(2) else "scalar",
            })
        # The grid node has a minimum of two rows and columns. Probe its one-point
        # equivalent directly at the kernel boundary.
        one_point = np.array([[0.17, -0.39, 0.0]], np.float32)
        for case in cases:
            self.assertEqual(stage_probe(one_point, case)[0], "none", case)
        for rows, columns in [(2, 3), (64, 64), (300, 300)]:
            for case in cases:
                for attrib in ("P", "height", "velocity"):
                    params = {**case, "attrib": attrib}
                    data = {"version": 1, "nodes": {
                        "grid": {"type": "grid", "params": {"rows": rows, "columns": columns,
                                                               "size_x": 20, "size_y": 13}},
                        "noise": {"type": "noise", "inputs": {"geo": "grid"}, "params": params},
                    }, "display": "noise"}
                    graph = Graph.from_dict(data)
                    with self.subTest(shape=(rows, columns), params=params):
                        with accel.disabled():
                            actual = Evaluator().cook(graph, "noise")
                            with rust.disabled():
                                expected = Evaluator().cook(graph, "noise")
                        self.assertEqual(set(actual.point.names()), set(expected.point.names()))
                        for name in expected.point.names():
                            np.testing.assert_array_equal(actual.point[name], expected.point[name], err_msg=name)
                        core = rust.cook(data, "noise")
                        for name in expected.point.names():
                            np.testing.assert_array_equal(core.point[name], expected.point[name],
                                                          err_msg=f"core {name}")

    def test_noise_then_erosion_matches_numpy(self):
        data = {"version": 1, "nodes": {
            "grid": {"type": "grid", "params": {"rows": 40, "columns": 40, "size_x": 10, "size_y": 10}},
            "noise": {"type": "noise", "inputs": {"geo": "grid"},
                      "params": {"amplitude": [0, 0, 3.5], "frequency": 0.11, "octaves": 6, "seed": 4}},
            "erode": {"type": "heightfield_erode", "inputs": {"geo": "noise"},
                      "params": {"iterations": 2, "droplets": 300, "thermal_iterations": 2, "seed": 17}},
        }, "display": "erode"}
        graph = Graph.from_dict(data)
        with accel.disabled():
            actual = Evaluator().cook(graph, "erode")
            with rust.disabled():
                expected = Evaluator().cook(graph, "erode")
        for name in ("P", "flow", "erosion", "deposit"):
            np.testing.assert_array_equal(actual.point[name], expected.point[name], err_msg=name)


if __name__ == "__main__":
    unittest.main()
