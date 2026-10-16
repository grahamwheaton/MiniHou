"""Rust USD serialization bridge tests; graph evaluation is never delegated to Python."""
import io
import json
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

from tools import rust_usd_export as bridge


def geometry(x=0):
    return {"npoints": 2, "vertex_point": [0, 1], "prim_offsets": [0, 2], "closed": [False],
            "point": {"P": {"dtype": "float32", "width": 3, "data": [x, 0, 0, 1, 2, 3]},
                      "id": {"dtype": "int32", "width": 1, "data": [4, 7]}},
            "vertex": {"uv": {"dtype": "float32", "width": 2, "data": [0, 0, 1, 1]}},
            "prim": {"group_curve": {"dtype": "int32", "width": 1, "data": [1]}},
            "detail": {"materials": ["Glass"], "tag": "Rust"}}


class RustUsdBridgeTests(unittest.TestCase):
    def test_geometry_decoding_preserves_topology_domains_and_types(self):
        g = bridge.decode_outputs({"curve": geometry()})["curve"]
        self.assertEqual((g.npoints, g.nvertices, g.nprims), (2, 2, 1))
        self.assertEqual(g.topology.closed.tolist(), [False])
        self.assertEqual(g.point["P"].shape, (2, 3))
        self.assertEqual(str(g.point["id"].dtype), "int32")
        self.assertEqual(g.vertex["uv"].shape, (2, 2))
        self.assertEqual(g.prim["group_curve"].tolist(), [1])
        self.assertEqual(g.detail, {"materials": ["Glass"], "tag": "Rust"})

    def test_current_frame_delegates_only_serialization_and_options(self):
        with tempfile.TemporaryDirectory() as folder:
            path = str(Path(folder) / "sample.usda")
            payload = {"path": path, "outputs": {"curve": geometry()}, "frame": 12,
                       "overwrite": True, "graph": {"nodes": {}},
                       "options": {"up_axis": "y", "units": "centimeters", "root": "Asset", "materials": False}}
            report = SimpleNamespace(to_dict=lambda: {"path": path, "outputs": {}})
            with patch("minihou.usd_io.pxr"), patch("minihou.usd_io.write_stage", return_value=report) as write:
                result = bridge.export(payload)
            args, kwargs = write.call_args
            self.assertEqual(str(args[0]), path)
            self.assertEqual(args[1]["curve"].npoints, 2)
            self.assertEqual(kwargs, dict(graph={"nodes": {}}, overwrite=True, up_axis="y",
                                          units="centimeters", root="Asset", materials=False, frame=12))
            self.assertEqual(result["frame"], 12)
            self.assertIn(path, result["message"])

    def test_animation_consumes_individual_streamed_samples(self):
        with tempfile.TemporaryDirectory() as folder:
            path = str(Path(folder) / "animated.usda")
            payload = {"path": path, "animated": True, "frame": 20, "fps": 60}
            seen = []

            def write(path_arg, frames, **kwargs):
                self.assertEqual(str(path_arg), path)
                self.assertEqual(kwargs["fps"], 60)
                for frame, outputs in frames:
                    seen.append((frame, float(outputs["curve"].P[0, 0])))
                return SimpleNamespace(to_dict=lambda: {"path": path, "outputs": {}, "frames": [1, 2, 2]})

            samples = ({"frame": f, "outputs": {"curve": geometry(f)}} for f in (1, 2))
            with patch("minihou.usd_io.pxr"), patch("minihou.usd_io.write_animation", side_effect=write):
                result = bridge.export(payload, samples)
            self.assertEqual(seen, [(1, 1), (2, 2)])
            self.assertEqual(result["frames"], [1, 2, 2])
            self.assertEqual(result["frame"], 20)

    def test_optional_dependency_error_is_json_http_501(self):
        payload = {"path": "unused.usda", "stream": True}
        with patch("sys.stdin", io.StringIO(json.dumps(payload) + "\n")), \
                patch("sys.stdout", new_callable=io.StringIO) as stdout, \
                patch("minihou.usd_io.pxr", side_effect=RuntimeError("USD support needs usd-core")):
            bridge.main()
        result = json.loads(stdout.getvalue())
        self.assertEqual(result["http_status"], 501)
        self.assertIn("usd-core", result["error"])

    def test_overwrite_conflict_is_json_http_409(self):
        with tempfile.TemporaryDirectory() as folder:
            payload = {"path": str(Path(folder) / "exists.usda"), "outputs": {"curve": geometry()}}
            with patch("sys.stdin", io.StringIO(json.dumps(payload))), \
                    patch("sys.stdout", new_callable=io.StringIO) as stdout, \
                    patch("minihou.usd_io.pxr"), \
                    patch("minihou.usd_io.write_stage", side_effect=FileExistsError("already exists")):
                bridge.main()
            result = json.loads(stdout.getvalue())
        self.assertEqual(result["http_status"], 409)
        self.assertIn("overwrite", result["error"])


if __name__ == "__main__":
    unittest.main()
