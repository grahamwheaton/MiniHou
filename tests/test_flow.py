"""Attribute flow: what a node did to the attributes of its inputs."""
import unittest

import numpy as np

from minihou import Geometry
from minihou.flow import attribute_flow, summary_line
from minihou.link.session import Session
from tests.helpers import graph_from
from tests.test_nodes import run


def names(items):
    return sorted((a["domain"], a["name"]) for a in items)


class FlowTests(unittest.TestCase):
    def test_added_modified_removed_and_unchanged(self):
        g = run("grid", columns=4, rows=4)
        w = run("wrangle", g, snippet="v@Cd = set(@P.x, 0, 0); f@h = @P.z; @P.z += 1", run_over="points")
        f = attribute_flow(w, [g])
        self.assertEqual(names(f["added"]), [("point", "Cd"), ("point", "h")])
        self.assertEqual(names(f["modified"]), [("point", "P")])
        self.assertEqual((f["removed"], f["moved"], f["topology"]), ([], [], "same"))
        self.assertGreaterEqual(f["unchanged"], 1)                              # uv passed through
        gone = run("attribute_delete", w, names="h Cd")
        f2 = attribute_flow(gone, [w])
        self.assertEqual(names(f2["removed"]), [("point", "Cd"), ("point", "h")])
        self.assertEqual(summary_line(f), "+Cd +h ~P")

    def test_a_rewritten_but_identical_array_is_not_a_change(self):
        g = run("grid", columns=3, rows=3)
        same = g.copy()
        same.point["P"] = g.P.copy()                                            # a new array with the same values
        self.assertEqual(attribute_flow(same, [g])["modified"], [])

    def test_moving_an_attribute_between_domains(self):
        g = run("grid", columns=3, rows=3).copy()
        g.point["w"] = np.arange(g.npoints, dtype=np.float32)
        moved = run("wrangle", g, snippet="f@w = @w", run_over="primitives")
        f = attribute_flow(moved, [g])
        self.assertEqual([(a["name"], a["from"], a["domain"]) for a in f["moved"]], [("w", "point", "prim")])
        self.assertEqual(f["added"], [])

    def test_detail_attributes_and_topology(self):
        g = run("grid", columns=3, rows=3)
        f = attribute_flow(g, [])
        self.assertEqual(f["topology"], "new")
        self.assertIn(("detail", "grid_columns"), [(a["domain"], a["name"]) for a in f["added"]])
        sub = run("subdivide", g, depth=1)
        f = attribute_flow(sub, [g])
        self.assertEqual(f["topology"], "changed")
        self.assertEqual(f["counts"]["prims"], [g.nprims, sub.nprims])
        a = attribute_flow(run("merge", [g, g]), [g, g])
        self.assertEqual(a["counts"]["points"][0], g.npoints)
        e = attribute_flow(Geometry.points_only(np.zeros((2, 3), np.float32)), [None])
        self.assertEqual(e["inputs"], 0)


class InspectFlowTests(unittest.TestCase):
    def test_inspect_reports_flow_and_what_a_wrangle_reads(self):
        s = Session(graph_from({
            "g": {"type": "grid", "params": {"columns": 5, "rows": 5}},
            "w": {"type": "wrangle", "inputs": {"geo": "g"}, "params": {"snippet": "f@h = @P.z + @uv.x; @P.z = 1"}},
            "o": {"type": "output", "inputs": {"geo": "w"}}}, display="o"))
        r = s.inspect("w")
        self.assertTrue(r["ok"])
        self.assertEqual(names(r["flow"]["added"]), [("point", "h")])
        self.assertEqual(names(r["flow"]["modified"]), [("point", "P")])
        self.assertEqual(r["wrangle"], {"reads": ["P", "uv"], "writes": ["P", "h"]})
        self.assertEqual(r["cooked"] + r["cached"], r["cooked"] + r["cached"])
        src = s.inspect("g")
        self.assertEqual(src["flow"]["inputs"], 0)
        self.assertFalse(s.inspect("nope")["ok"])
        out = s.inspect("o")                                                     # an output node passes everything through
        self.assertEqual((out["flow"]["added"], out["flow"]["modified"], out["flow"]["removed"]), ([], [], []))


if __name__ == "__main__":
    unittest.main()
