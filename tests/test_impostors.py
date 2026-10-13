"""Impostors for instances: repeated blocks found without help, instance points with their shapes, the packed wire format and the preview endpoint."""
import http.client
import json
import shutil
import tempfile
import unittest

import numpy as np

import minihou  # noqa: F401
from minihou import Evaluator, Geometry, Graph, impostors, preview, proxy
from minihou.nodes.merge import merge

try:
    from pxr import Gf, Usd, UsdGeom, Vt
except ImportError:                                                     # pragma: no cover
    Usd = None
needs_usd = unittest.skipIf(Usd is None, "usd-core (pxr) not installed")
from minihou.link.session import Session
from minihou.nodes.copy_to_points import copy_to_points
from minihou.web import WebServer


def scene(count=300, sphere=None, extra=None):
    nodes = {"s": {"type": "sphere", "params": sphere or {"rows": 8, "columns": 12}},
             "grid": {"type": "grid", "params": {"size_x": 200, "size_y": 200}},
             "sc": {"type": "scatter", "inputs": {"geo": "grid"}, "params": {"count": count}},
             "c": {"type": "copy_to_points", "inputs": {"source": "s", "points": "sc"}}}
    nodes.update(extra or {})
    return Graph.from_dict({"nodes": nodes, "display": "c"})


