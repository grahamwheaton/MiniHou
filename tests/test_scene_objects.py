"""Scene objects: the manifest the engine builds (nodes/scene_objects.py), how the link sends it (once per change, per client) and the
modifier path's response. The Blender side is tested in tests/test_blender_scene_objects.py and tests/test_scene_plan.py."""
import queue
import time
import unittest

import numpy as np

import minihou  # noqa: F401
from minihou import Evaluator, Geometry, Graph
from minihou.link.client import LinkClient
from minihou.link.server import LinkServer
from minihou.link.session import Session
from minihou.nodes.scene_objects import build_manifest, plan
from minihou.sdk import REGISTRY
from tests.test_link import wait_for

FOREST = {
    "g": {"type": "grid", "params": {"columns": 5, "rows": 5, "size_x": 10, "size_y": 10}},
    "p": {"type": "scatter", "inputs": {"geo": "g"}, "params": {"count": 12, "seed": 3}},
    "i": {"type": "instance", "inputs": {"points": "p"}, "params": {"objects": "Tree Bush", "pick": "cycle", "seed": 2}},
    "so": {"type": "scene_objects", "inputs": {"geo": "i"}, "params": {"prefix": "Forest"}},
    "out": {"type": "output", "inputs": {"geo": "g"}},
}


def params(**kw):
    nt = REGISTRY.get("scene_objects")
    return nt.coerce_params(kw)


class ManifestTests(unittest.TestCase):
    def cook(self, graph=FOREST, target="i"):
        return Evaluator().cook(Graph.from_dict({"nodes": graph, "display": "out"}), target)

    def test_instance_points_become_one_object_each_with_their_own_object_and_transform(self):
        geo = self.cook()
        m, arrays = build_manifest("so", params(prefix="Forest"), geo, "key1")
        self.assertEqual(arrays, {})
        self.assertEqual((m["type"], m["node"], m["key"], m["mode"], m["prefix"], m["collection"]),
                         ("scene_objects", "so", "key1", "instances", "Forest", "Forest"))
        self.assertEqual(m["sources"], ["Tree", "Bush"])
        self.assertEqual(len(m["id"]), 12)
        self.assertEqual(m["id"], list(range(12)))                                       # no id attribute: point numbers
        self.assertEqual(m["src"], [i % 2 for i in range(12)])                           # cycle: Tree, Bush, Tree...
        np.testing.assert_allclose(m["loc"], geo.P, atol=1e-5)
        np.testing.assert_allclose(m["rot"], geo.point["rot"], atol=1e-6)
        np.testing.assert_allclose(m["scale"], geo.point["scale"], atol=1e-5)

    def test_the_node_passes_geometry_through_and_is_never_the_display(self):
        g = Graph.from_dict({"nodes": FOREST, "display": "out"})
        ev = Evaluator()
        self.assertIs(ev.cook(g, "so"), ev.cook(g, "i"))
        self.assertEqual(g.output_node(), "out")
        self.assertEqual(g.scene_sinks(), {"so": "so"})

    def test_points_without_an_instance_attribute_use_the_source_parameter(self):
        geo = Geometry.from_arrays(np.array([[0, 0, 0], [1, 2, 3]], np.float32))
        m = plan(geo, "instances", "Rock", "id", 100)[0]
        self.assertEqual((m["sources"], m["src"], m["id"]), (["Rock"], [0, 0], [0, 1]))
        np.testing.assert_allclose(m["scale"], [[1, 1, 1], [1, 1, 1]])
        geo = geo.copy()
        geo.point["pscale"] = np.array([2.0, 0.5], np.float32)
        np.testing.assert_allclose(plan(geo, "instances", "Rock", "id", 100)[0]["scale"], [[2, 2, 2], [0.5, 0.5, 0.5]])
        with self.assertRaises(ValueError) as cm:
            plan(Geometry.from_arrays(np.zeros((2, 3), np.float32)), "instances", "", "id", 100)
        self.assertIn("source", str(cm.exception))

    def test_points_with_instance_zero_are_skipped(self):
        geo = Geometry.from_arrays(np.arange(12, dtype=np.float32).reshape(4, 3))
        geo.point["instance"] = np.array([1, 0, 2, 0], np.int32)
        geo.detail["instance_objects"] = ["A", "B"]
        m = plan(geo, "instances", "", "id", 100)[0]
        self.assertEqual((m["id"], m["src"], m["sources"]), ([0, 1], [0, 1], ["A", "B"]))
        self.assertEqual(m["loc"], [[0.0, 1.0, 2.0], [6.0, 7.0, 8.0]])

    def test_an_id_attribute_gives_stable_identity_and_must_be_unique_integers(self):
        geo = Geometry.from_arrays(np.zeros((3, 3), np.float32))
        geo.point["id"] = np.array([10, 7, 99], np.int32)
        self.assertEqual(plan(geo, "instances", "R", "id", 10)[0]["id"], [10, 7, 99])
        self.assertEqual(plan(geo, "instances", "R", "", 10)[0]["id"], [0, 1, 2])         # cleared: point numbers
        geo2 = geo.copy()
        geo2.point["id"] = np.array([1, 1, 2], np.int32)
        with self.assertRaises(ValueError) as cm:
            plan(geo2, "instances", "R", "id", 10)
        self.assertIn("repeated", str(cm.exception))
        geo3 = geo.copy()
        geo3.point["id"] = np.array([1.0, 2.0, 3.0], np.float32)
        with self.assertRaises(ValueError):
            plan(geo3, "instances", "R", "id", 10)

    def test_too_many_objects_are_refused_not_truncated(self):
        geo = Geometry.from_arrays(np.zeros((50, 3), np.float32))
        with self.assertRaises(ValueError) as cm:
            plan(geo, "instances", "R", "id", 20)
        self.assertIn("max_objects", str(cm.exception))
        self.assertEqual(len(plan(geo, "instances", "R", "id", 50)[0]["id"]), 50)

    def test_empty_prefix_and_collection_fall_back(self):
        geo = Geometry.from_arrays(np.zeros((1, 3), np.float32))
        m, _ = build_manifest("n", params(prefix="  ", source="R"), geo, "k")
        self.assertEqual((m["prefix"], m["collection"]), ("MiniHou", "MiniHou"))
        m, _ = build_manifest("n", params(prefix="Trees", collection="Props", source="R"), geo, "k")
        self.assertEqual((m["prefix"], m["collection"]), ("Trees", "Props"))

    def test_the_manifest_is_plain_json(self):
        import json
        json.dumps(build_manifest("so", params(prefix="Forest"), self.cook(), "k")[0], allow_nan=False)


