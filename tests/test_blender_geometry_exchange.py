"""Real Blender tests of what comes into MiniHou from curves, hair curves, point clouds and mixed Geometry Nodes output: the sampling is
compared with Blender's own evaluation, then whole objects are read, then sent through the live link. Skipped without bpy; run by
tools/blender_selftest.py on CI."""
import json
import sys
import unittest

import numpy as np

try:
    import bpy
except ImportError:                                    # pragma: no cover
    bpy = None

from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
sys.path[:0] = [str(ROOT), str(ROOT / "blender_addon")]
from tests.test_blender_scene_objects import LiveHarness  # noqa: E402


def legacy_curve(kind, n, cyclic=False, order=4, endpoint=False, bezier_end=False, res=6, name=None):
    cu = bpy.data.curves.new(name or f"C{kind}", "CURVE")
    cu.dimensions = "3D"
    cu.resolution_u = res
    if kind == "BEZIER":
        sp = cu.splines.new("BEZIER")
        sp.bezier_points.add(n - 1)
        for i, p in enumerate(sp.bezier_points):
            p.co = (i * 1.5, (i % 3) * 1.0, 0.3 * i)
            p.handle_left_type = p.handle_right_type = "AUTO"
            p.radius, p.tilt = 1 + i, 0.1 * i
    elif kind == "POLY":
        sp = cu.splines.new("POLY")
        sp.points.add(n - 1)
        for i, p in enumerate(sp.points):
            p.co = (i, (i * 7 % 5), 0.2 * i, 1)
    else:
        sp = cu.splines.new("NURBS")
        sp.points.add(n - 1)
        for i, p in enumerate(sp.points):
            p.co = (i, (i * 7 % 5), 0.2 * i, 1 + 0.3 * (i % 2))
        sp.order_u, sp.use_endpoint_u, sp.use_bezier_u, sp.resolution_u = order, endpoint, bezier_end, res
    sp.use_cyclic_u = cyclic
    ob = bpy.data.objects.new(cu.name, cu)
    bpy.context.scene.collection.objects.link(ob)
    return ob


def evaluated_mesh(ob):
    ev = ob.evaluated_get(bpy.context.evaluated_depsgraph_get())
    me = ev.to_mesh()
    try:
        return np.array([tuple(v.co) for v in me.vertices]).reshape(-1, 3)
    finally:
        ev.to_mesh_clear()


def hair_curves(name, counts, positions, poly=False):
    cd = bpy.data.hair_curves.new(name)
    cd.add_curves(counts)
    if poly:
        cd.set_types(type="POLY")
    cd.attributes["position"].data.foreach_set("vector", np.asarray(positions, np.float32).ravel())
    ob = bpy.data.objects.new(name, cd)
    bpy.context.scene.collection.objects.link(ob)
    return ob


def point_cloud(name, positions, radius=None, ids=None):
    pc = bpy.data.pointclouds.new(name)
    positions = np.asarray(positions, np.float32)
    pc.resize(len(positions))
    pc.attributes["position"].data.foreach_set("vector", positions.ravel())
    if radius is not None:
        pc.attributes.new("radius", "FLOAT", "POINT").data.foreach_set("value", np.asarray(radius, np.float32))
    if ids is not None:
        pc.attributes.new("id", "INT", "POINT").data.foreach_set("value", np.asarray(ids, np.int32))
    ob = bpy.data.objects.new(name, pc)
    bpy.context.scene.collection.objects.link(ob)
    return ob


def read(ob):
    from minihou_link import mesh_reader
    return mesh_reader.read_object(ob, bpy.context.evaluated_depsgraph_get())


