"""input_define_v1 add-on logic without Blender: rules mirror rust/crates/minihou-app/src/input_define.rs."""
import importlib
import json
import sys
import types
import unittest
from pathlib import Path
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
PACKAGE = "_input_define_addon"
package = types.ModuleType(PACKAGE)
package.__path__ = [str(ROOT / "blender_addon/minihou_link")]
with patch.dict(sys.modules, {PACKAGE: package}):
    D = importlib.import_module(PACKAGE + ".input_define")

GRAPH = {"display": "input", "exposed": [{"name": "Scale", "node": "t", "param": "s"}],
         "nodes": {"input": {"type": "modifier_input",
                             "modifier_inputs": [{"id": "input_1", "name": "Amount", "kind": "float", "default": 0.0}],
                             "input_values": {"input_1": 2.0}},
                   "t": {"type": "transform"}}}


class PureRules(unittest.TestCase):
    def test_collision_suffixes_ids_and_reserved_names(self):
        graph, entry = D.apply_offline(GRAPH, D.request(input={"name": "Amount", "kind": "object"}))
        self.assertEqual((entry["id"], entry["name"], entry["default"], entry["node"]), ("input_2", "Amount 2", "", "input"))
        for wanted, got in (("Scale", "Scale 2"), ("Geometry", "Geometry 2"), ("Object: x", "Object x"), ("", "Input")):
            self.assertEqual(D.plan(graph, D.request(input={"name": wanted, "kind": "bool"}))[2]["name"], got)
        self.assertEqual(len(GRAPH["nodes"]["input"]["modifier_inputs"]), 1, "offline apply must copy")

    def test_operator_drafts_for_every_dialog_type(self):
        cases = {"float": 1.5, "int": 3, "bool": True, "vector": [1.0, 2.0, 3.0], "color": [0.5, 0.5, 0.5, 1.0],
                 "string": "hi", "menu": "A", "object": "Cube"}
        for kind, want in cases.items():
            draft = D.build_draft(kind, kind.title(), float_value=1.5, int_value=3, bool_value=True,
                                  vector=(1, 2, 3), color=(.5, .5, .5, 1), string="hi", id_name="Cube",
                                  options="A, B", use_min=kind == "float", min_value=0.0, doc=" tip ")
            _, entry = D.apply_offline(GRAPH, D.request(input=draft))
            self.assertEqual(entry["default"], want, kind)
            self.assertEqual(entry["doc"], "tip")
        self.assertEqual(D.build_draft("float", "x", use_min=True, min_value=-1)["min"], -1.0)
        self.assertEqual(D.build_draft("vector", "x", subtype="xyz")["subtype"], "xyz")
        self.assertNotIn("subtype", D.build_draft("vector", "x", subtype="NONE"))

    def test_every_kind_definable_and_bad_definitions_refused(self):
        for kind in D.KINDS:
            D.plan(GRAPH, D.request(input={"name": kind, "kind": kind, "options": ["A", "B"]}))
        for bad in ({"name": "x", "kind": "quaternion"}, {"name": "x", "kind": "float", "default": "s"},
                    {"name": "x", "kind": "float", "min": 2.0, "max": 1.0}, {"name": "x", "kind": "menu"},
                    {"name": "x", "kind": "float", "default": 5.0, "max": 1.0},
                    {"name": "x" * D.MAX_MESSAGE_BYTES, "kind": "float"}):
            with self.assertRaises(ValueError):
                D.plan(GRAPH, D.request(input=bad))
        with self.assertRaises(ValueError):
            D.plan({"nodes": {"b": {"type": "box"}}}, D.request(input={"kind": "float"}))

    def test_rename_keeps_id_and_remove_drops_value(self):
        graph, entry = D.apply_offline(GRAPH, D.request("rename", ident="input_1", name="Scale"))
        self.assertEqual((entry["id"], entry["name"]), ("input_1", "Scale 2"))
        self.assertEqual(graph["nodes"]["input"]["input_values"], {"input_1": 2.0}, "values follow the stable id")
        graph, _ = D.apply_offline(GRAPH, D.request("remove", ident="input_1"))
        self.assertEqual(graph["nodes"]["input"]["modifier_inputs"], [])
        self.assertEqual(graph["nodes"]["input"]["input_values"], {})

    def test_merged_interface_keeps_exposed_and_follows_graph(self):
        graph, _ = D.apply_offline(GRAPH, D.request(input={"name": "Target", "kind": "object"}))
        old = [{"name": "Scale", "kind": "float", "node": "t", "param": "s", "value": 1.0},
               {"name": "Amount", "kind": "float", "node": "input", "id": "input_1", "value": 2.0}]
        params = D.merged_interface(old, graph)
        self.assertEqual([(p["name"], p["kind"]) for p in params], [("Scale", "float"), ("Amount", "float"), ("Target", "object")])
        self.assertEqual(params[1]["value"], 2.0)