class PiecesManifestTests(unittest.TestCase):
    def fractured(self):
        graph = {"b": {"type": "box", "params": {"size": [2, 2, 2]}},
                 "pts": {"type": "voronoi_fracture_points", "inputs": {"geo": "b"}, "params": {"count": 6, "seed": 4}},
                 "f": {"type": "voronoi_fracture", "inputs": {"geo": "b", "points": "pts"}, "params": {}},
                 "a": {"type": "assemble", "inputs": {"geo": "f"}, "params": {}}}
        return Evaluator().cook(Graph.from_dict({"nodes": graph, "display": "a"}))

    def rebuild(self, cols, arrays, k):
        """Piece k as (points in world space, faces as lists of local point numbers)."""
        pb = np.cumsum([0] + cols["npts"])
        cb = np.cumsum([0] + cols["ncorner"])
        fb = np.cumsum([0] + cols["nprim"])
        P = arrays["P"][pb[k]:pb[k + 1]] + np.array(cols["loc"][k], np.float32)
        vp = arrays["vp"][cb[k]:cb[k + 1]]
        sizes = arrays["sizes"][fb[k]:fb[k + 1]]
        faces = np.split(vp, np.cumsum(sizes)[:-1])
        return P, faces

    def test_every_piece_is_a_mesh_with_its_origin_at_its_centre(self):
        geo = self.fractured()
        cols, arrays = plan(geo, "pieces", "", "id", 100, "piece")
        n = len(cols["id"])
        self.assertGreaterEqual(n, 4)
        self.assertEqual(cols["id"], sorted(set(geo.prim["piece"].tolist())))               # the assemble node's piece numbers
        self.assertEqual(sum(cols["nprim"]), geo.nprims)
        self.assertEqual((len(arrays["P"]), len(arrays["vp"]), len(arrays["sizes"])), (sum(cols["npts"]), sum(cols["ncorner"]), geo.nprims))
        self.assertEqual(arrays["P"].dtype, np.float32)
        self.assertEqual(arrays["vp"].dtype, np.int32)
        for k, ident in enumerate(cols["id"]):
            P, faces = self.rebuild(cols, arrays, k)
            self.assertEqual(len(faces), cols["nprim"][k])
            self.assertTrue(all(f.max() < len(P) for f in faces))
            local = arrays["P"][np.cumsum([0] + cols["npts"])[k]:np.cumsum([0] + cols["npts"])[k + 1]]
            np.testing.assert_allclose(local.mean(0), 0, atol=1e-5)                       # centred on its own origin
            # the same faces as the source piece, in world space
            prims = np.flatnonzero(geo.prim["piece"] == ident)
            want = sorted(tuple(sorted(np.round(geo.P[geo.topology.vertex_point[geo.topology.prim_offsets[p]:geo.topology.prim_offsets[p + 1]]], 4).ravel().tolist()))
                          for p in prims)
            got = sorted(tuple(sorted(np.round(P[f], 4).ravel().tolist())) for f in faces)
            for a, b in zip(want, got):
                np.testing.assert_allclose(a, b, atol=2e-4)

    def test_without_a_piece_attribute_connected_pieces_are_used(self):
        geo = self.fractured().copy()
        del geo.prim["piece"]
        cols, _ = plan(geo, "pieces", "", "id", 100, "piece")
        self.assertEqual(cols["id"], list(range(len(cols["id"]))))
        self.assertGreaterEqual(len(cols["id"]), 4)

    def test_shared_points_are_in_both_pieces_and_limits_and_errors(self):
        # two quads sharing an edge, as separate pieces 5 and 9
        P = np.array([[0, 0, 0], [1, 0, 0], [2, 0, 0], [0, 1, 0], [1, 1, 0], [2, 1, 0]], np.float32)
        g = Geometry.from_arrays(P, [0, 1, 4, 3, 1, 2, 5, 4], [0, 4, 8])
        g.prim["piece"] = np.array([9, 5], np.int32)
        cols, arrays = plan(g, "pieces", "", "id", 10, "piece")
        self.assertEqual((cols["id"], cols["npts"], cols["nprim"]), ([5, 9], [4, 4], [1, 1]))
        np.testing.assert_allclose(cols["loc"], [[1.5, 0.5, 0], [0.5, 0.5, 0]], atol=1e-6)
        with self.assertRaises(ValueError) as cm:
            plan(g, "pieces", "", "id", 1, "piece")
        self.assertIn("max_objects", str(cm.exception))
        g2 = g.copy()
        g2.prim["piece"] = np.array([1.0, 2.0], np.float32)
        with self.assertRaises(ValueError):
            plan(g2, "pieces", "", "id", 10, "piece")
        with self.assertRaises(ValueError) as cm:
            plan(Geometry.from_arrays(np.zeros((3, 3), np.float32)), "pieces", "", "id", 10, "piece")
        self.assertIn("faces", str(cm.exception))

    def test_wanted_attributes_travel_with_their_pieces(self):
        geo = self.fractured().copy()
        geo.point["Cd"] = np.random.default_rng(1).random((geo.npoints, 3)).astype(np.float32)
        geo.vertex["uv"] = np.random.default_rng(2).random((geo.nvertices, 2)).astype(np.float32)
        geo.prim["material_index"] = (geo.prim["piece"] % 2).astype(np.int32)
        geo.point["secret"] = np.arange(geo.npoints, dtype=np.float32)
        cols, arrays = plan(geo, "pieces", "", "id", 100, "piece", "uv Cd material_index missing")
        self.assertEqual(cols["attributes"], ["vertex:uv", "point:Cd", "prim:material_index"])          # in the order asked, only existing ones
        self.assertNotIn("point:secret", arrays)
        self.assertEqual(len(arrays["point:Cd"]), len(arrays["P"]))
        self.assertEqual(len(arrays["vertex:uv"]), len(arrays["vp"]))
        self.assertEqual(len(arrays["prim:material_index"]), len(arrays["sizes"]))
        # piece 0's first face material comes from the source face of piece 0
        k0 = np.flatnonzero(geo.prim["piece"] == cols["id"][0])
        self.assertEqual(int(arrays["prim:material_index"][0]), int(geo.prim["material_index"][k0[0]]))
        # attributes follow the same rows as the shape: the uv of corner 0 is the source corner's uv
        corner0 = geo.topology.prim_offsets[k0[0]]
        np.testing.assert_allclose(arrays["vertex:uv"][0], geo.vertex["uv"][corner0])
        none, arrays2 = plan(geo, "pieces", "", "id", 100, "piece", "")
        self.assertEqual((none["attributes"], set(arrays2)), ([], {"P", "vp", "sizes"}))

    def test_the_manifest_carries_the_arrays_and_survives_the_link(self):
        geo = self.fractured()
        header, arrays = build_manifest("so", params(mode="pieces", prefix="Chunk"), geo, "k")
        self.assertEqual((header["mode"], header["prefix"]), ("pieces", "Chunk"))
        import json
        json.dumps(header, allow_nan=False)
        self.assertEqual(set(arrays), {"P", "vp", "sizes"})