@unittest.skipIf(bpy is None, "needs Blender / the bpy module")
class SamplingMatchesBlenderTests(unittest.TestCase):
    CASES = [("BEZIER", 4, False, {}), ("BEZIER", 5, True, {}), ("POLY", 5, False, {}), ("POLY", 5, True, {}),
             ("NURBS", 6, False, dict(order=4)), ("NURBS", 6, False, dict(order=4, endpoint=True)), ("NURBS", 6, True, dict(order=4)),
             ("NURBS", 7, False, dict(order=3, endpoint=True)), ("NURBS", 7, False, dict(order=4, endpoint=True, bezier_end=True)),
             ("NURBS", 7, False, dict(order=4, bezier_end=True)), ("NURBS", 5, False, dict(order=2, endpoint=True)),
             ("NURBS", 8, False, dict(order=5))]

    def setUp(self):
        bpy.ops.wm.read_factory_settings(use_empty=True)

    def test_every_curve_type_is_sampled_where_blender_puts_its_points(self):
        for kind, n, cyc, kw in self.CASES:
            with self.subTest(kind=kind, n=n, cyclic=cyc, **kw):
                ob = legacy_curve(kind, n, cyc, **kw)
                want = evaluated_mesh(ob)
                _, arrays = read(ob)
                got = arrays["point/P"]
                self.assertEqual(got.shape, want.shape)
                np.testing.assert_allclose(got, want, atol=2e-5)

    def test_catmull_rom_hair_curves_match_blenders_conversion_to_a_mesh(self):
        pos = np.array([(0, 0, 0), (1, 1, 0), (2, 0, 1), (3, 1, 0), (4, 0, 0), (0, 0, 1), (1, 2, 1), (2, 0, 1)], np.float32)
        ob = hair_curves("H", [5, 3], pos)
        bpy.context.view_layer.objects.active = ob
        ob.select_set(True)
        _, arrays = read(ob)
        got = arrays["point/P"]
        bpy.ops.object.convert(target="MESH")
        want = np.array([tuple(v.co) for v in bpy.context.object.data.vertices])
        self.assertEqual(got.shape, want.shape)
        np.testing.assert_allclose(got, want, atol=2e-5)


