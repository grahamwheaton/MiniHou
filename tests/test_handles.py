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
    def test_transform_has_three_translate_arrows_at_the_result(self):
        src = run("box", size=[2, 2, 2])
        out = run("transform", src, translate=[3, 0, 0])
        hs = by_id(handles.handles_for("transform", [src], out, {"translate": [3, 0, 0]}))
        self.assertEqual(sorted(hs), ["translate_x", "translate_y", "translate_z"])
        np.testing.assert_allclose(hs["translate_y"]["origin"], [3, 0, 0], atol=1e-6)    # at the moved box's centre
        self.assertEqual((hs["translate_y"]["param"], hs["translate_y"]["comp"], hs["translate_y"]["dir"]), ("translate", 1, [0.0, 1.0, 0.0]))

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


class PreviewHandlesTests(unittest.TestCase):
    def test_session_preview_returns_handles_only_for_nodes_that_have_them(self):
        s = Session(graph_from({"b": {"type": "box", "params": {"size": [2, 2, 2]}},
                                "t": {"type": "transform", "inputs": {"geo": "b"}, "params": {"translate": [1, 2, 3]}},
                                "n": {"type": "noise", "inputs": {"geo": "t"}},
                                "o": {"type": "output", "inputs": {"geo": "n"}}}, display="o"))
        t = s.preview("t")
        self.assertEqual(sorted(h["id"] for h in t["handles"]), ["translate_x", "translate_y", "translate_z"])
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
            self.assertEqual(sorted(h["id"] for h in head["handles"]), ["translate_x", "translate_y", "translate_z"])
        finally:
            t.doCleanups()


if __name__ == "__main__":
    unittest.main()