class FakeClient:
    def __init__(self, caps, reply=None):
        self.caps, self.reply, self.sent = caps, reply, []

    def has_capability(self, name):
        return name in self.caps

    def define_input(self, msg):
        self.sent.append(msg)
        return self.reply


class Group(dict):
    pass


class DefineRouting(unittest.TestCase):
    def setUp(self):
        self.ng = Group(minihou_modifier_id="m1", minihou_graph_path="C:/live.json",
                        minihou_graph=json.dumps(GRAPH), minihou_parameters="[]")
        self.applied, self.written = [], []
        embed = types.SimpleNamespace(EMBEDDED="EMBEDDED", IDS=("minihou_modifier_id", "minihou_gn_id"),
                                      runtime_graph=lambda ng: json.loads(ng["minihou_graph"]),
                                      mode=lambda ng: ng.get("mode", "EMBEDDED"),
                                      write=lambda ng: self.written.append(json.loads(ng["minihou_graph"])),
                                      note_graph=lambda ng: None)
        modifiers = types.SimpleNamespace(PATH="minihou_graph_path", GRAPH="minihou_graph", PARAMS="minihou_parameters")
        patcher = patch.dict(sys.modules, {PACKAGE: package, PACKAGE + ".embed": embed, PACKAGE + ".modifiers": modifiers})
        patcher.start()
        self.addCleanup(patcher.stop)
        package.embed, package.modifiers = embed, modifiers
        D._apply_interface = lambda ng, params, mod=None: self.applied.append(params)

    def test_bound_app_applies_and_returns_its_graph(self):
        graph, entry = D.apply_offline(GRAPH, D.request(input={"name": "T", "kind": "object"}))
        client = FakeClient({"input_define_v1"}, {"status": "ok", "input": entry, "graph": graph,
                                                  "interface": [{"name": "T", "kind": "object"}]})
        got, route = D.define(self.ng, D.request(input={"name": "T", "kind": "object"}), client=client)
        self.assertEqual((route, got["name"]), ("app", "T"))
        self.assertEqual((client.sent[0]["instance_id"], client.sent[0]["path"]), ("m1", "C:/live.json"))
        self.assertEqual(self.applied, [[{"name": "T", "kind": "object"}]])
        self.assertEqual(self.written, [])

    def test_not_bound_and_older_peer_fall_back_to_embedded_text(self):
        for client, want in ((FakeClient({"input_define_v1"}, {"status": "error", "code": "not_bound", "message": "x"}), "offline"),
                             (FakeClient(set()), "offline_old_app"), (None, "offline")):
            self.setUp()
            entry, route = D.define(self.ng, D.request(input={"name": "T", "kind": "object"}), client=client, connect=False)
            self.assertEqual((route, entry["kind"]), (want, "object"))
            self.assertEqual(self.written[-1]["nodes"]["input"]["modifier_inputs"][-1]["name"], "T")
            self.assertEqual(self.applied[-1][-1]["name"], "T")

    def test_app_refusal_and_linked_offline_raise(self):
        client = FakeClient({"input_define_v1"}, {"status": "error", "code": "invalid", "message": "bad default"})
        with self.assertRaisesRegex(ValueError, "bad default"):
            D.define(self.ng, D.request(input={"name": "T", "kind": "object"}), client=client)
        self.ng["mode"] = "LINKED"
        with self.assertRaisesRegex(ValueError, "Linked File"):
            D.define(self.ng, D.request(input={"name": "T", "kind": "object"}), client=FakeClient(set()))
        with self.assertRaisesRegex(ValueError, "unsupported"):
            D.define(self.ng, D.request(input={"name": "T", "kind": "nope"}), client=None, connect=False)


if __name__ == "__main__":
    unittest.main()