class LinkTests(unittest.TestCase):
    def setUp(self):
        self.session = Session(Graph.from_dict({"nodes": FOREST, "display": "out"}))
        self.server = LinkServer(self.session, "127.0.0.1", 0)
        self.port = self.server.start()
        self.session.start()
        self.q = queue.Queue()
        self.client = LinkClient("127.0.0.1", self.port, on_message=lambda h, a: self.q.put((h, a)), client_name="test")
        self.client.start()
        self.addCleanup(self.close)

    def close(self):
        self.client.stop()
        self.server.stop()
        self.session.stop()

    def drain(self, seconds=0.5):
        out = []
        end = time.monotonic() + seconds
        while time.monotonic() < end:
            try:
                out.append(self.q.get(timeout=0.05)[0])
            except queue.Empty:
                pass
        return out

    def test_a_manifest_arrives_once_and_again_only_when_it_changes(self):
        wait_for(self.q, "welcome")
        nodes, _ = wait_for(self.q, "scene_nodes")
        self.assertEqual(nodes["nodes"], ["so"])
        m, _ = wait_for(self.q, "scene_objects")
        self.assertEqual((m["node"], m["prefix"], len(m["id"])), ("so", "Forest", 12))
        self.assertTrue(self.session.wait_idle(10))
        self.drain(0.3)
        # an unrelated edit (the main output's grid) recooks but does not change the manifest
        self.session.edit([{"op": "set", "id": "out", "params": {"object": ""}}])
        self.assertTrue(self.session.wait_idle(10))
        self.assertEqual([h["type"] for h in self.drain() if h["type"] == "scene_objects"], [])
        # a change in what the sink gets does
        self.session.edit([{"op": "set", "id": "p", "params": {"count": 20}}])
        m2, _ = wait_for(self.q, "scene_objects", timeout=10)
        self.assertEqual(len(m2["id"]), 20)
        self.assertNotEqual(m2["key"], m["key"])
        # and so does a parameter of the node itself
        self.session.edit([{"op": "set", "id": "so", "params": {"prefix": "Wood"}}])
        m3, _ = wait_for(self.q, "scene_objects", timeout=10, pred=lambda h: h["prefix"] == "Wood")
        self.assertEqual(m3["collection"], "Wood")


    def test_a_pieces_manifest_arrives_with_its_mesh_arrays(self):
        graph = {"b": {"type": "box"},
                 "pts": {"type": "voronoi_fracture_points", "inputs": {"geo": "b"}, "params": {"count": 5, "seed": 1}},
                 "f": {"type": "voronoi_fracture", "inputs": {"geo": "b", "points": "pts"}, "params": {}},
                 "a": {"type": "assemble", "inputs": {"geo": "f"}, "params": {}},
                 "so": {"type": "scene_objects", "inputs": {"geo": "a"}, "params": {"mode": "pieces", "prefix": "Chunk"}},
                 "out": {"type": "output", "inputs": {"geo": "b"}}}
        self.session.replace_graph(Graph.from_dict({"nodes": graph, "display": "out"}))
        h, arrays = wait_for(self.q, "scene_objects", timeout=10, pred=lambda h: h["mode"] == "pieces")
        self.assertEqual(set(arrays), {"P", "vp", "sizes"})
        self.assertEqual(len(arrays["sizes"]), sum(h["nprim"]))
        self.assertEqual(len(arrays["P"]), sum(h["npts"]))

    def test_a_resync_sends_it_again(self):
        wait_for(self.q, "scene_objects")
        self.drain(0.3)
        self.client.resync()
        wait_for(self.q, "scene_nodes")
        wait_for(self.q, "scene_objects")

    def test_removing_the_node_says_so(self):
        wait_for(self.q, "scene_objects")
        self.session.edit([{"op": "remove", "id": "so"}])
        h, _ = wait_for(self.q, "scene_nodes", timeout=10, pred=lambda h: h["nodes"] == [])
        self.assertEqual(h["nodes"], [])

    def test_a_bad_sink_reports_an_error_and_the_main_output_still_flows(self):
        wait_for(self.q, "scene_objects")
        self.session.edit([{"op": "set", "id": "so", "params": {"max_objects": 3}}])
        h, _ = wait_for(self.q, "error", timeout=10, pred=lambda h: "max_objects" in h.get("message", ""))
        self.assertIn("so", h["message"])
        self.assertTrue(self.session.wait_idle(10))
        self.assertIsNotNone(self.session.latest_good)
        self.assertEqual(self.session.latest_good.geo.npoints, 25)                       # the main output is the grid

    def test_the_quick_pass_of_auto_proxy_sends_no_manifest(self):
        wait_for(self.q, "scene_objects")
        self.assertTrue(self.session.wait_idle(10))
        self.session.set_proxy(level=0.1, auto=True)
        self.session._full_ms = 5000.0
        self.drain(0.2)
        self.session.edit([{"op": "set", "id": "p", "params": {"count": 7}}])
        self.assertTrue(self.session.wait_idle(15))
        manifests = [h for h in self.drain(1.0) if h["type"] == "scene_objects"]
        self.assertEqual(len(manifests), 1)                                              # only the full pass, once
        self.assertEqual(len(manifests[0]["id"]), 7)


