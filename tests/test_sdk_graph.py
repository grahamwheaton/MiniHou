import json
import unittest

import numpy as np

from minihou import Geometry, Graph, GraphError, ParamError, REGISTRY
from minihou.sdk import Bool, Enum, Float, Int, Registry, Str, Vec3, node
from tests.helpers import graph_from


class SdkTests(unittest.TestCase):
    def test_builtin_nodes_are_registered(self):
        for n in ("grid", "transform", "noise", "scatter", "copy_to_points", "merge", "output"):
            self.assertIn(n, REGISTRY)

    def test_decorator_registers_and_keeps_function(self):
        reg = Registry()

        @node("double", inputs=["geo"], params=[Float("k", 2.0, min=0)], registry=reg)
        def double(geo, k):
            return geo

        self.assertIs(reg.get("double").fn, double)
        self.assertEqual(reg.get("double").default_params(), {"k": 2.0})

    def test_signature_mismatch_fails_at_decoration_time(self):
        reg = Registry()
        with self.assertRaises(ValueError):
            @node("bad", inputs=["geo"], params=[Float("k", 1.0)], registry=reg)
            def bad(geo):          # missing 'k'
                return geo

    def test_duplicate_names_rejected(self):
        reg = Registry()
        node("a", registry=reg)(lambda: None)
        with self.assertRaises(ValueError):
            node("a", registry=reg)(lambda: None)
        with self.assertRaises(ValueError):
            node("b", inputs=["x"], params=[Float("x")], registry=reg)(lambda x: None)

    def test_param_coercion(self):
        self.assertEqual(Float("f", 1).coerce(3), 3.0)
        self.assertEqual(Int("i", 1).coerce(3.0), 3)
        self.assertEqual(Vec3("v").coerce([1, 2, 3]), (1.0, 2.0, 3.0))
        self.assertEqual(Enum("e", ["a", "b"]).coerce("b"), "b")
        self.assertIs(Bool("b").coerce(1), True)
        for bad in (Int("i").coerce, Float("f").coerce, Vec3("v").coerce, Enum("e", ["a"]).coerce, Bool("b").coerce):
            with self.assertRaises(ParamError):
                bad("nope" if bad is not Vec3("v").coerce else [1, 2])
        with self.assertRaises(ParamError):
            Int("i").coerce(1.5)
        with self.assertRaises(ParamError):
            Float("f").coerce(float("nan"))
        with self.assertRaises(ParamError):
            Float("f").coerce(True)

    def test_param_ranges_are_hard_limits(self):
        p = Float("f", 1.0, min=0.0, max=2.0)
        with self.assertRaises(ParamError):
            p.coerce(-0.1)
        with self.assertRaises(ParamError):
            p.coerce(2.1)
        with self.assertRaises(ValueError):
            Int("i", 0, min=1)         # default outside range

    def test_coerce_params_fills_defaults_and_rejects_unknown(self):
        nt = REGISTRY.get("grid")
        full = nt.coerce_params({"rows": 5})
        self.assertEqual(full["rows"], 5)
        self.assertEqual(full["columns"], 10)
        with self.assertRaises(ParamError):
            nt.coerce_params({"nope": 1})


