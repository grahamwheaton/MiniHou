"""Graph edit ops (minihou.edit) and the Session's editing, undo, inspection and file round-trip."""
import json
import os
import tempfile
import time
import unittest

from minihou import Graph
from minihou.edit import EditError, apply_ops, layout_missing
from minihou.link.session import Session
from tests.helpers import graph_from

def _load(path):
    with open(path, encoding="utf-8") as f:
        return json.load(f)


BASE = {"g": {"type": "grid", "params": {"columns": 4, "rows": 4}},
        "n": {"type": "noise", "inputs": {"geo": "g"}},
        "o": {"type": "output", "inputs": {"geo": "n"}}}


def base():
    return graph_from(BASE, display="o", exposed=[{"name": "Amp", "node": "n", "param": "amplitude"}])


class OpsTests(unittest.TestCase):
    def test_frame_annotations_roundtrip_and_undo(self):
        session = Session(base())
        session.edit([{"op": "box_add", "id": "note", "pos": [10, 20], "size": [200, 100],
                       "note": True, "text": "<script>plain text</script>", "locked": True}])
        session.edit([{"op": "box_set", "id": "note", "collapsed": True, "text": "Updated"}])
        restored = Graph.from_dict(session.graph.to_dict())
        self.assertTrue(restored.boxes[0]["collapsed"])
        self.assertTrue(restored.boxes[0]["locked"])
        self.assertEqual(restored.boxes[0]["text"], "Updated")
        session.undo()
        self.assertEqual(session.graph.boxes[0]["text"], "<script>plain text</script>")
        self.assertNotIn("collapsed", session.graph.boxes[0])

    def test_frame_annotations_reject_invalid_values(self):
        for extra in ({"locked": "yes"}, {"collapsed": 1}, {"note": []}, {"text": 42}, {"text": "x" * 10001}):
            with self.subTest(extra=list(extra)):
                with self.assertRaises(EditError):
                    apply_ops(base(), [{"op": "box_add", "pos": [0, 0], "size": [100, 100], **extra}])

    def test_add_with_auto_id_inputs_and_display(self):
        g, res = apply_ops(base(), [{"op": "add", "type": "transform", "inputs": {"geo": "n"}},
                                    {"op": "add", "type": "transform"}])
        self.assertEqual([r["id"] for r in res], ["transform1", "transform2"])
        self.assertEqual(g.nodes["transform1"].inputs, {"geo": "n"})
        g2, _ = apply_ops(g, [{"op": "add", "type": "output", "id": "out2", "inputs": {"geo": "transform1"},
                               "display": True}])
        self.assertEqual(g2.display, "out2")

    def test_batches_are_atomic(self):
        g = base()
        before = g.to_dict()
        with self.assertRaises(EditError) as cm:
            apply_ops(g, [{"op": "add", "type": "grid", "id": "ok"}, {"op": "set", "id": "g", "params": {"rows": -5}}])
        self.assertEqual(cm.exception.index, 1)
        self.assertIn("rows", str(cm.exception))
        self.assertEqual(g.to_dict(), before)

    def test_rejects_what_can_never_be_valid(self):
        bad = [({"op": "add", "type": "nope"}, "unknown node type"),
               ({"op": "add", "type": "grid", "id": "g"}, "duplicate"),
               ({"op": "add", "type": "grid", "params": {"zzz": 1}}, "zzz"),
               ({"op": "connect", "from": "o", "to": "g"}, "no inputs"),
               ({"op": "connect", "from": "o", "to": "n"}, "cycle"),
               ({"op": "connect", "from": "g", "to": "n", "port": "bogus"}, "no input 'bogus'"),
               ({"op": "rename", "id": "g", "to": "n"}, "already exists"),
               ({"op": "rename", "id": "g", "to": "9x"}, "invalid node id"),
               ({"op": "set", "id": "missing", "params": {}}, "no such node"),
               ({"op": "frobnicate"}, "unknown op"),
               ({"op": "move", "id": "g", "pos": [1]}, "pos must be"),
               ({"op": "meta", "id": "g", "set": {"type": "x"}}, "reserved"),
               ({"op": "set", "id": "g"}, "missing field")]
        with self.assertRaises(EditError) as cm:                         # strings can't become modifier inputs
            apply_ops(base(), [{"op": "add", "type": "wrangle", "id": "w"}, {"op": "expose", "id": "w", "param": "snippet"}])
        self.assertIn("Blender can take", str(cm.exception))
        for op, msg in bad:
            with self.subTest(op=op):
                with self.assertRaises(EditError) as cm:
                    apply_ops(base(), [op])
                self.assertIn(msg, str(cm.exception))

    def test_incomplete_graphs_are_allowed(self):
        g, _ = apply_ops(base(), [{"op": "add", "type": "transform", "id": "t"}])     # input not wired yet
        self.assertTrue(any("not connected" in m for m in g.validate()))

    def test_rename_updates_every_reference(self):
        g, _ = apply_ops(base(), [{"op": "rename", "id": "n", "to": "hills"}])
        self.assertEqual(list(g.nodes), ["g", "hills", "o"])            # order kept
        self.assertEqual(g.nodes["o"].inputs["geo"], "hills")
        self.assertEqual(g.exposed[0].node, "hills")
        g2, _ = apply_ops(g, [{"op": "rename", "id": "o", "to": "out"}])
        self.assertEqual(g2.display, "out")

    def test_set_and_reset_params(self):
        g, res = apply_ops(base(), [{"op": "set", "id": "n", "params": {"amplitude": [0, 0, 2], "seed": 3}}])
        self.assertEqual(res[0]["params"]["amplitude"], [0.0, 0.0, 2.0])
        self.assertEqual(g.nodes["n"].params["seed"], 3)
        g2, res2 = apply_ops(g, [{"op": "set", "id": "n", "params": {"seed": None}}])
        self.assertNotIn("seed", g2.nodes["n"].params)
        self.assertEqual(res2[0]["params"]["seed"], 0)

    def test_connect_and_disconnect_multi_inputs(self):
        g, _ = apply_ops(base(), [{"op": "add", "type": "box", "id": "b"},
                                  {"op": "add", "type": "merge", "id": "m", "inputs": {"geos": ["g"]}},
                                  {"op": "connect", "from": "b", "to": "m"},
                                  {"op": "connect", "from": "n", "to": "m", "index": 0}])
        self.assertEqual(g.nodes["m"].inputs["geos"], ["n", "g", "b"])
        g2, _ = apply_ops(g, [{"op": "disconnect", "to": "m", "from": "g"}])
        self.assertEqual(g2.nodes["m"].inputs["geos"], ["n", "b"])
        g3, _ = apply_ops(g2, [{"op": "disconnect", "to": "m"}])
        self.assertEqual(g3.nodes["m"].inputs, {})

    def test_connect_picks_the_first_free_port(self):
        g, res = apply_ops(base(), [{"op": "add", "type": "copy_to_points", "id": "c"},
                                    {"op": "connect", "from": "g", "to": "c"},
                                    {"op": "connect", "from": "n", "to": "c"}])
        self.assertEqual([r.get("port") for r in res[1:]], ["source", "points"])

    def test_expose_names_and_unexpose(self):
        g, res = apply_ops(base(), [{"op": "expose", "id": "n", "param": "frequency"},
                                    {"op": "add", "type": "noise", "id": "n2", "inputs": {"geo": "n"}},
                                    {"op": "expose", "id": "n2", "param": "frequency"}])
        self.assertEqual(res[0]["name"], "Frequency")
        self.assertEqual(res[2]["name"], "n2 Frequency")                 # name clash resolved
        g2, _ = apply_ops(g, [{"op": "unexpose", "name": "Frequency"}, {"op": "unexpose", "id": "n", "param": "amplitude"}])
        self.assertEqual([e.name for e in g2.exposed], ["n2 Frequency"])

    def test_remove_cleans_references(self):
        g, _ = apply_ops(base(), [{"op": "remove", "id": "n"}])
        self.assertEqual(g.nodes["o"].inputs, {})
        self.assertEqual(g.exposed, [])

    def test_replace_whole_graph(self):
        g, _ = apply_ops(base(), [{"op": "replace", "graph": {"nodes": {"b": {"type": "box"}}, "display": "b"}}])
        self.assertEqual(list(g.nodes), ["b"])

    def test_layout(self):
        g = base()
        self.assertTrue(layout_missing(g))
        ys = [g.nodes[n].meta["pos"][1] for n in ("g", "n", "o")]
        self.assertEqual(ys, sorted(ys))                                   # flows top to bottom
        g2, _ = apply_ops(g, [{"op": "add", "type": "transform", "id": "t", "inputs": {"geo": "n"}}])
        self.assertGreater(g2.nodes["t"].meta["pos"][1], g.nodes["n"].meta["pos"][1])
        self.assertNotEqual(tuple(g2.nodes["t"].meta["pos"]), tuple(g2.nodes["o"].meta["pos"]))  # no overlap
        g3, _ = apply_ops(g2, [{"op": "move", "positions": {"t": [5, 6]}}])
        self.assertEqual(g3.nodes["t"].meta["pos"], [5.0, 6.0])
        self.assertEqual(Graph.from_dict(g3.to_dict()).nodes["t"].meta["pos"], [5.0, 6.0])   # round-trips

    def test_layout_op_tidies_everything(self):
        g = Graph.load(os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), "examples", "graphs", "demo.json"))
        for n in g.nodes.values():
            n.meta["pos"] = [0.0, 0.0]                                    # everything piled up
        g2, _ = apply_ops(g, [{"op": "layout"}])
        pos = {nid: tuple(n.meta["pos"]) for nid, n in g2.nodes.items()}
        self.assertEqual(len(set(pos.values())), len(pos))                # no two nodes on top of each other
        for nid in g2.nodes:
            for src in g2.input_ids(nid):
                self.assertLess(pos[src][1], pos[nid][1])                 # inputs above consumers
        ys = sorted({p[1] for p in pos.values()})
        rows = [[p[0] for p in pos.values() if p[1] == y] for y in ys]
        for xs in rows:
            xs.sort()
            self.assertTrue(all(b - a >= 160 for a, b in zip(xs, xs[1:])))  # nodes in a row don't overlap

    def test_unrelated_broken_branch_does_not_block_the_display_cook(self):
        from minihou import Evaluator
        g, _ = apply_ops(base(), [{"op": "add", "type": "transform", "id": "dangling"}])
        geo = Evaluator().cook(g)
        self.assertEqual(geo.npoints, 16)


