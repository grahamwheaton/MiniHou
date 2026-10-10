import unittest

import numpy as np

from minihou import Geometry, Topology
from tests.helpers import quad_geo


class GeometryTests(unittest.TestCase):
    def test_from_arrays_normalizes_dtypes(self):
        g = Geometry.from_arrays(np.zeros((3, 3), np.float64), np.array([0, 1, 2], np.int64), [0, 3])
        self.assertEqual(g.P.dtype, np.float32)
        self.assertEqual(g.topology.vertex_point.dtype, np.int32)
        self.assertEqual((g.npoints, g.nvertices, g.nprims), (3, 3, 1))
        g.validate()

    def test_attribute_length_is_enforced(self):
        g = quad_geo()
        with self.assertRaises(ValueError):
            g.point["Cd"] = np.zeros((5, 3))
        g.point["Cd"] = np.ones((4, 3))
        g.prim["id"] = np.array([7])
        with self.assertRaises(ValueError):
            g.prim["id"] = np.array([7, 8])

    def test_bad_dtype_rejected(self):
        g = quad_geo()
        with self.assertRaises(TypeError):
            g.point["name"] = np.array(["a", "b", "c", "d"])

    def test_freeze_makes_arrays_readonly_and_tables_immutable(self):
        g = quad_geo()
        g.point["Cd"] = np.ones((4, 3))
        g.freeze()
        with self.assertRaises(ValueError):
            g.P[0, 0] = 5.0                      # numpy: read-only array
        with self.assertRaises(ValueError):
            g.point["new"] = np.zeros(4)         # table frozen
        with self.assertRaises(ValueError):
            g.topology.vertex_point[0] = 2

    def test_copy_is_shallow_and_mutable(self):
        g = quad_geo()
        g.freeze()
        c = g.copy()
        self.assertIs(c.P, g.P)                  # shared array
        self.assertIs(c.topology, g.topology)
        c.point["P"] = c.P + 1                   # replacing is fine
        self.assertFalse(np.array_equal(c.P, g.P))
        self.assertTrue(g.frozen and not c.frozen)

    def test_topology_validation(self):
        with self.assertRaises(ValueError):
            Geometry.from_arrays(np.zeros((2, 3)), [0, 5], [0, 2]).validate()      # index out of range
        with self.assertRaises(ValueError):
            Geometry.from_arrays(np.zeros((3, 3)), [0, 1, 2], [0, 2]).validate()   # offsets end != nverts
        with self.assertRaises(ValueError):
            Geometry.from_arrays(np.zeros((3, 3)), [0, 1, 2], [1, 3]).validate()   # doesn't start at 0

    def test_validate_requires_P(self):
        with self.assertRaises(ValueError):
            Geometry(3).validate()

    def test_topology_digest_tracks_content(self):
        a = Topology.from_polys([[0, 1, 2, 3]])
        b = Topology.from_polys([[0, 1, 2, 3]])
        c = Topology.from_polys([[0, 1, 3, 2]])
        self.assertEqual(a.digest, b.digest)
        self.assertNotEqual(a.digest, c.digest)
        self.assertEqual(a.polys(), [[0, 1, 2, 3]])

    def test_points_only_has_no_prims(self):
        g = Geometry.points_only(np.zeros((10, 3)))
        g.validate()
        self.assertEqual((g.nvertices, g.nprims), (0, 0))


if __name__ == "__main__":
    unittest.main()
