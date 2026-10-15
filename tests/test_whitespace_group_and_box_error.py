"""Empty group expressions and box size errors behave the same across paths."""
import unittest

import numpy as np

import minihou  # noqa: F401 - registers the built-in nodes
from minihou import Geometry, rust
from minihou.nodes.primitives import box_node
from minihou.sdk import REGISTRY


class WhitespaceGroupAndBoxErrorTests(unittest.TestCase):
    def assert_same_geometry(self, a, b):
        self.assertEqual((a.npoints, a.nvertices, a.nprims),
                         (b.npoints, b.nvertices, b.nprims))
        for field in ("vertex_point", "prim_offsets"):
            np.testing.assert_array_equal(getattr(a.topology, field), getattr(b.topology, field))
        np.testing.assert_array_equal(a.topology.closed_mask(), b.topology.closed_mask())
        for domain in ("point", "vertex", "prim"):
            left, right = a.table(domain), b.table(domain)
            self.assertEqual(set(left.names()), set(right.names()))
            for name in left.names():
                np.testing.assert_array_equal(left[name], right[name])
        self.assertEqual(a.detail, b.detail)

    def test_whitespace_group_matches_empty_group(self):
        with rust.disabled():
            unit_box = box_node()
        empty = Geometry.from_arrays(np.empty((0, 3), dtype=np.float32))

        for source_name, source in (("unit box", unit_box), ("empty", empty)):
            for node_name in ("attribute_blur", "smooth", "attribute_create"):
                with self.subTest(source=source_name, node=node_name):
                    node = REGISTRY.get(node_name)
                    params = {"value": (1.0, 2.0, 3.0)} if node_name == "attribute_create" else {}
                    blank = node.fn(source, **node.coerce_params({**params, "group": ""}))
                    whitespace = node.fn(source, **node.coerce_params({**params, "group": " "}))
                    self.assert_same_geometry(blank, whitespace)

    def test_box_over_limit_python_error(self):
        with rust.disabled(), self.assertRaises(ValueError) as error:
            box_node(divisions=1000)
        self.assertEqual(str(error.exception), "Box exceeds one million face cells")

    @unittest.skipUnless(rust.enabled(), "the Rust core is not enabled")
    def test_box_over_limit_rust_error_matches_python(self):
        with rust.disabled(), self.assertRaises(ValueError) as python_error:
            box_node(divisions=1000)
        with self.assertRaises(ValueError) as rust_error:
            box_node(divisions=1000)
        self.assertEqual(str(rust_error.exception), str(python_error.exception))


if __name__ == "__main__":
    unittest.main()
