"""Viewport handles: what the engine says about draggable arrows for a previewed node."""
import json
import unittest

import numpy as np

from minihou import Evaluator, handles
from minihou.link.session import Session
from tests.helpers import graph_from
from tests.test_nodes import run


def by_id(hs):
    return {h["id"]: h for h in hs}


class ProviderTests(unittest.TestCase):
    def test_transform_has_translate_arrows_at_the_result(self):
        src = run("box", size=[2, 2, 2])
        out = run("transform", src, translate=[3, 0, 0])
        hs = by_id(handles.handles_for("transform", [src], out, {"translate": [3, 0, 0]}))
        self.assertEqual(sorted(k for k in hs if k.startswith("translate")), ["translate_x", "translate_y", "translate_z"])
        np.testing.assert_allclose(hs["translate_y"]["origin"], [3, 0, 0], atol=1e-6)    # at the moved box's centre
        self.assertEqual((hs["translate_y"]["param"], hs["translate_y"]["comp"], hs["translate_y"]["dir"]), ("translate", 1, [0.0, 1.0, 0.0]))

    def test_transform_rings_turn_about_the_axes_each_euler_angle_really_uses(self):
        src = run("box", size=[2, 2, 2])
        p = {"translate": [1, 0, 0], "rotate": [10, 20, 90], "pivot": [0, 2, 0]}
        hs = by_id(handles.handles_for("transform", [src], run("transform", src, **p), p))
        for i in range(3):
            self.assertEqual((hs[f"rotate_{'xyz'[i]}"]["kind"], hs[f"rotate_{'xyz'[i]}"]["tool"], hs[f"rotate_{'xyz'[i]}"]["comp"]), ("ring", "rotate", i))
        np.testing.assert_allclose(hs["rotate_x"]["origin"], [1, 2, 0], atol=1e-9)           # pivot + translate: it stays put while dragged
        self.assertAlmostEqual(hs["rotate_z"]["scale"], 180 / np.pi)                          # radians dragged -> degrees
        from minihou.nodes._util import euler_xyz_matrix
        # turning each Euler angle by a little must move points about exactly the ring's axis (right-handed)
        P = np.array([[1.0, 2.0, 3.0]])
        for i in range(3):
            r2 = list(p["rotate"]); r2[i] += 0.01
            d = (euler_xyz_matrix(r2) @ P.T - euler_xyz_matrix(p["rotate"]) @ P.T).ravel()
            axis = np.asarray(hs[f"rotate_{'xyz'[i]}"]["dir"])
            self.assertLess(abs(d @ axis), 1e-6, i)                                           # no movement along the axis
            np.testing.assert_allclose(d / np.linalg.norm(d), np.cross(axis, euler_xyz_matrix(p["rotate"]) @ P[0]) / np.linalg.norm(np.cross(axis, euler_xyz_matrix(p["rotate"]) @ P[0])), atol=2e-2)
        self.assertGreater(hs["rotate_x"]["radius"], 0.5)

    def test_transform_scale_handles_follow_the_rotated_axes_and_uniform_scale_is_a_square(self):
        src = run("box", size=[2, 2, 2])
        p = {"rotate": [0, 0, 90]}
        hs = by_id(handles.handles_for("transform", [src], run("transform", src, **p), p))
        np.testing.assert_allclose(hs["scale_x"]["dir"], [0, 1, 0], atol=1e-9)               # scaling happens before the rotation
        self.assertEqual((hs["scale_x"]["kind"], hs["scale_x"]["param"], hs["scale_x"]["comp"], hs["scale_x"]["tool"]), ("scale", "scale", 0, "scale"))
        self.assertEqual((hs["uniform_scale"]["kind"], hs["uniform_scale"]["param"], hs["uniform_scale"]["comp"]), ("uniform", "uniform_scale", None))

    def test_box_and_sphere_arrows_sit_on_the_surface_and_scale_correctly(self):
        hs = by_id(handles.handles_for("box", [], run("box"), {"size": [2, 4, 6], "center": [1, 0, 0]}))
        np.testing.assert_allclose(hs["size_x"]["origin"], [2, 0, 0])
        np.testing.assert_allclose(hs["size_z"]["origin"], [1, 0, 3])
        self.assertEqual(hs["size_x"]["scale"], 2.0)                                     # size is the full width: twice the drag
        s = handles.handles_for("sphere", [], run("sphere"), {"radius": 2.5, "center": [0, 1, 0]})
        np.testing.assert_allclose(s[0]["origin"], [2.5, 1, 0])
        self.assertEqual((s[0]["param"], s[0]["min"]), ("radius", 0.0))

    def test_polyextrude_arrow_stands_on_the_selected_faces_along_their_normal(self):
        src = run("box", size=[2, 2, 2])
        vp, offs = src.topology.vertex_point.astype(int), src.topology.prim_offsets
        top = max(range(src.nprims), key=lambda q: src.P[vp[offs[q]:offs[q + 1]]].mean(0)[2])
        h = handles.handles_for("polyextrude", [src], run("polyextrude", src, group=str(top)), {"group": str(top)})[0]
        np.testing.assert_allclose(h["origin"], [0, 0, 1], atol=1e-6)
        np.testing.assert_allclose(h["dir"], [0, 0, 1], atol=1e-6)
        self.assertEqual(h["param"], "distance")
        self.assertEqual(len(handles.handles_for("polyextrude", [src], src, {"group": "nothing_here"})), 0)    # a bad group: no arrow

    def test_polybevel_arrow_runs_from_the_edge_into_a_face(self):
        src = run("box", size=[2, 2, 2])
        vp, offs = src.topology.vertex_point.astype(int), src.topology.prim_offsets
        top = max(range(src.nprims), key=lambda q: src.P[vp[offs[q]:offs[q + 1]]].mean(0)[2])
        v = vp[offs[top]:offs[top + 1]]
        edge = f"{min(v[0], v[1])}-{max(v[0], v[1])}"
        h = handles.handles_for("polybevel", [src], src, {"edges": edge})[0]
        mid = src.P[[v[0], v[1]]].mean(0)
        np.testing.assert_allclose(h["origin"], mid, atol=1e-6)
        face_centre = src.P[v].mean(0)
        inside = (face_centre - mid) / np.linalg.norm(face_centre - mid)
        self.assertGreater(float(np.dot(h["dir"], inside)), 0.99)                         # towards the face the edge belongs to
        self.assertEqual(h["min"], 0.0)
        self.assertEqual(handles.handles_for("polybevel", [src], src, {"edges": ""}), [])

    def test_unknown_types_and_failures_give_nothing(self):
        self.assertEqual(handles.handles_for("noise", [], run("grid"), {}), [])
        self.assertEqual(handles.handles_for("transform", [], None, {}), [])
        self.assertEqual(handles.handles_for("polybevel", [run("box")], run("box"), {"edges": "garbage"}), [])


