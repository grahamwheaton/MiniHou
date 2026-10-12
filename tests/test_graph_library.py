"""The graph / library browser's engine side: listing, Save As, file operations, and saving a selection as a library node."""
import http.client
import json
import os
import shutil
import tempfile
import unittest

import numpy as np

from minihou import Evaluator, Graph, subgraph
from minihou.graph import GraphError
from minihou.library import NodeLibrary
from minihou.link.session import Session
from minihou.web import WebServer

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

GRAPH = {"version": 1, "nodes": {
    "b": {"type": "box", "params": {"size": [2, 2, 2]}, "pos": [0, 0]},
    "t": {"type": "transform", "inputs": {"geo": "b"}, "params": {"translate": [0, 0, 1]}, "pos": [0, 100]},
    "n": {"type": "noise", "inputs": {"geo": "t"}, "params": {"frequency": 2.0}, "pos": [0, 200]},
    "c": {"type": "sphere", "pos": [200, 0]},
    "m": {"type": "merge", "inputs": {"geos": ["n", "c"]}, "pos": [100, 300]},
    "o": {"type": "output", "inputs": {"geo": "m"}, "pos": [100, 400]}},
    "display": "o", "exposed": [{"name": "Noise Frequency", "node": "n", "param": "frequency"}]}


class SubgraphTests(unittest.TestCase):
    def graph(self):
        return Graph.from_dict(GRAPH)

    def test_extract_makes_inputs_output_and_carries_exposed_values(self):
        data, ports = subgraph.extract(self.graph(), ["t", "n"], "shift and roughen")
        self.assertEqual(ports, [("b", "b")])
        self.assertEqual(data["nodes"]["in_b"], {"type": "input", "params": {"name": "b", "index": 0}, "pos": [0.0, 0.0]})
        self.assertEqual(data["nodes"]["t"]["inputs"], {"geo": "in_b"})
        self.assertEqual(data["nodes"]["result"]["inputs"], {"geo": "n"})
        self.assertEqual(data["display"], "result")
        self.assertEqual(data["exposed"][0]["name"], "Noise Frequency")
        self.assertEqual(data["doc"], "shift and roughen")

    def test_several_inputs_and_unique_port_names(self):
        g = Graph.from_dict({"nodes": {"a": {"type": "box"}, "b": {"type": "box"}, "m": {"type": "merge", "inputs": {"geos": ["a", "b"]}},
                                        "o": {"type": "output", "inputs": {"geo": "m"}}}})
        data, ports = subgraph.extract(g, ["m"])
        self.assertEqual(ports, [("a", "a"), ("b", "b")])
        self.assertEqual(data["nodes"]["m"]["inputs"]["geos"], ["in_a", "in_b"])
        self.assertEqual([data["nodes"]["in_a"]["params"]["index"], data["nodes"]["in_b"]["params"]["index"]], [0, 1])

    def test_the_result_must_be_one_node_and_some_nodes_cannot_go_in(self):
        g = self.graph()
        with self.assertRaises(GraphError) as e:
            subgraph.extract(g, ["b", "c"])                       # both feed the rest of the graph through different nodes
        self.assertIn("one output", str(e.exception))
        with self.assertRaises(GraphError):
            subgraph.extract(g, ["o"])
        with self.assertRaises(GraphError):
            subgraph.extract(g, [])
        g2 = Graph.from_dict({"nodes": {"b": {"type": "box"}, "p": {"type": "rbd_solver", "inputs": {"bodies": "b"}}}})
        with self.assertRaises(GraphError):
            subgraph.extract(g2, ["p"])

    def test_replace_ops_wire_the_new_node_into_the_same_slots(self):
        g = self.graph()
        ops = subgraph.replace_ops(g, ["t", "n"], "shifted")
        self.assertEqual(ops[0], {"op": "add", "type": "shifted", "id": "shifted", "pos": [0.0, 150.0], "inputs": {"b": "b"}})
        self.assertEqual(ops[1], {"op": "disconnect", "to": "m", "port": "geos", "from": "n"})
        self.assertEqual(ops[2], {"op": "connect", "from": "shifted", "to": "m", "port": "geos", "index": 0})      # the same place in the merge
        self.assertEqual(ops[-1], {"op": "remove", "id": ["t", "n"]})
        g2 = Graph.from_dict({"nodes": {"b": {"type": "box"}, "t": {"type": "transform", "inputs": {"geo": "b"}}}, "display": "t"})
        self.assertTrue(subgraph.replace_ops(g2, ["t"], "mover")[0]["display"])                                  # the display flag moves too

    def test_a_loop_must_be_selected_whole(self):
        g = Graph.from_dict({"nodes": {"b": {"type": "box"}, "l": {"type": "loop_begin", "inputs": {"geo": "b"}},
                                        "t": {"type": "transform", "inputs": {"geo": "l"}},
                                        "e": {"type": "loop_end", "inputs": {"geo": "t", "begin": "l"}}}})
        with self.assertRaises(GraphError):
            subgraph.extract(g, ["l", "t"])
        with self.assertRaises(GraphError):
            subgraph.extract(g, ["t", "e"])
        data, ports = subgraph.extract(g, ["l", "t", "e"])
        self.assertEqual(ports, [("b", "b")])


