"""dissolve: neighbouring flat faces become n-gons; surfaces stay closed and keep their area."""
import unittest

import numpy as np

from minihou.nodes.bevel import twins
from minihou.nodes.normal import prim_normals
from tests.test_nodes import run


def area(geo):
    """Total polygon area (fan from the first vertex of each face; exact for planar faces)."""
    P = geo.P.astype(np.float64)
    vp, offs = geo.topology.vertex_point.astype(np.int64), geo.topology.prim_offsets.astype(np.int64)
    total = 0.0
    for p in range(geo.nprims):
        v = P[vp[offs[p]:offs[p + 1]]]
        c = np.cross(v[1:-1] - v[0], v[2:] - v[0]).sum(0)
        total += 0.5 * np.linalg.norm(c)
    return total


class DissolveTests(unittest.TestCase):
    def test_triangulated_box_becomes_six_quads(self):
        box = run("box", size=[1, 2, 3])
        tri = run("divide", box)
        self.assertEqual(tri.nprims, 12)
        out = run("dissolve", tri)
        self.assertEqual(out.nprims, 6)
        self.assertEqual(set(out.topology.prim_sizes().tolist()), {4})
        self.assertTrue(np.all(twins(out) >= 0))
        self.assertAlmostEqual(area(out), area(box), places=5)

    def test_subdivided_faces_merge_back(self):
        box = run("box", size=[1, 1, 1])
        fine = run("subdivide", box, depth=1, algorithm="linear")
        self.assertGreater(fine.nprims, 6)
        out = run("dissolve", fine)
        self.assertEqual(out.nprims, 6)
        self.assertTrue(np.all(twins(out) >= 0))
        self.assertAlmostEqual(area(out), 6.0, places=5)

    def test_group_limits_the_merge_and_curved_surfaces_stay_in_patches(self):
        tri = run("divide", run("box", size=[1, 1, 1]))
        part = run("dissolve", tri, group="0-5")
        self.assertGreater(part.nprims, 6)
        self.assertLess(part.nprims, 12)
        s = run("divide", run("sphere", rows=40, columns=64))
        out = run("dissolve", s, angle=6.0)
        self.assertLess(out.nprims, s.nprims)
        self.assertGreater(out.nprims, 100)                                  # patches of a dozen faces, not one slab
        self.assertTrue(np.all(twins(out) >= 0))
        self.assertAlmostEqual(area(out), area(s), delta=0.05 * area(s))     # curved patches flatten a little
        self.assertEqual(run("dissolve", tri, angle=0.0).nprims, 6)        # exactly flat pairs still merge

    def test_attributes_follow_the_faces(self):
        box = run("box", size=[1, 1, 1])
        box = box.copy()
        box.prim["id"] = np.arange(box.nprims, dtype=np.int32) * 10
        tri = run("divide", box)
        out = run("dissolve", tri)
        self.assertEqual(sorted(out.prim["id"].tolist()), [0, 10, 20, 30, 40, 50])
        for key in ("uv", "N"):
            if key in out.vertex:
                self.assertEqual(len(out.vertex[key]), out.topology.nvertices)

    def test_a_flat_grid_with_a_hole_keeps_its_area(self):
        g = run("divide", run("grid", columns=7, rows=7, size_x=6.0, size_y=6.0))
        centre = run("blast", g, group="34-37")                           # a few triangles missing in the middle
        out = run("dissolve", centre)
        self.assertLess(out.nprims, centre.nprims)
        self.assertAlmostEqual(area(out), area(centre), places=4)
        self.assertAlmostEqual(area(out), 36.0 * (1 - 4 / 72), places=4)

    def test_open_curves_are_refused(self):
        with self.assertRaises(Exception):
            run("dissolve", run("line", points=5))


if __name__ == "__main__":
    unittest.main()
