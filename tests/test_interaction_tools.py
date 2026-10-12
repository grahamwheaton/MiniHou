"""Numerical contracts for editable curves and transform frames."""
import json
import unittest

import numpy as np

from minihou import Evaluator, Graph
from minihou.nodes.curves import bezier, bezier_controls, BEZIER_DEFAULT
from minihou.nodes.primitives import box
from minihou.nodes.transform import transform
from minihou.handles import handles_for


class BezierTests(unittest.TestCase):
    def test_endpoints_sampling_and_closed_topology(self):
        g = bezier(BEZIER_DEFAULT, False, 16)
        g.validate()
        self.assertEqual(g.npoints, 33)
        np.testing.assert_allclose(g.P[[0, 16, -1]], [[-1, 0, 0], [0, 0, 1], [1, 0, 0]])
        self.assertFalse(g.topology.closed_mask()[0])
        closed = bezier(BEZIER_DEFAULT, True, 16)
        self.assertEqual(closed.npoints, 48)
        self.assertTrue(closed.topology.closed_mask()[0])

    def test_handle_modes_and_invalid_controls(self):
        controls = json.loads(BEZIER_DEFAULT)
        controls[1].update(mode="MIRRORED", right=[2, 0, 1])
        np.testing.assert_allclose(bezier_controls(controls)[1]["left"], [-2, 0, 1])
        controls[1].update(mode="ALIGNED", left=[0, 0, 3])
        np.testing.assert_allclose(bezier_controls(controls)[1]["left"], [-2, 0, 1])
        controls[1]["mode"] = "VECTOR"
        g = bezier(json.dumps(controls), False, 12)
        # The vector tangent approaches the central anchor from the previous anchor.
        np.testing.assert_allclose(bezier_controls(controls)[1]["left"], [-1/3, 0, 2/3])
        self.assertTrue(np.isfinite(g.P).all())
        controls[0]["p"][0] = float("nan")
        with self.assertRaises(ValueError):
            bezier(json.dumps(controls), False, 12)
        with self.assertRaises(ValueError):
            bezier("[]", False, 12)

    def test_curve_can_be_resampled_and_swept(self):
        graph = Graph.from_dict({"nodes": {
            "curve": {"type": "bezier"},
            "resample": {"type": "resample", "inputs": {"geo": "curve"}},
            "sweep": {"type": "sweep", "inputs": {"geo": "resample"}},
        }, "display": "sweep"})
        g = Evaluator().cook(graph)
        g.validate()
        self.assertGreater(g.nprims, 1)


class OrientationTests(unittest.TestCase):
    def test_translation_and_scale_in_rotated_frame(self):
        g = box((2, 2, 2), (0, 0, 0), 1)
        out = transform(g, [2, 0, 0], [0, 0, 0], [2, 1, 1], 1, [0, 0, 0], axis_rotation=[0, 0, 90])
        np.testing.assert_allclose(out.P.mean(0), [0, 2, 0], atol=1e-6)
        np.testing.assert_allclose(np.ptp(out.P, axis=0), [2, 4, 2], atol=1e-6)

    def test_oriented_rotation_normals_and_handles(self):
        g = box((2, 4, 6), (0, 0, 0), 1)
        g.point["N"] = np.tile([0., 0., 1.], (g.npoints, 1)).astype(np.float32)
        params = dict(translate=[1, 0, 0], rotate=[90, 0, 0], scale=[1, 1, 1], uniform_scale=1,
                      pivot=[0, 0, 0], axis_rotation=[0, 0, 90])
        out = transform(g, **params)
        # Frame X is world Y: a quarter turn about it sends +Z to +X.
        np.testing.assert_allclose(out.point["N"], np.tile([1, 0, 0], (g.npoints, 1)), atol=1e-6)
        handles = {h["id"]: h for h in handles_for("transform", [g], out, params)}
        np.testing.assert_allclose(handles["translate_x"]["dir"], [0, 1, 0], atol=1e-6)
        np.testing.assert_allclose(handles["rotate_x"]["dir"], [0, 1, 0], atol=1e-6)
        np.testing.assert_allclose(handles["scale_x"]["origin"], [0, 1, 0], atol=1e-6)

    def test_default_frame_preserves_existing_graph(self):
        g = box((2, 2, 2), (0, 0, 0), 1)
        args = (g, [1, 2, 3], [12, 30, 45], [2, 3, 4], 1.5, [0.1, 0.2, 0.3])
        np.testing.assert_array_equal(transform(*args).P, transform(*args, axis_rotation=[0, 0, 0]).P)