class ModifierPathTests(unittest.TestCase):
    def test_the_modifier_cook_returns_manifests_and_errors(self):
        from minihou.link.instances import InstanceManager
        host = Geometry.from_arrays(np.array([[0, 0, 0], [4, 0, 0], [0, 4, 0], [4, 4, 0]], np.float32),
                                    [0, 1, 2, 3], [0, 4])
        graph = {"version": 1, "display": "out", "nodes": {
            "m": {"type": "modifier_input"},
            "p": {"type": "scatter", "inputs": {"geo": "m"}, "params": {"count": 9, "seed": 1}},
            "so": {"type": "scene_objects", "inputs": {"geo": "p"}, "params": {"source": "Rock", "prefix": "Rocks"}},
            "bad": {"type": "scene_objects", "inputs": {"geo": "p"}, "params": {}},
            "out": {"type": "output", "inputs": {"geo": "m"}}}}

        class FakeSession:
            path = None
            evaluator = Evaluator()

            def snapshot(self):
                return {"graph": graph}
        mgr = InstanceManager(FakeSession())
        header, _arrays = mgr.cook({"type": "instance_cook", "instance_id": "i1", "graph": graph, "params": {}}, host.freeze())
        self.assertEqual(len(header["scene"]), 1)
        self.assertEqual((header["scene"][0]["node"], len(header["scene"][0]["id"]), header["scene"][0]["sources"]), ("so", 9, ["Rock"]))
        self.assertEqual(len(header["scene_errors"]), 1)
        self.assertEqual(header["scene_errors"][0]["node"], "bad")
        self.assertIn("source", header["scene_errors"][0]["message"])


if __name__ == "__main__":
    unittest.main()