class InsertOpTests(unittest.TestCase):
    def chain(self):
        return graph_from({"g": {"type": "grid", "pos": [0, 0]}, "n": {"type": "noise", "inputs": {"geo": "g"}, "pos": [0, 110]},
                           "m": {"type": "merge", "inputs": {"geos": ["g", "n", "g"]}, "pos": [0, 220]},
                           "o": {"type": "output", "inputs": {"geo": "n"}}}, display="n")

    def test_the_new_node_takes_over_every_reader_in_place(self):
        g, res = apply_ops(self.chain(), [{"op": "insert", "after": "n", "type": "transform", "params": {"uniform_scale": 2}}])
        self.assertEqual(res[0], {"id": "transform1", "rewired": 2})
        self.assertEqual(g.nodes["transform1"].inputs, {"geo": "n"})
        self.assertEqual(g.nodes["m"].inputs["geos"], ["g", "transform1", "g"])           # same slot in a multi input
        self.assertEqual(g.nodes["o"].inputs["geo"], "transform1")
        self.assertEqual(g.display, "transform1")                                          # the display flag moves along
        self.assertEqual(g.nodes["transform1"].params, {"uniform_scale": 2.0})
        self.assertEqual(g.nodes["transform1"].meta["pos"], [190.0, 180.0])                # beside and below the node
        from minihou import Evaluator
        self.assertEqual(Evaluator().cook(g).npoints, 100)

    def test_a_leaf_gets_a_plain_append_and_ports_can_be_chosen(self):
        g, _ = apply_ops(self.chain(), [{"op": "insert", "after": "o", "type": "attribute_delete", "id": "tidy",
                                         "params": {"names": "uv"}, "pos": [5, 6]}])
        self.assertEqual((g.nodes["tidy"].inputs, g.nodes["tidy"].meta["pos"]), ({"geo": "o"}, [5.0, 6.0]))
        two, _ = apply_ops(self.chain(), [{"op": "insert", "after": "g", "type": "copy_to_points", "port": "points"}])
        self.assertEqual(two.nodes["copy_to_points1"].inputs, {"points": "g"})
        self.assertEqual(two.nodes["n"].inputs["geo"], "copy_to_points1")

    def test_bad_inserts_change_nothing(self):
        g = self.chain()
        for bad in ({"op": "insert", "after": "nope", "type": "transform"}, {"op": "insert", "after": "n", "type": "nope"},
                    {"op": "insert", "after": "n", "type": "grid"}, {"op": "insert", "after": "n", "type": "transform", "port": "x"},
                    {"op": "insert", "after": "n", "type": "transform", "id": "g"}):
            with self.assertRaises(EditError, msg=bad):
                apply_ops(g, [bad])
        self.assertEqual(g.nodes["m"].inputs["geos"], ["g", "n", "g"])


