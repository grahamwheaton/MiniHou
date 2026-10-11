"""Blender objects as graph inputs (blender_object node), engine side: a link client plays the add-on."""
import time
import unittest

import numpy as np

from minihou import Evaluator, Graph
from minihou.inputs import INPUTS
from minihou.link.protocol import TOPO_PRIM_OFFSETS, TOPO_VERTEX_POINT
from tests.test_link import Harness, wait_for

QUAD_P = np.array([[0, 0, 0], [1, 0, 0], [1, 1, 0], [0, 1, 0]], np.float32)
MOVE = [1, 0, 0, 10,  0, 1, 0, 0,  0, 0, 1, 0,  0, 0, 0, 1]          # translate +10 in X (row-major)


def graph(obj="Cube", space="world", snippet="@P.z = @w"):
    return Graph.from_dict({"nodes": {
        "src": {"type": "blender_object", "params": {"object": obj, "space": space}},
        "w": {"type": "wrangle", "inputs": {"geo": "src"}, "params": {"snippet": snippet}},
        "o": {"type": "output", "inputs": {"geo": "w"}}}, "display": "o"}, check=False)


def mesh_arrays(z=0.0, weights=(0, 0.25, 0.5, 1)):
    return {"point/P": QUAD_P + np.float32(z), TOPO_VERTEX_POINT: np.array([0, 1, 2, 3], np.int32),
            TOPO_PRIM_OFFSETS: np.array([0, 4], np.int32),
            "point/w": np.array(weights, np.float32),
            "vertex/uv": QUAD_P[:, :2].copy(), "prim/material_index": np.array([2], np.int32),
            "point/N": np.tile(np.array([0, 0, 1], np.float32), (4, 1))}


class ObjectInputTests(unittest.TestCase):
    def setUp(self):
        INPUTS.clear()
        self.addCleanup(INPUTS.clear)

    def start(self, g):
        h = Harness(g)
        self.addCleanup(h.close)
        c, q = h.client()
        return h, c, q

    def send(self, c, name="Cube", matrix=MOVE, **kw):
        self.assertTrue(c.send({"type": "object_geo", "object": name, "matrix": matrix}, mesh_arrays(**kw)))

    def wait_cooked(self, h, pred, timeout=5.0):
        end = time.monotonic() + timeout
        while time.monotonic() < end:
            h.session.wait_idle(1.0)
            st = h.session.status()
            if pred(st):
                return st
            time.sleep(0.02)
        self.fail(f"not reached; status {h.session.status()}")

    def test_engine_asks_for_objects_and_cooks_what_arrives(self):
        h, c, q = self.start(graph())
        hdr, _ = wait_for(q, "watch_objects")
        self.assertEqual(hdr["objects"], ["Cube"])
        st = self.wait_cooked(h, lambda s: s.get("ok") is False)            # nothing received yet
        self.assertIn("waiting for Blender object 'Cube'", st["error"]["message"])
        self.send(c)
        self.wait_cooked(h, lambda s: s.get("ok"))
        geo = Evaluator().cook(h.session.graph)
        np.testing.assert_allclose(geo.P[:, 0], QUAD_P[:, 0] + 10)            # world space: the object's transform
        np.testing.assert_allclose(geo.P[:, 2], [0, 0.25, 0.5, 1])            # @P.z = @w (a vertex group)
        self.assertEqual(geo.vertex["uv"].shape, (4, 2))
        self.assertEqual(geo.prim["material_index"].tolist(), [2])
        c.send({"type": "object_geo", "object": "Cube", "matrix": MOVE, "info": {"vertex_groups": ["w"]}}, mesh_arrays())
        self.wait_cooked(h, lambda s: s.get("ok"))
        end = time.monotonic() + 5
        while h.session.inspect("src").get("blender", {}).get("vertex_groups") != ["w"] and time.monotonic() < end:
            time.sleep(0.02)
        self.assertEqual(h.session.inspect("src")["blender"]["vertex_groups"], ["w"])    # what Blender sent

    def test_new_data_recooks_and_watch_follows_the_graph(self):
        h, c, q = self.start(graph())
        wait_for(q, "watch_objects")
        self.send(c)
        self.wait_cooked(h, lambda s: s.get("ok"))
        rev = h.session.rev
        self.send(c, weights=(1, 1, 1, 1))                                     # the object changed in Blender
        self.wait_cooked(h, lambda s: s.get("ok") and s["cooked_rev"] > rev)
        np.testing.assert_allclose(h.session.inspect("w")["bounds"]["max"][2], 1.0)
        h.session.edit([{"op": "set", "id": "src", "params": {"object": "Suzanne"}}])
        hdr, _ = wait_for(q, "watch_objects", pred=lambda hd: hd["objects"] == ["Suzanne"])
        self.assertEqual(hdr["objects"], ["Suzanne"])

    def test_missing_objects_and_bad_data_are_reported(self):
        h, c, q = self.start(graph())
        wait_for(q, "watch_objects")
        c.send({"type": "object_missing", "object": "Cube", "reason": "no object named 'Cube' in the scene"})
        st = self.wait_cooked(h, lambda s: s.get("ok") is False and "scene" in s["error"]["message"])
        self.assertEqual(st["error"]["node"], "src")
        bad = mesh_arrays()
        bad[TOPO_VERTEX_POINT] = np.array([0, 1, 2, 99], np.int32)
        c.send({"type": "object_geo", "object": "Cube", "matrix": MOVE}, bad)
        hdr, _ = wait_for(q, "error", pred=lambda hd: hd.get("where") == "object_geo")
        self.assertIn("Cube", hdr["message"])

    def test_object_space_normals_and_own_objects(self):
        from minihou.link.server import _decode_object
        rot = [0, -1, 0, 0,  1, 0, 0, 0,  0, 0, 1, 0,  0, 0, 0, 1]            # 90 degrees about Z
        arrays = mesh_arrays()
        arrays["point/N"] = np.tile(np.array([1, 0, 0], np.float32), (4, 1))
        INPUTS.put("Cube", _decode_object({}, arrays), rot)
        world = Evaluator().cook(graph(snippet="f@k = 1"))
        np.testing.assert_allclose(world.point["N"][0], [0, 1, 0], atol=1e-6)    # normals follow the rotation
        local = Evaluator().cook(graph(space="object", snippet="f@k = 1"))
        np.testing.assert_allclose(local.P, QUAD_P, atol=1e-6)
        from minihou import NodeError
        with self.assertRaises(NodeError) as cm:
            Evaluator().cook(graph(obj="MiniHou_Output"))
        self.assertIn("feed the result back", str(cm.exception))

    def test_cache_key_follows_the_data(self):
        from minihou.link.server import _decode_object
        ev, g = Evaluator(), graph()
        INPUTS.put("Cube", _decode_object({}, mesh_arrays()), MOVE)
        ev.cook(g)
        ev.cook(g)
        self.assertEqual(ev.last_stats.cooked, [])                            # same data: nothing recooks
        INPUTS.put("Cube", _decode_object({}, mesh_arrays(weights=(2, 2, 2, 2))), MOVE)
        geo = ev.cook(g)
        self.assertIn("src", ev.last_stats.cooked)                            # new data: recooked
        np.testing.assert_allclose(geo.P[:, 2], 2)


if __name__ == "__main__":
    unittest.main()