class GraphTests(unittest.TestCase):
    def simple(self):
        return {
            "g": {"type": "grid", "params": {"columns": 4, "rows": 4}},
            "n": {"type": "noise", "inputs": {"geo": "g"}},
            "o": {"type": "output", "inputs": {"geo": "n"}},
        }

    def test_json_roundtrip(self):
        g = graph_from(self.simple(), display="o", exposed=[{"name": "Amp", "node": "n", "param": "amplitude"}])
        text = g.to_json()
        g2 = Graph.from_json(text)
        self.assertEqual(g.to_dict(), g2.to_dict())
        self.assertEqual(json.loads(text)["nodes"]["g"]["params"], {"columns": 4, "rows": 4})

    def test_save_and_load_file(self):
        import tempfile, os
        g = graph_from(self.simple(), display="o")
        with tempfile.TemporaryDirectory() as d:
            p = os.path.join(d, "g.json")
            g.save(p)
            self.assertEqual(Graph.load(p).to_dict(), g.to_dict())

    def test_editor_metadata_roundtrips(self):
        n = self.simple()
        n["g"]["pos"] = [10, 20]
        self.assertEqual(graph_from(n).to_dict()["nodes"]["g"]["pos"], [10, 20])

    def test_errors_are_collected(self):
        with self.assertRaises(GraphError) as cm:
            graph_from({"a": {"type": "nope"}, "b": {"type": "noise", "inputs": {"geo": "missing"}},
                        "c": {"type": "grid", "params": {"rows": 1}}})
        msg = str(cm.exception)
        self.assertIn("unknown node type 'nope'", msg)
        self.assertIn("rows", msg)

    def test_missing_reference_and_unconnected_input(self):
        with self.assertRaises(GraphError) as cm:
            graph_from({"n": {"type": "noise", "inputs": {"geo": "ghost"}}})
        self.assertIn("missing node 'ghost'", str(cm.exception))
        with self.assertRaises(GraphError) as cm:
            graph_from({"n": {"type": "noise"}})
        self.assertIn("not connected", str(cm.exception))

    def test_cycle_detected(self):
        with self.assertRaises(GraphError) as cm:
            graph_from({"a": {"type": "noise", "inputs": {"geo": "b"}}, "b": {"type": "noise", "inputs": {"geo": "a"}}})
        self.assertIn("cycle", str(cm.exception))

    def test_bad_exposed(self):
        n = self.simple()
        with self.assertRaises(GraphError):
            graph_from(n, exposed=[{"name": "x", "node": "n", "param": "attrib"}])     # str not exposable
        with self.assertRaises(GraphError):
            graph_from(n, exposed=[{"name": "x", "node": "zzz", "param": "seed"}])
        with self.assertRaises(GraphError):
            graph_from(n, exposed=[{"name": "x", "node": "n", "param": "seed"}, {"name": "x", "node": "n", "param": "gain"}])

    def test_invalid_json_and_version(self):
        with self.assertRaises(GraphError):
            Graph.from_json("{nope")
        with self.assertRaises(GraphError):
            Graph.from_dict({"version": 99, "nodes": {}})

    def test_upstream_order_is_dependencies_first_and_minimal(self):
        g = graph_from({
            "a": {"type": "grid"}, "b": {"type": "grid"},
            "t": {"type": "transform", "inputs": {"geo": "a"}},
            "m": {"type": "merge", "inputs": {"geos": ["t", "a"]}},
            "unused": {"type": "noise", "inputs": {"geo": "b"}},
            "o": {"type": "output", "inputs": {"geo": "m"}},
        })
        order = g.upstream_order("o")
        self.assertNotIn("b", order)
        self.assertNotIn("unused", order)
        self.assertEqual(order[-1], "o")
        self.assertEqual(sorted(order), ["a", "m", "o", "t"])
        self.assertLess(order.index("a"), order.index("t"))
        self.assertLess(order.index("t"), order.index("m"))
        self.assertEqual(set(g.downstream("a")), {"t", "m", "o"})

    def test_deep_chain_does_not_hit_recursion_limit(self):
        nodes = {"n0": {"type": "grid", "params": {"columns": 2, "rows": 2}}}
        for i in range(1, 3000):
            nodes[f"n{i}"] = {"type": "transform", "inputs": {"geo": f"n{i - 1}"}}
        g = graph_from(nodes)
        self.assertEqual(len(g.upstream_order("n2999")), 3000)

    def test_exposed_interface_and_set_exposed(self):
        g = graph_from(self.simple(), display="o", exposed=[
            {"name": "Height", "node": "n", "param": "amplitude"},
            {"name": "Octaves", "node": "n", "param": "octaves", "label": "Detail"}])
        iface = g.interface()
        self.assertEqual([p["name"] for p in iface], ["Height", "Octaves"])
        self.assertEqual(iface[0]["kind"], "vec3")
        self.assertEqual(iface[1]["min"], 1)
        self.assertEqual(iface[1]["label"], "Detail")
        self.assertEqual(iface[0]["value"], [0.0, 0.0, 1.0])
        g.set_exposed("Octaves", 4)
        self.assertEqual(g.interface()[1]["value"], 4)
        with self.assertRaises(ParamError):
            g.set_exposed("Octaves", 99)

    def test_edit_api(self):
        g = Graph()
        g.add_node("g", "grid", {"rows": 3})
        g.add_node("o", "output", inputs={"geo": "g"})
        self.assertEqual(g.output_node(), "o")
        self.assertEqual(g.get_param("g", "rows"), 3)
        self.assertEqual(g.get_param("g", "columns"), 10)
        with self.assertRaises(GraphError):
            g.add_node("g", "grid")
        with self.assertRaises(GraphError):
            g.connect("o", "nope", "g")
        g.remove_node("g")
        self.assertIn("not connected", "; ".join(g.validate()))


if __name__ == "__main__":
    unittest.main()