class ApiTests(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.mkdtemp(prefix="minihou_lib_")
        self.addCleanup(shutil.rmtree, self.dir, True)
        self.path = os.path.join(self.dir, "main.json")
        with open(self.path, "w", encoding="utf-8") as f:
            json.dump(GRAPH, f)
        os.makedirs(os.path.join(self.dir, "graphs"))
        shutil.copyfile(self.path, os.path.join(self.dir, "graphs", "other.json"))
        self.s = Session(Graph.load(self.path, check=False))
        self.s.start()
        self.s.watch_file(self.path, interval=0.05)
        self.lib = NodeLibrary([os.path.join(self.dir, "nodes")])
        self.addCleanup(self.lib.unload)
        self.s.attach_library(self.lib, interval=0.05)
        self.web = WebServer(self.s, "127.0.0.1", 0, root=self.dir)
        self.port = self.web.start()
        self.addCleanup(self.s.stop)
        self.addCleanup(self.web.stop)

    def req(self, method, path, body=None):
        c = http.client.HTTPConnection("127.0.0.1", self.port, timeout=10)
        c.request(method, path, body=json.dumps(body) if body is not None else None,
                  headers={"Content-Type": "application/json"} if body is not None else {})
        r = c.getresponse()
        data = r.read()
        c.close()
        try:
            return r.status, json.loads(data)
        except ValueError:
            return r.status, data

    def test_listing_has_names_ages_counts_and_the_current_file(self):
        st, d = self.req("GET", "/api/library")
        self.assertEqual(st, 200)
        by = {f["path"]: f for f in d["files"]}
        self.assertEqual(sorted(by), ["graphs/other.json", "main.json"])
        self.assertTrue(by["main.json"]["current"] and not by["graphs/other.json"]["current"])
        self.assertEqual(by["main.json"]["nodes"], 6)
        self.assertEqual(by["main.json"]["exposed"], 1)
        self.assertFalse(by["main.json"]["system"])
        self.assertEqual(os.path.basename(d["library_dir"]), "nodes")

    def test_read_a_graph_file_and_stay_inside_the_root(self):
        st, d = self.req("GET", "/api/file?path=graphs/other.json")
        self.assertEqual(st, 200)
        self.assertIn("noise", [n["type"] for n in d["graph"]["nodes"].values()])
        self.assertEqual(self.req("GET", "/api/file?path=../escape.json")[0], 403)
        self.assertEqual(self.req("GET", "/api/file?path=missing.json")[0], 404)
        self.assertEqual(self.req("GET", "/api/file?path=graphs/other.txt")[0], 400)

    def test_save_as_copies_switches_and_refuses_to_overwrite(self):
        st, d = self.req("POST", "/api/save_as", {"path": "copies/mine.json"})
        self.assertEqual(st, 200)
        self.assertEqual(os.path.basename(str(self.s.path)), "mine.json")
        self.assertEqual(Graph.load(os.path.join(self.dir, "copies", "mine.json"), check=False).to_dict()["nodes"].keys(), Graph.from_dict(GRAPH).to_dict()["nodes"].keys())
        st, d = self.req("POST", "/api/save_as", {"path": "main.json"})                  # exists
        self.assertEqual(st, 409)
        st, d = self.req("POST", "/api/save_as", {"path": "copies/mine.json"})           # the file that is open
        self.assertEqual(st, 409)
        self.assertEqual(self.req("POST", "/api/save_as", {"path": "main.json", "overwrite": True})[0], 200)
        self.assertEqual(self.req("POST", "/api/save_as", {"path": "../x.json"})[0], 403)

    def test_file_operations(self):
        st, d = self.req("POST", "/api/file_op", {"op": "duplicate", "path": "graphs/other.json"})
        self.assertEqual((st, d["path"]), (200, "graphs/other_copy.json"))
        st, d = self.req("POST", "/api/file_op", {"op": "duplicate", "path": "graphs/other.json"})
        self.assertEqual(d["path"], "graphs/other_copy2.json")
        st, d = self.req("POST", "/api/file_op", {"op": "rename", "path": "graphs/other_copy2.json", "to": "graphs/renamed.json"})
        self.assertEqual((st, d["path"]), (200, "graphs/renamed.json"))
        self.assertEqual(self.req("POST", "/api/file_op", {"op": "rename", "path": "graphs/renamed.json", "to": "graphs/other.json"})[0], 409)
        self.assertEqual(self.req("POST", "/api/file_op", {"op": "delete", "path": "main.json"})[0], 409)       # the open graph
        st, d = self.req("POST", "/api/file_op", {"op": "delete", "path": "graphs/renamed.json"})
        self.assertEqual(st, 200)
        self.assertFalse(os.path.exists(os.path.join(self.dir, "graphs", "renamed.json")))
        self.assertEqual(self.req("POST", "/api/file_op", {"op": "frobnicate", "path": "graphs/other.json"})[0], 400)
        self.assertEqual(self.req("POST", "/api/file_op", {"op": "delete", "path": "nope.json"})[0], 404)

    def test_make_a_library_node_from_the_selection_and_replace_it(self):
        st, d = self.req("POST", "/api/make_system", {"name": "shifted", "nodes": ["t", "n"], "replace": True, "doc": "up and rough"})
        self.assertEqual(st, 200, d)
        self.assertTrue(d["replaced"])
        self.assertTrue(os.path.isfile(os.path.join(self.dir, "nodes", "shifted.json")))
        reg = self.s.evaluator.registry
        self.assertIn("shifted", reg)
        self.assertEqual(reg.get("shifted").category, "system")
        self.assertEqual([i.name for i in reg.get("shifted").inputs], ["b"])
        self.assertEqual([p.name for p in reg.get("shifted").params], ["noise_frequency"])            # the exposed value became a parameter
        self.assertEqual(reg.get("shifted").doc, "up and rough")
        g = self.s.graph
        self.assertEqual(sorted(g.nodes), ["b", "c", "m", "o", "shifted"])
        self.assertEqual(g.nodes["m"].inputs["geos"], ["shifted", "c"])
        # the result is the same geometry as before
        before = Evaluator().cook(Graph.from_dict(GRAPH))
        after = Evaluator(reg).cook(g.copy())
        np.testing.assert_allclose(after.P, before.P, atol=1e-6)
        # one undo puts the nodes back
        self.assertTrue(self.s.undo())
        self.assertEqual(sorted(self.s.graph.nodes), sorted(GRAPH["nodes"]))
        st, listing = self.req("GET", "/api/library")
        sysrow = next(f for f in listing["files"] if f["name"] == "shifted")
        self.assertTrue(sysrow["system"] and sysrow["type"] == "shifted" and sysrow["doc"] == "up and rough")

    def test_make_without_replacing_and_the_checks(self):
        st, d = self.req("POST", "/api/make_system", {"name": "keep_it", "nodes": ["t"]})
        self.assertEqual(st, 200)
        self.assertNotIn("replaced", d)
        self.assertEqual(sorted(self.s.graph.nodes), sorted(GRAPH["nodes"]))                      # the graph is untouched
        self.assertEqual(self.req("POST", "/api/make_system", {"name": "keep_it", "nodes": ["t"]})[0], 409)      # already there
        self.assertEqual(self.req("POST", "/api/make_system", {"name": "keep_it", "nodes": ["n"], "overwrite": True})[0], 200)
        self.assertEqual(self.req("POST", "/api/make_system", {"name": "box", "nodes": ["t"]})[0], 409)          # a built-in node's name
        self.assertEqual(self.req("POST", "/api/make_system", {"name": "bad name", "nodes": ["t"]})[0], 400)
        self.assertEqual(self.req("POST", "/api/make_system", {"name": "x", "nodes": []})[0], 400)
        st, d = self.req("POST", "/api/make_system", {"name": "two_ends", "nodes": ["b", "c"]})
        self.assertEqual(st, 400)
        self.assertIn("one output", d["error"])
        self.assertFalse(os.path.exists(os.path.join(self.dir, "nodes", "two_ends.json")))

    def test_a_system_the_engine_refuses_is_rolled_back(self):
        # a node that cannot be inside a system slips past the pre-checks when its type is registered later: simulate the
        # engine refusing by making the target name a directory entry the library cannot read as a system
        from unittest import mock
        with mock.patch.object(Session, "_poll_library", lambda self: None):                        # the library never loads it
            st, d = self.req("POST", "/api/make_system", {"name": "ghost", "nodes": ["t"], "replace": True})
        self.assertEqual(st, 400)
        self.assertFalse(os.path.exists(os.path.join(self.dir, "nodes", "ghost.json")))
        self.assertEqual(sorted(self.s.graph.nodes), sorted(GRAPH["nodes"]))                       # and the graph was not touched


if __name__ == "__main__":
    unittest.main()