class NetworkBoxTests(unittest.TestCase):
    def test_box_around_nodes_set_and_remove(self):
        g, res = apply_ops(base(), [{"op": "move", "positions": {"g": [0, 0], "n": [200, 110], "o": [0, 220]}},
                                    {"op": "box_add", "around": ["g", "n"], "title": "Source", "color": "#335577"}])
        self.assertEqual(res[1]["id"], "box1")
        (b,) = g.boxes
        self.assertEqual((b["title"], b["color"]), ("Source", "#335577"))
        self.assertLessEqual(b["pos"][0], 0)
        self.assertGreaterEqual(b["pos"][0] + b["size"][0], 200 + 160)                  # holds both nodes entirely
        self.assertGreaterEqual(b["pos"][1] + b["size"][1], 110 + 46)
        g2, _ = apply_ops(g, [{"op": "box_set", "id": "box1", "title": "Renamed", "pos": [5, 6], "size": [300, 200]}])
        self.assertEqual((g2.boxes[0]["title"], g2.boxes[0]["pos"], g2.boxes[0]["size"]), ("Renamed", [5.0, 6.0], [300.0, 200.0]))
        self.assertEqual(g.boxes[0]["title"], "Source")                                  # edits work on a copy
        g3, _ = apply_ops(g2, [{"op": "box_add", "pos": [0, 0], "size": [50, 50]}, {"op": "box_remove", "id": "box1"}])
        self.assertEqual([b["id"] for b in g3.boxes], ["box2"])

    def test_round_trip_and_bad_boxes(self):
        g, _ = apply_ops(base(), [{"op": "box_add", "pos": [1, 2], "size": [100, 80], "title": "A, [b]"}])
        again = Graph.from_json(g.to_json(), check=False)
        self.assertEqual(again.boxes, g.boxes)
        self.assertEqual(again.to_dict(), g.to_dict())
        for bad in ({"op": "box_add", "pos": [0, 0], "size": [10, 10], "color": "red"},
                    {"op": "box_add", "pos": [0], "size": [10, 10]},
                    {"op": "box_add", "id": "box 1", "pos": [0, 0], "size": [10, 10]},
                    {"op": "box_add", "around": ["nope"]},
                    {"op": "box_set", "id": "missing", "title": "x"},
                    {"op": "box_remove", "id": "missing"}):
            with self.assertRaises(EditError, msg=bad):
                apply_ops(g, [bad])
        with self.assertRaises(Exception):
            Graph.from_dict({"nodes": {}, "boxes": [{"id": "b", "pos": "x", "size": [1, 1]}]})

    def test_boxes_do_not_change_what_cooks(self):
        from minihou import Evaluator
        g = base()
        k1 = Evaluator().cook(g)
        h = apply_ops(g, [{"op": "box_add", "around": ["g", "n"]}])[0]
        ev = Evaluator()
        ev.cook(g)
        ev.cook(h)
        self.assertEqual(ev.last_stats.cooked, [])                                       # all cached: boxes are cosmetic
        self.assertEqual(k1.npoints, ev.cook(h).npoints)

    def test_tidy_layout_keeps_boxes_around_their_nodes(self):
        g, _ = apply_ops(base(), [{"op": "move", "positions": {"g": [0, 0], "n": [500, 20], "o": [900, 0]}},
                                  {"op": "box_add", "around": ["g", "n"]}])
        g2, _ = apply_ops(g, [{"op": "layout"}])
        (b,) = g2.boxes
        for nid in ("g", "n"):
            x, y = g2.nodes[nid].meta["pos"]
            self.assertTrue(b["pos"][0] <= x and x + 160 <= b["pos"][0] + b["size"][0], nid)
            self.assertTrue(b["pos"][1] <= y and y + 46 <= b["pos"][1] + b["size"][1], nid)
        x, y = g2.nodes["o"].meta["pos"]
        self.assertFalse(b["pos"][1] <= y <= b["pos"][1] + b["size"][1] and b["pos"][0] <= x <= b["pos"][0] + b["size"][0])

    def test_session_undo_covers_boxes(self):
        s = Session(base())
        s.edit([{"op": "box_add", "pos": [0, 0], "size": [100, 100]}])
        self.assertEqual(len(s.graph.boxes), 1)
        self.assertTrue(s.undo())
        self.assertEqual(s.graph.boxes, [])
        self.assertTrue(s.redo())
        self.assertEqual(len(s.snapshot()["graph"]["boxes"]), 1)


