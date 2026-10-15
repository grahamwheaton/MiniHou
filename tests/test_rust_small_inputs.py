"""The Rust kernels are parallel and cost a fixed ~0.2 ms per call: they must not be used for the many small meshes of a
fracture or a scatter, where numpy is faster (fractured_rock.json was 2.8x slower with Rust on because of this)."""
import unittest
from unittest import mock

import numpy as np

import minihou  # noqa: F401
from minihou import rust, topo
from minihou.geometry import Geometry


def quad_grid(n):
    idx = np.arange((n + 1) * (n + 1), dtype=np.int32).reshape(n + 1, n + 1)
    quads = np.stack([idx[:-1, :-1], idx[:-1, 1:], idx[1:, 1:], idx[1:, :-1]], -1).reshape(-1)
    offs = np.arange(0, quads.size + 1, 4, dtype=np.int32)
    return Geometry.from_arrays(np.zeros(((n + 1) * (n + 1), 3), np.float32), quads, offs)


@unittest.skipUnless(rust.available(), "the Rust core is not built")
class SmallInputTests(unittest.TestCase):
    def test_small_meshes_do_not_call_the_rust_edge_kernel(self):
        small = quad_grid(20)                                   # 1600 vertices
        self.assertLess(small.topology.nvertices, topo._NUMBA_EDGES)
        with mock.patch.object(rust, "edges", side_effect=AssertionError("Rust used for a small mesh")):
            topo.edges(small)                                   # must not raise

    def test_large_meshes_still_do_(self):
        large = quad_grid(100)                                  # 40000 vertices
        self.assertGreaterEqual(large.topology.nvertices, topo._NUMBA_EDGES)
        with mock.patch.object(rust, "edges", wraps=rust.edges) as spy:
            topo.edges(large)
        self.assertEqual(spy.call_count, 1)

    def test_numbering_is_identical_across_the_threshold(self):
        for n in (49, 50, 51, 70):                              # 19600 .. 19600+ vertices around the 20000 threshold
            g = quad_grid(n)
            with rust.disabled():
                ref = topo.edges(g)
            got = topo.edges(g)
            with self.subTest(vertices=g.topology.nvertices):
                np.testing.assert_array_equal(got.points, ref.points)
                np.testing.assert_array_equal(got.of_vertex, ref.of_vertex)
                np.testing.assert_array_equal(got.count, ref.count)


if __name__ == "__main__":
    unittest.main()