@unittest.skipIf(bpy is None, "needs Blender / the bpy module")
class ReadObjectTests(unittest.TestCase):
    def setUp(self):
        bpy.ops.wm.read_factory_settings(use_empty=True)

    def test_a_curve_object_arrives_as_curves_with_radius_and_tilt(self):
        ob = legacy_curve("BEZIER", 3, res=4)
        header, arrays = read(ob)
        info = header["info"]
        self.assertEqual(info["components"], ["curves"])
        self.assertEqual((info["curves"], info["points"], info["prims"]), (1, 9, 1))
        np.testing.assert_array_equal(arrays["topo/prim_offsets"], [0, 9])
        np.testing.assert_array_equal(arrays["topo/prim_closed"], [0])
        np.testing.assert_allclose(arrays["point/radius"], np.linspace(1, 3, 9), atol=1e-6)
        np.testing.assert_allclose(arrays["point/tilt"], np.linspace(0, 0.2, 9), atol=1e-6)

    def test_several_splines_are_several_primitives_and_a_cyclic_one_is_closed(self):
        cu = bpy.data.curves.new("M", "CURVE")
        a = cu.splines.new("POLY")
        a.points.add(3)
        b = cu.splines.new("POLY")
        b.points.add(2)
        b.use_cyclic_u = True
        for sp in (a, b):
            for i, p in enumerate(sp.points):
                p.co = (i, len(sp.points), 0, 1)
        ob = bpy.data.objects.new("M", cu)
        bpy.context.scene.collection.objects.link(ob)
        header, arrays = read(ob)
        np.testing.assert_array_equal(arrays["topo/prim_offsets"], [0, 4, 7])
        np.testing.assert_array_equal(arrays["topo/prim_closed"], [0, 1])

    def test_hair_curves_keep_their_attributes(self):
        pos = np.array([(0, 0, 0), (1, 0, 0), (2, 0, 0), (0, 1, 0), (1, 1, 0)], np.float32)
        ob = hair_curves("H", [3, 2], pos, poly=True)
        cd = ob.data
        cd.attributes.new("strand", "INT", "CURVE").data.foreach_set("value", np.array([7, 8], np.int32))
        cd.attributes.new("age", "FLOAT", "POINT").data.foreach_set("value", np.arange(5, dtype=np.float32))
        header, arrays = read(ob)
        np.testing.assert_array_equal(arrays["prim/strand"], [7, 8])
        np.testing.assert_array_equal(arrays["point/age"], np.arange(5))
        self.assertEqual(header["info"]["curves"], 2)

    def test_a_point_cloud_arrives_as_points_with_radius_and_attributes(self):
        ob = point_cloud("PC", np.arange(12).reshape(4, 3), radius=[.1, .2, .3, .4], ids=[5, 6, 7, 8])
        header, arrays = read(ob)
        self.assertEqual(header["info"]["components"], ["points"])
        self.assertEqual((header["info"]["points"], header["info"]["prims"]), (4, 0))
        np.testing.assert_allclose(arrays["point/radius"], [.1, .2, .3, .4], atol=1e-6)
        np.testing.assert_array_equal(arrays["point/id"], [5, 6, 7, 8])

    def test_a_bevelled_curve_and_text_come_in_as_the_mesh_they_evaluate_to(self):
        ob = legacy_curve("POLY", 4)
        ob.data.bevel_depth = 0.2
        header, arrays = read(ob)
        self.assertEqual(header["info"]["components"], ["mesh"])
        self.assertGreater(header["info"]["prims"], 4)
        tc = bpy.data.curves.new("T", "FONT")
        tc.body = "Hi"
        text = bpy.data.objects.new("T", tc)
        bpy.context.scene.collection.objects.link(text)
        header, _ = read(text)
        self.assertEqual(header["info"]["components"], ["mesh"])

    def test_geometry_nodes_output_with_a_mesh_curves_and_points_is_one_geometry(self):
        ob = legacy_curve("POLY", 3, name="Host")
        ob.data.splines.clear()                                               # an empty curve to host the node tree
        ng = bpy.data.node_groups.new("Mixed", "GeometryNodeTree")
        ng.interface.new_socket("Geometry", in_out="INPUT", socket_type="NodeSocketGeometry")
        ng.interface.new_socket("Geometry", in_out="OUTPUT", socket_type="NodeSocketGeometry")
        out = ng.nodes.new("NodeGroupOutput")
        join = ng.nodes.new("GeometryNodeJoinGeometry")
        grid = ng.nodes.new("GeometryNodeMeshGrid")
        line = ng.nodes.new("GeometryNodeCurvePrimitiveLine")
        line.inputs["End"].default_value = (0, 0, 3)
        to_points = ng.nodes.new("GeometryNodeMeshToPoints")
        cube = ng.nodes.new("GeometryNodeMeshCube")
        for src, dst in ((grid.outputs["Mesh"], join.inputs[0]), (line.outputs["Curve"], join.inputs[0]),
                         (cube.outputs["Mesh"], to_points.inputs["Mesh"]), (to_points.outputs["Points"], join.inputs[0]),
                         (join.outputs["Geometry"], out.inputs[0])):
            ng.links.new(src, dst)
        mod = ob.modifiers.new("Mixed", "NODES")
        mod.node_group = ng
        header, arrays = read(ob)
        comps = header["info"]["components"]
        self.assertIn("mesh", comps)
        self.assertIn("curves", comps)
        self.assertIn("points", comps)
        n = len(arrays["point/P"])
        self.assertEqual(int(arrays["topo/prim_offsets"][-1]), len(arrays["topo/vertex_point"]))
        self.assertLess(int(arrays["topo/vertex_point"].max()), n)
        closed = arrays["topo/prim_closed"]
        self.assertGreater(int(closed.sum()), 0)                              # the grid's faces
        self.assertGreater(int((closed == 0).sum()), 0)                       # the line

    def test_an_empty_curve_object_is_an_error_not_a_crash(self):
        cu = bpy.data.curves.new("E", "CURVE")
        ob = bpy.data.objects.new("E", cu)
        bpy.context.scene.collection.objects.link(ob)
        with self.assertRaises(ValueError):
            read(ob)