class SessionEditTests(unittest.TestCase):
    def setUp(self):
        self.s = Session(base())
        self.s.start()
        self.addCleanup(self.s.stop)

    def test_edit_cooks_and_reports(self):
        r = self.s.edit([{"op": "set", "id": "g", "params": {"columns": 10}}])
        self.assertTrue(r["changed"])
        self.assertTrue(self.s.wait_idle())
        st = self.s.status()
        self.assertTrue(st["ok"])
        self.assertEqual(st["points"], 40)
        self.assertEqual(st["cooked_rev"], r["rev"])

    def test_errors_name_the_node(self):
        self.s.edit([{"op": "add", "type": "wrangle", "id": "w", "inputs": {"geo": "n"},
                      "params": {"snippet": "@P.z = nope"}}, {"op": "connect", "from": "w", "to": "o"}])
        self.s.wait_idle()
        st = self.s.status()
        self.assertFalse(st["ok"])
        self.assertEqual(st["error"]["node"], "w")
        self.s.edit([{"op": "remove", "id": "o"}])
        self.s.wait_idle()
        self.assertIn("no display node", self.s.status()["error"]["message"])

    def test_undo_redo_and_slider_drags_merge(self):
        rev0 = self.s.snapshot()["graph"]
        for v in (1, 2, 3, 4):
            self.s.edit([{"op": "set", "id": "n", "params": {"seed": v}}])
        self.s.edit([{"op": "add", "type": "box", "id": "b"}])
        self.assertTrue(self.s.undo())
        self.assertNotIn("b", self.s.graph.nodes)
        self.assertEqual(self.s.graph.nodes["n"].params["seed"], 4)
        self.assertTrue(self.s.undo())                                  # the whole drag in one step
        self.assertEqual(self.s.snapshot()["graph"], rev0)
        self.assertFalse(self.s.undo())
        self.assertTrue(self.s.redo())
        self.assertEqual(self.s.graph.nodes["n"].params["seed"], 4)
        self.s.edit([{"op": "set", "id": "g", "params": {"rows": 7}}])
        self.assertFalse(self.s.redo())                                  # new edit clears redo

    def test_failed_batch_changes_nothing(self):
        rev = self.s.rev
        with self.assertRaises(EditError):
            self.s.edit([{"op": "add", "type": "box"}, {"op": "add", "type": "nope"}])
        self.assertEqual(self.s.rev, rev)
        self.assertFalse(self.s.snapshot()["can_undo"])

    def test_noop_edit_does_not_bump(self):
        rev = self.s.rev
        r = self.s.edit([{"op": "set", "id": "g", "params": {"columns": 4}}])
        self.assertFalse(r["changed"])
        self.assertEqual(self.s.rev, rev)

    def test_inspect_any_node(self):
        out = self.s.inspect("g", detail=True)
        self.assertTrue(out["ok"])
        self.assertEqual((out["points"], out["prims"]), (16, 9))
        names = {(a["domain"], a["name"]) for a in out["attributes"]}
        self.assertIn(("point", "P"), names)
        P = next(a for a in out["attributes"] if a["name"] == "P")
        self.assertEqual(P["components"], 3)
        self.assertEqual(len(P["min"]), 3)
        self.assertEqual(out["bounds"]["size"][2], 0.0)
        self.assertFalse(self.s.inspect("zzz")["ok"])

    def test_listeners_hear_graph_and_interface_changes(self):
        events = []

        class L:
            def on_result(self, r): events.append(("result",))
            def on_interface(self, i): events.append(("iface", i["params"][0]["value"]))
            def on_graph(self, rev, origin): events.append(("graph", origin))

        self.s.add_listener(L())
        self.s.edit([{"op": "set", "id": "n", "params": {"amplitude": [0, 0, 3]}}], origin="editor")
        self.assertIn(("iface", [0.0, 0.0, 3.0]), events)                # editor edits reach Blender
        self.assertIn(("graph", "editor"), events)
        events.clear()
        self.s.set_param("Amp", [0, 0, 4])                               # Blender's own edit: no echo
        self.assertEqual([e for e in events if e[0] == "iface"], [])
        self.assertIn(("graph", "link"), events)                         # ...but the editor hears it