class DeclaredHandleTests(unittest.TestCase):
    """``@node(handles=[Handle(...)])``: gizmos for any node type, library nodes included."""

    def reg(self):
        from minihou.sdk import Float, Handle, Registry, Vec3, node
        r = Registry()

        @node("probe", params=[Float("radius", 2.0, min=0.0), Vec3("origin", (1.0, 2.0, 3.0)), Vec3("dir", (0.0, 0.0, 2.0)),
                               Float("angle", 30.0), Float("size", 4.0)],
              registry=r,
              handles=[Handle("radius", axis="x", at="center", reach=True, min=0.0),
                       Handle("angle", kind="ring", axis="@dir", at="@origin"),
                       Handle("size", axis="-y", at="top", scale=2.0, reach=True),
                       Handle("origin", comp=2, axis=(0, 0, 1), at="origin")])
        def probe(radius, origin, dir, angle, size):
            from minihou import Geometry
            return Geometry.points_only(np.array([[0, 0, 0], [2, 2, 2]], np.float32))
        return r

    def test_specs_resolve_against_the_parameters_and_the_output(self):
        r = self.reg()
        nt = r.get("probe")
        out = nt.fn(**nt.default_params())
        hs = by_id(handles.handles_for("probe", [], out, nt.default_params(), nt))
        self.assertEqual(sorted(hs), ["angle", "origin_z", "radius", "size"])
        np.testing.assert_allclose(hs["radius"]["origin"], [1 + 2, 1, 1])                 # centre of the bounds, then along x by the radius
        self.assertEqual(hs["radius"]["min"], 0.0)
        np.testing.assert_allclose(hs["angle"]["origin"], [1, 2, 3])                      # "@origin": a Vec3 parameter
        np.testing.assert_allclose(hs["angle"]["dir"], [0, 0, 1])                         # "@dir" is normalised
        self.assertEqual(hs["angle"]["kind"], "ring")
        self.assertAlmostEqual(hs["angle"]["scale"], 180 / np.pi)                         # a ring edits degrees
        self.assertGreater(hs["angle"]["radius"], 0.5)
        np.testing.assert_allclose(hs["size"]["origin"], [1, 1 - 4 / 2.0, 2])             # top of the bounds, plus value / scale along -y
        np.testing.assert_allclose(hs["size"]["dir"], [0, -1, 0])
        self.assertEqual((hs["origin_z"]["param"], hs["origin_z"]["comp"]), ("origin", 2))
        np.testing.assert_allclose(hs["origin_z"]["origin"], [0, 0, 0])                   # "origin" is the world origin

    def test_bad_specs_are_refused_when_the_node_is_defined(self):
        from minihou.sdk import Bool, Float, Handle, Registry, Vec3, node
        base = dict(params=[Float("a", 1.0), Vec3("v", (0, 0, 1)), Bool("flag", False)], registry=Registry())
        for bad, why in ((Handle("missing"), "no parameter"), (Handle("flag"), "bool"), (Handle("v"), "which component"),
                         (Handle("a", comp=0), "comp only"), (Handle("a", kind="sphere"), "kind"), (Handle("a", axis="w"), "axis"),
                         (Handle("a", axis="@a"), "Vec3"), (Handle("a", at="middle"), "at"), (Handle("a", at=(1, 2)), "three numbers")):
            with self.assertRaisesRegex(ValueError, why, msg=str(bad)):
                node("n", handles=[bad], **dict(base, registry=Registry()))(lambda a, v, flag: None)
        with self.assertRaisesRegex(ValueError, "unique"):
            node("n", handles=[Handle("a"), Handle("a")], **dict(base, registry=Registry()))(lambda a, v, flag: None)

    def test_a_failing_spec_shows_nothing_and_never_breaks_the_preview(self):
        r = self.reg()
        nt = r.get("probe")
        params = nt.default_params()
        params["dir"] = (0, 0, 0)                                                         # a zero axis: that handle is skipped
        hs = by_id(handles.handles_for("probe", [], nt.fn(**nt.default_params()), params, nt))
        self.assertNotIn("angle", hs)
        self.assertIn("radius", hs)
        self.assertEqual(handles.handles_for("probe", [], None, params, nt), [])

    def test_built_in_primitives_declare_their_handles_and_the_session_serves_them(self):
        s = Session(graph_from({"t": {"type": "tube", "params": {"radius_top": 0.5, "height": 3.0}},
                                "o": {"type": "output", "inputs": {"geo": "t"}}}, display="o"))
        hs = by_id(s.preview("t")["handles"])
        self.assertEqual(sorted(hs), ["height", "radius_bottom", "radius_top"])
        np.testing.assert_allclose(hs["radius_top"]["origin"], [0.5, 0, 1.5], atol=1e-5)  # on the top rim
        np.testing.assert_allclose(hs["height"]["origin"], [0, 0, 1.5], atol=1e-5)
        self.assertEqual(hs["height"]["scale"], 2.0)                                      # it grows both ways from the centre


