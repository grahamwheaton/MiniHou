"""Pure tests (no Blender) for the add-on's object-name search and Blender Object pickers (add-on 0.5.0)."""
import importlib
import sys
import types
import unittest
from pathlib import Path
from unittest.mock import patch

ROOT = Path(__file__).resolve().parent.parent
PACKAGE = "_object_names_addon"
package = types.ModuleType(PACKAGE)
package.__path__ = [str(ROOT / "blender_addon/minihou_link")]
with patch.dict(sys.modules, {PACKAGE: package}):           # the add-on package imports bpy; these modules do not
    on = importlib.import_module(PACKAGE + ".object_names")
    d = importlib.import_module(PACKAGE + ".input_define")


class Obj:
    def __init__(self, name, type="EMPTY"):
        self.name, self.type = name, type


class Objects(list):
    def get(self, name):
        return next((o for o in self if o.name == name), None)


class ScoreTests(unittest.TestCase):
    def test_same_order_as_the_app(self):
        # Mirrors rust/crates/minihou-app/src/object_names.rs scoring_prefers_exact_prefix_word_starts_and_short_names.
        self.assertEqual(on.score("cube", "Cube"), 1000)
        self.assertGreater(on.score("cu", "Cube"), on.score("cu", "Cube.001"))
        self.assertGreater(on.score("cu", "Cube.001"), on.score("cu", "BigCube"))
        self.assertGreater(on.score("cu", "BigCube"), on.score("cu", "Biscuit"))
        self.assertGreater(on.score("bc", "Big_Cube"), on.score("bc", "abacus"))
        self.assertIsNone(on.score("xyz", "Cube"))


class SearchTests(unittest.TestCase):
    def test_ranked_bounded_and_resumable(self):
        objs = Objects([Obj("Biscuit"), Obj("BigCube", "MESH"), Obj("Camera", "CAMERA"), Obj("Cube", "MESH"),
                        Obj("Cube.001", "MESH")] + [Obj(f"Empty_{i:04d}") for i in range(2000)])
        final, partials, ticks, _ = on.run_blocking("cu", objs, budget_ms=0.01)
        self.assertGreater(ticks, 1, "time-sliced")
        self.assertEqual([m["name"] for m in final["matches"][:3]], ["Cube", "Cube.001", "BigCube"])
        self.assertEqual(final["matches"][0], {"name": "Cube", "type": "MESH", "mesh": True})
        self.assertEqual((final["count"], final["partial"]), (2005, False))
        everything, _, _, _ = on.run_blocking("e", objs)
        self.assertEqual(len(everything["matches"]), on.LIMIT)

    def test_newer_query_replaces_and_cache_is_reused(self):
        objs = Objects(Obj(f"Rock_{i:04d}") for i in range(3000))
        s = on.Searcher()
        out = []
        s.start({"request_id": 1, "query": "rock"}, objs)
        s.step(out.append, 0.01)
        s.start({"request_id": 2, "query": "k_0002"}, objs)
        while s.step(out.append):
            pass
        self.assertEqual(out[-1]["request_id"], 2)
        self.assertEqual(out[-1]["matches"][0]["name"], "Rock_0002")
        s.start({"request_id": 3, "query": "0010"}, objs)
        self.assertIsNotNone(s.job.cache, "complete scan cached the names")
        while s.step(out.append):
            pass
        self.assertEqual(out[-1]["matches"][0]["name"], "Rock_0010")

    def test_removed_cursor_object_restarts_the_iterator(self):
        objs = Objects(Obj(f"N{i:04d}") for i in range(1000))
        job = on.Job(1, "n05", 50, objs)
        job.step(0.01)
        self.assertFalse(job.done())
        on.GENERATION[0] += 1                      # undo/load: never resume a held iterator
        while not job.step():
            pass
        r = job.response("h", False)
        self.assertEqual((r["matches"][0]["name"], r["count"]), ("N0500", 1000))
        names = [m["name"] for m in r["matches"]]
        self.assertEqual(len(names), len(set(names)), "restart skipped what was already read")


class PickerEntryTests(unittest.TestCase):
    GRAPH = {"nodes": {"b": {"type": "blender_object", "params": {"object": "Cube"}},
                       "c": {"type": "blender_object", "picker_id": "b"},
                       "input": {"type": "modifier_input", "modifier_inputs": [
                           {"id": "input_1", "name": "Amount", "kind": "float", "default": 0.0}]}}}

    def test_mirrors_the_app(self):
        self.assertEqual(d.pickers(self.GRAPH), [("b", "b"), ("c", "c")])
        entry = d.picker_entries(self.GRAPH)[0]
        self.assertEqual((entry["node"], entry["id"], entry["kind"], entry["value"]), ("@picker", "b", "object", "Cube"))
        self.assertIn("b", d.taken_names(self.GRAPH))
        self.assertEqual([e["name"] for e in d.entries(self.GRAPH)], ["Amount"], "pickers are not dynamic inputs")
        merged = d.merged_interface([{"name": "old", "node": "@picker", "id": "gone", "kind": "object"}], self.GRAPH)
        self.assertEqual([p["name"] for p in merged], ["Amount", "b", "c"])


if __name__ == "__main__":
    unittest.main()
