"""Volumes: surface nets, mesh SDFs, remesh, points_to_surface, sdf_boolean -- checked by volume and watertightness."""
import unittest

import numpy as np

from minihou import Evaluator, Geometry, Graph, NodeError, sdf, topo
from minihou.nodes.normal import prim_normals


def cook(nodes):
    nodes = dict(nodes)
    last = list(nodes)[-1]
    nodes["o"] = {"type": "output", "inputs": {"geo": last}}
    g = Evaluator().cook(Graph.from_dict({"nodes": nodes, "display": "o"}))
    g.validate()
    return g


def closed(g):
    return bool(len(g.P)) and bool(np.all(topo.edges(g).count == 2))


def volume(g):
    return float(np.einsum("ij,ij->", prim_normals(g), topo.prim_centroids(g))) / 6


BOX = {"b": {"type": "box", "params": {"size": [2, 2, 2]}}}


class SurfaceNetsTests(unittest.TestCase):
    def test_sphere_from_an_analytic_field(self):
        h, lo = 0.1, np.array([-1.5] * 3)
        P = lo + np.stack(np.meshgrid(*[np.arange(31)] * 3, indexing="ij"), -1) * h
        pts, quads = sdf.surface_nets(sdf.Volume(lo, h, (np.linalg.norm(P, axis=-1) - 1.0).astype(np.float32)))
        g = Geometry.from_arrays(pts.astype(np.float32), quads.reshape(-1), np.arange(0, quads.size + 1, 4))
        self.assertTrue(closed(g))
        self.assertAlmostEqual(volume(g), 4 / 3 * np.pi, delta=0.08)       # outward and the right size
        r = np.linalg.norm(pts, axis=1)
        self.assertTrue(np.all(np.abs(r - 1) < 0.02))

    def test_mesh_sdf_signs(self):
        from minihou.nodes.volume import _tris
        g = cook(BOX)
        vol = sdf.from_mesh(_tris(g, "t"), 0.1)
        c = tuple(np.array(vol.values.shape) // 2)
        self.assertLess(vol.values[c], 0)                                  # inside is negative
        self.assertGreater(vol.values[0, 0, 0], 0)


class VolumeNodeTests(unittest.TestCase):
    def test_remesh_offset_smooth_transfer(self):
        g = cook({**BOX, "c": {"type": "color", "inputs": {"geo": "b"}, "params": {"color": [1, 0, 0]}},
                  "r": {"type": "remesh", "inputs": {"geo": "c"}, "params": {"voxel_size": 0.1, "transfer": "Cd"}}})
        self.assertTrue(closed(g))
        self.assertAlmostEqual(volume(g), 8.0, delta=0.4)
        np.testing.assert_allclose(g.point["Cd"], np.tile([1, 0, 0], (g.npoints, 1)))
        grown = cook({**BOX, "r": {"type": "remesh", "inputs": {"geo": "b"}, "params": {"voxel_size": 0.1, "offset": 0.25}}})
        self.assertGreater(volume(grown), 8.0 + 2)                          # offset adds ~ area x 0.25
        smooth = cook({**BOX, "r": {"type": "remesh", "inputs": {"geo": "b"}, "params": {"voxel_size": 0.1, "smooth": 4}}})
        self.assertLess(volume(smooth), volume(g))                          # rounded corners lose volume
        with self.assertRaises(NodeError):
            cook({**BOX, "r": {"type": "remesh", "inputs": {"geo": "b"}, "params": {"voxel_size": 0.0001}}})

    def test_points_to_surface(self):
        one = cook({"p": {"type": "curve", "params": {"points": "0,0,0 3,0,0"}},     # two points far apart
                    "s": {"type": "points_to_surface", "inputs": {"points": "p"},
                          "params": {"radius": 0.5, "blend": 1.0, "voxel_size": 0.04}}})
        self.assertTrue(closed(one))
        r = np.linalg.norm(one.P, axis=1)
        self.assertAlmostEqual(float(np.median(r[r < 2])), 0.5, delta=0.02)  # a lone point: a sphere of its radius
        two = cook({"p": {"type": "curve", "params": {"points": "0,0,0 0.9,0,0"}},
                    "s": {"type": "points_to_surface", "inputs": {"points": "p"},
                          "params": {"radius": 0.5, "blend": 1.0, "voxel_size": 0.05}}})
        self.assertEqual(len(np.unique(topo.edges(two).count)), 1)
        from minihou.nodes.attrib_tools import piece_labels
        self.assertEqual(int(piece_labels(two)[1].max()) + 1, 1)            # close points melt into one blob

    def test_booleans(self):
        nodes = {**BOX, "s": {"type": "sphere", "params": {"radius": 1.2, "rows": 24, "columns": 32, "center": [1, 1, 1]}}}
        octant = np.pi * 1.2 ** 3 / 6
        expect = {"subtract": 8 - octant, "intersect": octant, "union": 8 + 4 / 3 * np.pi * 1.2 ** 3 - octant}
        for op, want in expect.items():
            g = cook({**nodes, "x": {"type": "sdf_boolean", "inputs": {"a": "b", "b": "s"},
                                     "params": {"operation": op, "voxel_size": 0.08}}})
            self.assertTrue(closed(g), op)
            self.assertAlmostEqual(volume(g), want, delta=0.06 * want + 0.1, msg=op)

    def test_curves_are_refused(self):
        with self.assertRaises(NodeError):
            cook({"l": {"type": "line"}, "r": {"type": "remesh", "inputs": {"geo": "l"}}})


if __name__ == "__main__":
    unittest.main()
