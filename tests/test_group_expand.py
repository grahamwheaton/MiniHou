"""group_expand: grow and shrink groups by rings of neighbours."""
import unittest

import numpy as np

from minihou import groups
from tests.test_nodes import run


def grid_with(prims):
    g = run("grid", columns=7, rows=7)                                   # 6 x 6 quads
    return run("group_create", g, name="sel", domain="primitives", method="pattern", pattern=prims)


def members(geo, domain, name):
    return np.flatnonzero(groups.member(geo, domain, name))


class GroupExpandTests(unittest.TestCase):
    def test_expanding_one_quad_by_points_and_by_edges(self):
        g = grid_with("14")                                              # row 2, column 2: an interior quad
        by_points = run("group_expand", g, group="sel", name="out", steps=1, connectivity="points")
        self.assertEqual(len(members(by_points, "prim", "out")), 9)      # 3 x 3 block
        by_edges = run("group_expand", g, group="sel", name="out", steps=1, connectivity="edges")
        self.assertEqual(sorted(members(by_edges, "prim", "out").tolist()), [8, 13, 14, 15, 20])   # a plus sign
        two = run("group_expand", g, group="sel", name="out", steps=2, connectivity="points")
        self.assertEqual(len(members(two, "prim", "out")), 25)

    def test_contract_undoes_expand_on_open_surfaces(self):
        g = grid_with("14")
        big = run("group_expand", g, group="sel", name="big", steps=1)
        back = run("group_expand", big, group="big", name="back", mode="contract", steps=1)
        self.assertEqual(members(back, "prim", "back").tolist(), [14])
        wide = run("group_expand", g, group="sel", name="wide", steps=2)         # reaches the open border...
        part = run("group_expand", wide, group="wide", name="part", mode="contract", steps=2)
        self.assertEqual(members(part, "prim", "part").tolist(), [0, 1, 2, 6, 7, 8, 12, 13, 14])   # ...which never shrinks
        gone = run("group_expand", g, group="sel", name="x", mode="contract")
        self.assertEqual(len(members(gone, "prim", "x")), 0)             # a lone quad touches non-members everywhere

    def test_the_whole_mesh_never_shrinks_from_its_borders(self):
        g = run("grid", columns=5, rows=5)
        g = run("group_create", g, name="all", domain="primitives", method="pattern", pattern="*")
        c = run("group_expand", g, group="all", name="all", mode="contract", steps=3)
        self.assertEqual(len(members(c, "prim", "all")), g.nprims)       # the open boundary is not a non-member

    def test_points_domain_and_replacing_the_group_in_place(self):
        g = run("grid", columns=6, rows=6)
        g = run("group_create", g, name="p", domain="points", method="pattern", pattern="14")
        e = run("group_expand", g, group="p", domain="points", steps=1)  # no name: the group is replaced
        self.assertEqual(len(members(e, "point", "p")), 5)               # the point and its four edge neighbours
        e2 = run("group_expand", g, group="p", domain="points", steps=2)
        self.assertEqual(len(members(e2, "point", "p")), 13)
        self.assertEqual(len(members(g, "point", "p")), 1)               # the input is untouched

    def test_ring_selection_with_subtract_and_pattern_names(self):
        g = grid_with("14")
        e = run("group_expand", g, group="sel", name="grown")
        ring = run("group_create", e, name="ring", domain="primitives", method="pattern", pattern="grown ^sel")
        self.assertEqual(len(members(ring, "prim", "ring")), 8)
        with self.assertRaises(Exception):
            run("group_expand", g, group="14")                           # a pattern needs an explicit result name
        with self.assertRaises(Exception):
            run("group_expand", g, group="nothing_here", name="x")


if __name__ == "__main__":
    unittest.main()