def graph_reading(name):
    return {"version": 1, "display": "main", "nodes": {
        "src": {"type": "blender_object", "params": {"object": name, "space": "object"}},
        "main": {"type": "output", "inputs": {"geo": "src"}}}}


@unittest.skipIf(bpy is None, "needs Blender / the bpy module")
class LiveCurveTests(LiveHarness, unittest.TestCase):
    """Whole path: a Blender object -> add-on -> engine -> graph -> output object back in Blender."""

    def initial_graph(self):
        return graph_reading("Strand")

    def output_mesh(self):
        ob = bpy.data.objects.get("MiniHou_Output")
        return ob.data if ob is not None else None

    def test_a_bezier_curve_goes_through_the_graph_and_comes_out_as_edges(self):
        legacy_curve("BEZIER", 4, res=5, name="Strand")
        self.pump(lambda: self.output_mesh() is not None and len(self.output_mesh().vertices) == 16)
        mesh = self.output_mesh()
        self.assertEqual((len(mesh.vertices), len(mesh.edges), len(mesh.polygons)), (16, 15, 0))
        self.assertIn("radius", [a.name for a in mesh.attributes])

    def test_a_point_cloud_comes_out_as_vertices_with_its_attributes(self):
        point_cloud("Strand", np.arange(15).reshape(5, 3), radius=np.linspace(.1, .5, 5), ids=np.arange(5))
        self.pump(lambda: self.output_mesh() is not None and len(self.output_mesh().vertices) == 5)
        mesh = self.output_mesh()
        self.assertEqual((len(mesh.edges), len(mesh.polygons)), (0, 0))
        names = [a.name for a in mesh.attributes]
        self.assertIn("radius", names)
        self.assertIn("id", names)

    def test_hair_curves_come_through_as_separate_strands(self):
        hair_curves("Strand", [4, 3], np.arange(21, dtype=np.float32).reshape(7, 3), poly=True)
        self.pump(lambda: self.output_mesh() is not None and len(self.output_mesh().vertices) == 7)
        self.assertEqual(len(self.output_mesh().edges), 3 + 2)


def graph_sending(name, mode, **params):
    return {"version": 1, "display": "main", "nodes": {
        "src": {"type": "blender_object", "params": {"object": name, "space": "object"}},
        "so": {"type": "scene_objects", "inputs": {"geo": "src"}, "params": dict({"mode": mode, "prefix": "Out"}, **params)},
        "main": {"type": "output", "inputs": {"geo": "src"}}}}


def made(tag="session:so"):
    return sorted((o for o in bpy.data.objects if o.get("minihou_scene") == tag), key=lambda o: o.get("minihou_id", 0))


