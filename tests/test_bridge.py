"""bridge: bands of faces between pairs of open boundary loops."""
import unittest

import numpy as np

from minihou.nodes.bevel import twins
from tests.test_nodes import run


def pipes(columns_a=16, columns_b=16, gap=1.0, turn_b=0.0, radius=0.5, height=1.0):
    a = run("tube", columns=columns_a, rows=1, height=height, radius_bottom=radius, radius_top=radius, caps=False)
    b = run("tube", columns=columns_b, rows=1, height=height, radius_bottom=radius, radius_top=radius, caps=False)
    b = run("transform", b, translate=[0, 0, height + gap], rotate=[0, 0, turn_b])
    return run("merge", [a, b])


def volume(g):
    P, vp, offs = g.P.astype(float), g.topology.vertex_point, g.topology.prim_offsets
    v = 0.0
    for p in range(g.nprims):
        q = P[vp[offs[p]:offs[p + 1]]]
        for k in range(1, len(q) - 1):
            v += np.dot(q[0], np.cross(q[k], q[k + 1])) / 6.0
    return v


class BridgeTests(unittest.TestCase):
    def test_two_equal_pipes_become_one_open_pipe_with_outward_faces(self):
        g = pipes()
        self.assertEqual(int((twins(g) < 0).sum()), 4 * 16)                   # four open rims before
        out = run("bridge", g)
        self.assertEqual(out.nprims, g.nprims + 16)                           # the facing ends only: top of A, bottom of B
        self.assertEqual(int((twins(out) < 0).sum()), 2 * 16)                 # only the far ends stay open
        new = np.arange(g.nprims, out.nprims)
        vp, offs = out.topology.vertex_point, out.topology.prim_offsets
        P = out.P.astype(float)
        for p in new:
            q = P[vp[offs[p]:offs[p + 1]]]
            self.assertEqual(len(q), 4)
            n = np.cross(q[1] - q[0], q[2] - q[0])
            radial = q.mean(0) * [1, 1, 0]
            self.assertGreater(float(n @ radial), 0.0)                        # facing outwards like the pipes

    def test_a_turned_second_pipe_is_aligned_without_twisting(self):
        g = pipes(turn_b=360.0 / 16 * 5)                                      # five steps of rotation
        aligned = run("bridge", g)
        raw = run("bridge", g, align=False)
        def strip_area(o):
            P, vp, offs = o.P.astype(float), o.topology.vertex_point, o.topology.prim_offsets
            tot = 0.0
            for p in range(g.nprims, o.nprims):
                q = P[vp[offs[p]:offs[p + 1]]]
                tot += 0.5 * np.linalg.norm(np.cross(q[2] - q[0], q[3] - q[1]))
            return tot
        self.assertAlmostEqual(strip_area(aligned), 2 * 16 * 0.5 * np.sin(np.pi / 16) * 1.0, delta=0.05)   # the plain band
        self.assertGreater(abs(strip_area(raw) - strip_area(aligned)), 0.5)       # unaligned: a different, twisted band
        twisted = run("bridge", g, twist=1)
        self.assertGreater(abs(strip_area(twisted) - strip_area(aligned)), 0.05)

    def test_loops_of_different_sizes_get_triangles_and_stay_manifold(self):
        out = run("bridge", pipes(16, 24))
        sizes = out.topology.prim_sizes()
        self.assertGreater(int((sizes == 3).sum()), 0)
        self.assertEqual(int((twins(out) < 0).sum()), 16 + 24)                # the two far rims

    def test_closing_a_ring_gives_a_closed_torus_like_shape(self):
        a = run("tube", columns=12, rows=1, height=1.0, caps=False)
        b = run("transform", a, translate=[0, 0, 3.0])
        out = run("bridge", run("merge", [a, b]))
        capped = run("polyfill", out)
        self.assertTrue(np.all(twins(capped) >= 0))
        self.assertGreater(volume(capped), 0)

    def test_pairing_several_loops_and_errors(self):
        far = run("transform", pipes(), translate=[10, 0, 0])
        four = run("merge", [pipes(), far])
        out = run("bridge", four)
        self.assertEqual(out.nprims, four.nprims + 2 * 16)                    # each facing pair of ends is bridged
        self.assertEqual(int((twins(out) < 0).sum()), 4 * 16)
        inorder = run("bridge", four, pairing="in_order")                     # (loop 0, loop 1), (2, 3)...: each pipe on itself
        self.assertEqual(inorder.nprims, four.nprims + 4 * 16)
        with self.assertRaises(Exception):
            run("bridge", run("tube", caps=True))                             # no open loops at all
        one = run("bridge", run("tube", caps=False))                          # one piece: its own two ends get joined
        self.assertEqual(one.nprims, 24 + 24)


if __name__ == "__main__":
    unittest.main()