class PreviewHandlesTests(unittest.TestCase):
    def test_session_preview_returns_handles_only_for_nodes_that_have_them(self):
        s = Session(graph_from({"b": {"type": "box", "params": {"size": [2, 2, 2]}},
                                "t": {"type": "transform", "inputs": {"geo": "b"}, "params": {"translate": [1, 2, 3]}},
                                "n": {"type": "noise", "inputs": {"geo": "t"}},
                                "o": {"type": "output", "inputs": {"geo": "n"}}}, display="o"))
        t = s.preview("t")
        self.assertEqual(sorted(h["id"] for h in t["handles"]), ["rotate_x", "rotate_y", "rotate_z", "scale_x", "scale_y", "scale_z", "translate_x", "translate_y", "translate_z", "uniform_scale"])
        np.testing.assert_allclose(t["handles"][0]["origin"], [1, 2, 3], atol=1e-5)
        self.assertEqual(s.preview("n")["handles"], [])
        self.assertEqual(len(s.preview("b")["handles"]), 3)

    def test_the_http_preview_carries_them_in_its_header(self):
        from tests.test_web import WebTests
        from minihou import preview
        t = WebTests("test_state_has_everything_an_editor_needs")
        t.setUp()
        try:
            import http.client
            c = http.client.HTTPConnection("127.0.0.1", t.port, timeout=10)
            c.request("POST", "/api/preview", json.dumps({"node": "spike_up"}), {"Content-Type": "application/json"})
            data = c.getresponse().read()
            c.close()
            head, _ = preview.unpack(data)
            self.assertEqual(sorted(h["id"] for h in head["handles"]), ["rotate_x", "rotate_y", "rotate_z", "scale_x", "scale_y", "scale_z", "translate_x", "translate_y", "translate_z", "uniform_scale"])
        finally:
            t.doCleanups()


if __name__ == "__main__":
    unittest.main()