@unittest.skipIf(bpy is None, "needs Blender / the bpy module")
class CurvesOutTests(LiveHarness, unittest.TestCase):
    """A Blender curves object through the engine and back as a real Blender curves object (Scene Objects, mode curves)."""

    def initial_graph(self):
        return graph_sending("Src", "curves")

    def setUp(self):
        super().setUp()
        pos = np.array([(0, 0, 0), (1, 0, 0), (2, 1, 0), (0, 2, 0), (1, 2, 0), (2, 2, 1)], np.float32)
        self.src = hair_curves("Src", [3, 3], pos, poly=True)
        a = self.src.data.attributes.new("radius", "FLOAT", "POINT")
        a.data.foreach_set("value", np.linspace(.1, .6, 6, dtype=np.float32))
        self.src.data.attributes.new("strand", "INT", "CURVE").data.foreach_set("value", np.array([4, 9], np.int32))

    def out(self):
        objs = made()
        return objs[0] if objs and objs[0].type == "CURVES" and len(objs[0].data.points) else None

    def test_curves_come_out_as_a_curves_object_with_the_same_points_radius_and_attributes(self):
        self.pump(lambda: self.out() is not None)
        o = self.out()
        self.assertEqual(o.name, "Out_0000")
        self.assertEqual((len(o.data.curves), len(o.data.points)), (2, 6))
        header, arrays = read(o)
        np.testing.assert_allclose(arrays["point/P"], read(self.src)[1]["point/P"], atol=1e-6)
        np.testing.assert_allclose(arrays["point/radius"], np.linspace(.1, .6, 6), atol=1e-6)

    def test_a_curve_made_of_a_polygon_outline_is_cyclic_and_the_user_keeps_modifiers_on_the_object(self):
        self.pump(lambda: self.out() is not None)
        o = self.out()
        o.modifiers.new("Mine", "NODES")
        self.write(graph_sending("Src", "curves", prefix="Out", attributes="radius strand"))        # same shape, another manifest
        self.pump(lambda: "session:so" in self.so.last_summary)
        self.assertEqual(len(made()), 1)
        self.assertEqual([m.name for m in made()[0].modifiers], ["Mine"])

    def test_changing_the_curves_rewrites_the_data_and_keeps_the_object(self):
        self.pump(lambda: self.out() is not None)
        first = self.out()
        pos = np.array([(0, 0, 0), (1, 0, 0), (2, 1, 0), (0, 2, 0)], np.float32)
        hair_curves("Src2", [4], pos, poly=True)
        self.write(graph_sending("Src2", "curves"))
        self.pump(lambda: self.out() is not None and len(self.out().data.points) == 4)
        self.assertEqual(self.out().name, first.name)
        self.assertEqual(len(made()), 1)


@unittest.skipIf(bpy is None, "needs Blender / the bpy module")
class PointsOutTests(LiveHarness, unittest.TestCase):
    def initial_graph(self):
        return graph_sending("Src", "points", attributes="Cd id")

    def setUp(self):
        super().setUp()
        self.src = point_cloud("Src", np.arange(12).reshape(4, 3), radius=[.1, .2, .3, .4], ids=[5, 6, 7, 8])

    def test_a_point_cloud_comes_out_as_a_point_cloud_with_radius_and_attributes(self):
        self.pump(lambda: made() and made()[0].type == "POINTCLOUD" and len(made()[0].data.points) == 4)
        o = made()[0]
        header, arrays = read(o)
        np.testing.assert_allclose(arrays["point/P"], np.arange(12).reshape(4, 3), atol=1e-6)
        np.testing.assert_allclose(arrays["point/radius"], [.1, .2, .3, .4], atol=1e-6)
        np.testing.assert_array_equal(arrays["point/id"], [5, 6, 7, 8])


@unittest.skipIf(bpy is None, "needs Blender / the bpy module")
class EngineMadeCurvesOutTests(LiveHarness, unittest.TestCase):
    """Curves made in MiniHou itself, closed ones included, and a mode change on the same node."""

    def initial_graph(self):
        return {"version": 1, "display": "main", "nodes": {
            "c": {"type": "curve", "params": {"points": "0,0,0 2,0,0 2,2,0 0,2,0", "closed": True}},
            "so": {"type": "scene_objects", "inputs": {"geo": "c"}, "params": {"mode": "curves", "prefix": "Loop"}},
            "main": {"type": "output", "inputs": {"geo": "c"}}}}

    def test_a_closed_curve_becomes_a_cyclic_curve(self):
        self.pump(lambda: made() and made()[0].type == "CURVES" and len(made()[0].data.points) == 4)
        o = made()[0]
        cyc = np.empty(1, np.int8)
        o.data.attributes["cyclic"].data.foreach_get("value", cyc)
        self.assertEqual(int(cyc[0]), 1)
        self.assertEqual(o.name, "Loop_0000")

    def test_switching_the_node_to_points_replaces_the_object_with_a_point_cloud(self):
        self.pump(lambda: made() and made()[0].type == "CURVES")
        g = self.initial_graph()
        g["nodes"]["so"]["params"]["mode"] = "points"
        self.write(g)
        self.pump(lambda: made() and made()[0].type == "POINTCLOUD" and len(made()[0].data.points) == 4)
        self.assertEqual(len(made()), 1)


if __name__ == "__main__":
    unittest.main()