class SessionFileTests(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.TemporaryDirectory()
        self.addCleanup(self.dir.cleanup)
        self.path = os.path.join(self.dir.name, "g.json")
        base().save(self.path)
        self.s = Session(Graph.load(self.path))
        self.s.start()
        self.s.watch_file(self.path, interval=0.02)
        self.addCleanup(self.s.stop)

    def read(self):
        for _ in range(50):
            try:
                with open(self.path, encoding="utf-8") as f:
                    return json.load(f)
            except PermissionError:      # Windows: the session is replacing the file right now; try again
                time.sleep(0.01)
        raise AssertionError(f"{self.path} stayed locked")

    def wait(self, pred, timeout=3.0):
        end = time.monotonic() + timeout
        while time.monotonic() < end:
            if pred():
                return True
            time.sleep(0.02)
        return False

    def test_edits_are_saved_without_reloading_themselves(self):
        self.s.edit([{"op": "set", "id": "g", "params": {"rows": 9}}])
        self.assertTrue(self.wait(lambda: self.read()["nodes"]["g"]["params"].get("rows") == 9))
        rev = self.s.rev
        time.sleep(0.15)
        self.assertEqual(self.s.rev, rev)                                # our own write was not "reloaded"
        self.assertTrue(self.s.snapshot()["can_undo"])                   # and undo history is intact

    def test_external_change_reloads_and_is_undoable(self):
        d = self.read()
        d["nodes"]["g"]["params"]["rows"] = 6
        time.sleep(0.05)
        with open(self.path, "w", encoding="utf-8") as f:
            json.dump(d, f)
        self.assertTrue(self.wait(lambda: self.s.graph.nodes["g"].params.get("rows") == 6))
        self.assertTrue(self.s.undo())
        self.assertEqual(self.s.graph.nodes["g"].params.get("rows"), 4)

    def test_incomplete_graph_saves_and_reopens(self):
        self.s.edit([{"op": "add", "type": "transform", "id": "t"}])     # unwired: invalid for check()
        self.s.flush()
        s2 = Session(Graph.load(self.path, check=False))
        self.assertIn("t", s2.graph.nodes)

    def test_open_and_create_files(self):
        other = os.path.join(self.dir.name, "sub", "new.json")
        snap = self.s.open_file(other, create=True)
        self.assertTrue(os.path.exists(other))
        self.assertEqual(snap["path"], os.path.realpath(other))
        self.assertFalse(snap["can_undo"])
        self.assertTrue(self.s.wait_idle())
        self.assertTrue(self.s.status()["ok"])
        self.s.edit([{"op": "set", "id": "box1", "params": {"size": [3, 2, 1]}}])
        self.assertTrue(self.wait(lambda: _load(other)["nodes"]["box1"].get("params", {}).get("size") == [3, 2, 1]))
        with self.assertRaises(Exception):
            self.s.open_file(os.path.join(self.dir.name, "missing.json"))


if __name__ == "__main__":
    unittest.main()


class FileFormatTests(unittest.TestCase):
    """What a saved graph file looks like: clean numbers, short arrays on one line, lossless round-trips."""

    def test_float32_values_from_blender_are_saved_as_short_decimals(self):
        import numpy as np
        from minihou.link.session import snap_float32
        self.assertEqual(snap_float32(float(np.float32(0.22))), 0.22)
        self.assertEqual(snap_float32([float(np.float32(0.1)), 0.0, float(np.float32(1.2))]), [0.1, 0.0, 1.2])
        self.assertEqual(snap_float32(0.1), 0.1)                               # not a float32: untouched
        self.assertEqual(snap_float32(0.30000000000000004), 0.30000000000000004)
        self.assertEqual((snap_float32(7), snap_float32(True), snap_float32("x")), (7, True, "x"))
        for v in (0.22, 1e-8, 123456.79, -3.3, 1e30):                          # always the same float32 afterwards
            f = float(np.float32(v))
            self.assertEqual(np.float32(snap_float32(f)), np.float32(f))
        s = Session(base())
        s.set_param("Amp", [float(np.float32(0.22)), 0.0, float(np.float32(1.2))])            # from Blender
        self.assertEqual(s.graph.nodes["n"].params["amplitude"], (0.22, 0.0, 1.2))
        s.edit([{"op": "set", "id": "n", "params": {"frequency": float(np.float32(0.7))}}])   # editor/AI: exact
        self.assertEqual(s.graph.nodes["n"].params["frequency"], float(np.float32(0.7)))

    def test_saved_json_is_tidy_and_round_trips(self):
        g, _ = apply_ops(base(), [
            {"op": "set", "id": "g", "params": {"size_x": 10.0}},
            {"op": "set", "id": "n", "params": {"amplitude": [0, 0.5, 2.0]}},
            {"op": "add", "type": "merge", "id": "m", "inputs": {"geos": ["g", "n"]}},
            {"op": "add", "type": "wrangle", "id": "w", "inputs": {"geo": "m"},
             "params": {"snippet": "a = 1, 2\n# [not, an, array]\n@P.z += a"}, "comment": "x, [y]"},
            {"op": "move", "positions": {"m": [100.0, 220.0]}}])
        text = g.to_json()
        self.assertRegex(text, r'"size_x": 10\n')
        self.assertIn('"amplitude": [0, 0.5, 2]', text)
        self.assertIn('"geos": ["g", "n"]', text)
        self.assertIn('"pos": [100, 220]', text)
        self.assertEqual(json.loads(text), g.to_dict())                         # formatting is lossless
        g2 = Graph.from_json(text, check=False)
        self.assertEqual(g2.to_dict(), g.to_dict())
        self.assertEqual(g2.nodes["g"].params["size_x"], 10.0)                  # ints read back as floats
        self.assertIsInstance(g2.nodes["g"].params["size_x"], float)
        self.assertEqual(g2.nodes["w"].params["snippet"], "a = 1, 2\n# [not, an, array]\n@P.z += a")


class ModifierInputOwnershipTests(unittest.TestCase):
    def test_rename_delete_undo_and_roundtrip(self):
        graph = Graph.from_dict({"nodes": {
            "input": {"type": "modifier_input"},
            "socket": {"type": "object_input", "input_owner": "input", "input_label": "Monkey"}}, "display": "input"})
        session = Session(graph)
        session.edit([{"op": "rename", "id": "input", "to": "group_input"}])
        self.assertEqual(session.graph.nodes["socket"].meta["input_owner"], "group_input")
        restored = Graph.from_json(session.graph.to_json())
        self.assertEqual(restored.nodes["socket"].meta["input_label"], "Monkey")
        session.edit([{"op": "remove", "id": "group_input"}])
        self.assertNotIn("socket", session.graph.nodes)
        session.undo()
        self.assertIn("socket", session.graph.nodes)