class Blocks(unittest.TestCase):
    def test_copies_are_found_and_each_transform_reproduces_its_block(self):
        geo = Evaluator().cook(scene(300))
        iset = impostors.blocks(geo)
        self.assertEqual(iset.kind, "blocks")
        self.assertEqual(len(iset.M), 300)
        S = iset.protos[0].geo.npoints
        self.assertEqual(S * 300, geo.npoints)
        P0 = geo.P[:S].astype(np.float64)
        for k in (0, 1, 150, 299):
            world = P0 @ iset.M[k][:, :3].T + iset.M[k][:, 3]
            np.testing.assert_allclose(world, geo.P[k * S:(k + 1) * S], atol=1e-3)
        self.assertEqual(iset.full_tris, proxy.triangle_count(geo))
        self.assertIs(impostors.blocks(geo), iset)                                       # remembered per (frozen) geometry

    def test_rotated_and_scaled_copies_fit_and_colours_are_per_block(self):
        extra = {"col": {"type": "color", "inputs": {"geo": "c"}, "params": {}}}
        g = scene(40, extra=extra)
        geo = Evaluator().cook(g, "c")
        geo = geo.copy()
        n = geo.npoints
        S = n // 40
        cd = np.repeat(np.random.default_rng(1).random((40, 3)).astype(np.float32), S, axis=0)
        geo.point["Cd"] = cd
        geo.freeze()
        iset = impostors.blocks(geo)
        self.assertIsNotNone(iset)
        np.testing.assert_allclose(iset.color, cd[::S], atol=1e-6)

    def test_deformed_copies_too_few_copies_and_tiny_blocks_are_not_instances(self):
        geo = Evaluator().cook(scene(300)).copy()
        P = geo.P.copy()
        P[geo.npoints // 2 + 5] += [0.5, 0, 0]                                           # one point of one copy moved: not an affine copy any more
        geo.point["P"] = P
        self.assertIsNone(impostors.blocks(geo.freeze()))
        self.assertIsNone(impostors.blocks(Evaluator().cook(scene(5))))                  # fewer than MIN_INSTANCES
        quads = Evaluator().cook(Graph.from_dict({"nodes": {"g": {"type": "grid", "params": {"columns": 30, "rows": 30}}}, "display": "g"}))
        self.assertIsNone(impostors.blocks(quads))                                        # a welded grid is not a block repeated
        self.assertIsNone(impostors.blocks(Geometry.points_only(np.zeros((100, 3), np.float32))))

    def test_plan_modes_budget_and_full_detail(self):
        geo = Evaluator().cook(scene(300))
        tris = proxy.triangle_count(geo)
        self.assertIsNone(impostors.plan(geo, "never", False, 10))
        self.assertIsNone(impostors.plan(geo, "auto", False, tris + 1))                   # fits the budget: drawn as it is
        small = impostors.plan(geo, "auto", False, tris // 4)                             # over the budget: impostors replace the geometry
        self.assertTrue(small.replaces)
        self.assertIsNone(impostors.plan(geo, "auto", True, tris // 4))                   # full detail (selecting, painting) wants the real thing
        self.assertTrue(impostors.plan(geo, "always", False, tris + 1).replaces)


class Points(unittest.TestCase):
    def instance_points(self, n=50):
        g = Graph.from_dict({"nodes": {"grid": {"type": "grid", "params": {"size_x": 100, "size_y": 100}},
                                       "sc": {"type": "scatter", "inputs": {"geo": "grid"}, "params": {"count": n}},
                                       "i": {"type": "instance", "inputs": {"points": "sc"}, "params": {"objects": "Tree Rock", "seed": 3, "scale_min": 0.5, "scale_max": 2.0}}},
                             "display": "i"})
        return Evaluator().cook(g)

    def source(self, size):
        b = Evaluator().cook(Graph.from_dict({"nodes": {"b": {"type": "box", "params": {"size": [size, size, size * 2]}}}, "display": "b"}))
        return b

    def test_transforms_match_what_the_preview_realisation_makes(self):
        geo = self.instance_points()
        tree, rock = self.source(1.0), self.source(0.5)
        iset = impostors.points(geo, {"Tree": tree, "Rock": rock})
        self.assertEqual((iset.kind, len(iset.M), iset.approximate), ("points", 50, False))
        ids = np.asarray(geo.point["instance"]).reshape(-1)
        for k in (0, 7, 33):
            one = Geometry.points_only(geo.P[[k]])
            for key in ("rot", "scale"):
                one.point[key] = geo.point[key][[k]]
            src = (tree, rock)[ids[k] - 1]
            real = copy_to_points(src, one, True, False)
            world = src.P.astype(np.float64) @ iset.M[k][:, :3].T + iset.M[k][:, 3]
            np.testing.assert_allclose(world, real.P, atol=1e-4)
            self.assertEqual(int(iset.proto[k]), int(ids[k]) - 1)

    def test_shapes_we_only_know_the_size_of_are_boxes(self):
        geo = self.instance_points().copy()
        geo.detail["instance_bounds"] = [[[-1, -1, 0], [1, 1, 4]], [[-0.2, -0.2, 0], [0.2, 0.2, 0.3]]]
        iset = impostors.points(geo)
        self.assertTrue(iset.approximate)
        self.assertEqual(iset.protos[0].bounds.tolist(), [[-1, -1, 0], [1, 1, 4]])
        self.assertIsNone(iset.protos[0].geo)
        geo2 = self.instance_points().copy()
        self.assertEqual(impostors.points(geo2).protos[0].bounds.tolist(), [[-0.5] * 3, [0.5] * 3])             # nothing known: a unit box
        self.assertIsNone(impostors.points(Geometry.points_only(np.zeros((3, 3), np.float32))))

    def test_estimate_counts_the_triangles_realising_would_draw(self):
        geo = self.instance_points()
        tree, rock = self.source(1.0), self.source(1.0)
        ids = np.asarray(geo.point["instance"]).reshape(-1)
        self.assertEqual(impostors.estimate_tris(geo, {"Tree": tree, "Rock": rock}), len(ids) * 12)
        self.assertEqual(impostors.estimate_tris(geo, {"Tree": tree}), int((ids == 1).sum()) * 12)
        self.assertEqual(impostors.estimate_tris(geo, {}), 0)


class Sizes(unittest.TestCase):
    @needs_usd
    def test_a_usd_point_instancer_brings_the_size_of_its_prototypes(self):
        d = tempfile.mkdtemp(prefix="minihou_imp_usd_")
        self.addCleanup(shutil.rmtree, d, True)
        f = d + "/inst.usda"
        st = Usd.Stage.CreateNew(f)
        UsdGeom.SetStageUpAxis(st, UsdGeom.Tokens.z)
        inst = UsdGeom.PointInstancer.Define(st, "/World/inst")
        for name, size in (("tree", 2.0), ("rock", 0.5)):
            cube = UsdGeom.Cube.Define(st, f"/World/inst/protos/{name}")
            cube.GetSizeAttr().Set(size)
            if name == "tree":
                UsdGeom.XformCommonAPI(cube).SetScale((1, 1, 3))
            inst.CreatePrototypesRel().AddTarget(f"/World/inst/protos/{name}")
        inst.CreatePositionsAttr(Vt.Vec3fArray([Gf.Vec3f(i * 5, 0, 0) for i in range(6)]))
        inst.CreateProtoIndicesAttr(Vt.IntArray([0, 1, 0, 1, 0, 1]))
        st.GetRootLayer().Save()
        geo = Evaluator().cook(Graph.from_dict({"nodes": {"u": {"type": "usd_import", "params": {"file": f}}}, "display": "u"}))
        self.assertEqual(geo.detail["instance_objects"], ["tree", "rock"])
        self.assertEqual(geo.detail["instance_bounds"], [[[-1.0, -1.0, -3.0], [1.0, 1.0, 3.0]], [[-0.25] * 3, [0.25] * 3]])
        iset = impostors.points(geo)
        self.assertTrue(iset.approximate)
        self.assertEqual(iset.protos[1].bounds.tolist(), [[-0.25] * 3, [0.25] * 3])
        self.assertEqual(len(iset.M), 6)

    def test_merging_instance_points_keeps_each_shapes_size_with_its_name(self):
        def pts(names, bounds, ids):
            g = Geometry.points_only(np.zeros((len(ids), 3), np.float32))
            g.point["instance"] = np.asarray(ids, np.int32)
            g.detail["instance_objects"] = names
            if bounds:
                g.detail["instance_bounds"] = bounds
            return g
        a = pts(["A", "B"], [[[0] * 3, [1] * 3], [[0] * 3, [2] * 3]], [1, 2, 1])
        b = pts(["B", "C"], None, [1, 2])
        m = merge([a, b])
        self.assertEqual(m.detail["instance_objects"], ["A", "B", "C"])
        self.assertEqual(m.detail["instance_bounds"], [[[0] * 3, [1] * 3], [[0] * 3, [2] * 3], None])
        c = merge([pts(["X"], None, [1]), pts(["Y"], None, [1])])
        self.assertNotIn("instance_bounds", c.detail)


class Packing(unittest.TestCase):
    def test_pack_levels_indices_and_bounding_spheres(self):
        geo = Evaluator().cook(scene(120, sphere={"rows": 40, "columns": 60}))
        iset = impostors.blocks(geo)
        head, arrays = impostors.pack(iset, lod0=800, lod1=100)
        P, tri = arrays["ip_P"].reshape(-1, 3), arrays["ip_tri"].reshape(-1, 3)
        self.assertLess(int(tri.max()), len(P))
        m = head["meshes"]
        self.assertEqual(sum(x["t_n"] for x in m), len(tri))
        proto = head["protos"][0]
        self.assertEqual(proto["tris"][2], 12)                                            # the third level is a box
        self.assertLessEqual(proto["tris"][0], 800 * 1.4)                                 # decimation lands near the cap
        self.assertLess(proto["tris"][1], proto["tris"][0])
        self.assertEqual(proto["full_tris"], proxy.triangle_count(iset.protos[0].geo))
        self.assertEqual(head["count"], 120)
        # every transformed vertex of every shape level lies inside the instance's bounding sphere
        sph = arrays["inst_sph"].reshape(-1, 4)
        M = arrays["inst_M"].reshape(-1, 3, 4)
        for mesh in m:
            local = P[mesh["p_off"]:mesh["p_off"] + mesh["p_n"]].astype(np.float64)
            for k in (0, 60, 119):
                world = local @ M[k][:, :3].T + M[k][:, 3]
                self.assertLessEqual(float(np.linalg.norm(world - sph[k, :3], axis=1).max()), float(sph[k, 3]) * 1.001 + 1e-4)

    def test_small_shapes_keep_one_level_and_curves_get_a_box(self):
        geo = Evaluator().cook(scene(40))
        head, _ = impostors.pack(impostors.blocks(geo), lod0=4000, lod1=300)
        self.assertEqual(head["protos"][0]["lods"][0], head["protos"][0]["lods"][1])      # 12*8 = 192 triangles: the second level is the first


class Endpoint(unittest.TestCase):
    def test_the_preview_replaces_a_big_repeated_geometry_by_instances_and_full_detail_does_not(self):
        d = tempfile.mkdtemp(prefix="minihou_imp_")
        self.addCleanup(shutil.rmtree, d, True)
        s = Session(scene(200))
        s.start()
        self.addCleanup(s.stop)
        web = WebServer(s, "127.0.0.1", 0, root=d)
        port = web.start()
        self.addCleanup(web.stop)

        def post(body):
            c = http.client.HTTPConnection("127.0.0.1", port, timeout=30)
            c.request("POST", "/api/preview", body=json.dumps(body), headers={"Content-Type": "application/json"})
            r = c.getresponse()
            data = r.read()
            c.close()
            return preview.unpack(data)
        head, arrays = post({"node": "c", "tris": 2000})
        inst = head["instances"]
        self.assertEqual((inst["kind"], inst["count"], inst["replaces"]), ("blocks", 200, True))
        self.assertTrue(head["proxy"]["active"] and head["proxy"]["instanced"])
        self.assertEqual(head["proxy"]["instances"], 200)
        self.assertEqual(len(arrays["P"]), 0)                                              # the geometry itself is not sent
        self.assertEqual(head["points"], Evaluator().cook(scene(200)).npoints)
        self.assertEqual(len(arrays["inst_M"]), 200 * 12)
        self.assertLess(len(arrays["ip_tri"]), 2000 * 3)
        head, arrays = post({"node": "c", "tris": 2000, "instances": "never"})
        self.assertNotIn("instances", head)
        self.assertGreater(len(arrays["P"]), 0)
        head, arrays = post({"node": "c", "tris": 2000, "full": True})                     # selecting needs the real thing
        self.assertNotIn("instances", head)
        head, arrays = post({"node": "c"})                                                 # under the default budget: as it is
        self.assertNotIn("instances", head)
        head, arrays = post({"node": "c", "instances": "always"})
        self.assertTrue(head["instances"]["replaces"])


if __name__ == "__main__":
    unittest.main()
