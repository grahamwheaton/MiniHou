"""Addon logic that doesn't need Blender: inbox merging, param diffing, feedback-loop breaker, sync."""
import ast
import threading
import unittest
from pathlib import Path

import numpy as np

from minihou.link.diff import DiffState
from minihou.link.protocol import GeoUpdate, Mirror
from tests.addon_loader import ADDON_DIR, pure_module
from tests.helpers import quad_geo

ROOT = Path(__file__).resolve().parent.parent


class AddonSyncTests(unittest.TestCase):
    def test_shared_files_are_identical_copies(self):
        for name in ("protocol.py", "client.py", "instance_client.py", "shm.py"):
            self.assertEqual((ROOT / "minihou" / "link" / name).read_bytes(), (ADDON_DIR / name).read_bytes(),
                             f"{name} drifted; run: python tools/sync_addon.py")

    def test_protocol_module_is_self_contained(self):
        """protocol.py gets copied into Blender, so it may only import stdlib + numpy (no relative imports)."""
        tree = ast.parse((ROOT / "minihou" / "link" / "protocol.py").read_text(encoding="utf-8"))
        allowed = {"__future__", "json", "socket", "struct", "time", "typing", "numpy"}
        for node in ast.walk(tree):
            if isinstance(node, ast.ImportFrom):
                self.assertEqual(node.level, 0, "relative import in protocol.py")
                self.assertIn(node.module.split(".")[0], allowed)
            elif isinstance(node, ast.Import):
                for a in node.names:
                    self.assertIn(a.name.split(".")[0], allowed)

    def test_engine_has_no_bpy_imports(self):
        """The engine must run (and be testable) without Blender: no import of bpy anywhere."""
        for py in (ROOT / "minihou").rglob("*.py"):
            for node in ast.walk(ast.parse(py.read_text(encoding="utf-8"))):
                mods = ([a.name for a in node.names] if isinstance(node, ast.Import)
                        else [node.module or ""] if isinstance(node, ast.ImportFrom) else [])
                for m in mods:
                    self.assertNotEqual(m.split(".")[0], "bpy", f"{py} imports bpy")

    def test_only_listed_addon_modules_import_bpy(self):
        for name in ("state.py", "protocol.py", "client.py", "instance_client.py", "server_control.py", "shm.py"):
            for node in ast.walk(ast.parse((ADDON_DIR / name).read_text(encoding="utf-8"))):
                mods = ([a.name for a in node.names] if isinstance(node, ast.Import)
                        else [node.module or ""] if isinstance(node, ast.ImportFrom) else [])
                for m in mods:
                    self.assertNotEqual(m.split(".")[0], "bpy", f"{name} must stay bpy-free")


class InboxTests(unittest.TestCase):
    def make_updates(self):
        d = DiffState()
        g1 = quad_geo().freeze()
        g2 = g1.copy(); g2.point["P"] = g1.P + 1; g2.freeze()
        g3 = g2.copy(); g3.point["P"] = g2.P + 1; g3.freeze()
        return [d.build(g, i + 1) for i, g in enumerate((g1, g2, g3))]

    def test_geo_updates_are_merged_while_waiting(self):
        Inbox = pure_module("state").Inbox
        inbox = Inbox()
        for h, a in self.make_updates():
            inbox.push(h, a)
        events, geo = inbox.take()
        self.assertEqual(events, [])
        self.assertEqual(geo.seq, 3)
        self.assertEqual(inbox.geo_received, 3)
        self.assertEqual(inbox.geo_merged, 2)
        m = Mirror()
        m.apply(geo)
        np.testing.assert_array_equal(m.P, quad_geo().P + 2)
        self.assertEqual(inbox.take(), ([], None))

    def test_other_events_keep_order_and_are_not_merged(self):
        Inbox = pure_module("state").Inbox
        inbox = Inbox()
        inbox.push({"type": "welcome"}, {})
        inbox.push({"type": "interface", "params": []}, {})
        inbox.push({"type": "error", "message": "x"}, {})
        events, geo = inbox.take()
        self.assertEqual([e["type"] for e in events], ["welcome", "interface", "error"])
        self.assertIsNone(geo)

    def test_push_from_many_threads_is_safe(self):
        Inbox = pure_module("state").Inbox
        inbox = Inbox()
        ups = self.make_updates()

        def work():
            for h, a in ups * 50:
                inbox.push(dict(h), dict(a))
                inbox.push({"type": "pong"}, {})

        ts = [threading.Thread(target=work) for _ in range(4)]
        [t.start() for t in ts]
        [t.join() for t in ts]
        events, geo = inbox.take()
        self.assertEqual(len(events), 4 * 150)
        self.assertEqual(inbox.geo_received, 4 * 150)
        self.assertIsNotNone(geo)


class ParamForwarderTests(unittest.TestCase):
    def setUp(self):
        self.F = pure_module("state").ParamForwarder

    def test_only_differences_are_forwarded(self):
        f = self.F()
        f.reset({"a": 1.0, "v": (0.0, 0.0, 1.0), "n": 3, "b": True})
        self.assertEqual(f.changes({"a": 1.0, "v": (0.0, 0.0, 1.0), "n": 3, "b": True}), {})
        self.assertEqual(f.changes({"a": 2.0, "v": (0.0, 0.0, 1.0), "n": 3, "b": True}), {"a": 2.0})
        self.assertEqual(set(f.changes({"a": 1.0, "v": (0.0, 1.0, 1.0), "n": 4, "b": False})), {"v", "n", "b"})

    def test_changes_is_pure_and_commit_records_delivery(self):
        f = self.F()
        f.reset({"a": 1.0})
        self.assertEqual(f.changes({"a": 2.0}), {"a": 2.0})
        self.assertEqual(f.changes({"a": 2.0}), {"a": 2.0})      # not delivered yet: still pending
        f.commit("a", 2.0, 0.0)
        self.assertEqual(f.changes({"a": 2.0}), {})

    def test_float_noise_below_tolerance_is_ignored(self):
        f = self.F()
        f.reset({"a": 0.1 + 0.2})
        self.assertEqual(f.changes({"a": 0.3}), {})

    def test_float32_roundtrip_is_not_an_edit(self):
        """Regression: Blender stores socket floats as float32; 0.4 reads back as 0.40000000596..."""
        f = self.F()
        f.reset({"a": 0.4, "v": (0.1, 0.2, 0.3), "big": 12345.678})
        back = {"a": float(np.float32(0.4)), "v": tuple(float(np.float32(c)) for c in (0.1, 0.2, 0.3)),
                "big": float(np.float32(12345.678))}
        self.assertEqual(f.changes(back), {})
        self.assertEqual(set(f.changes({"a": 0.4001, "v": (0.1, 0.2, 0.3), "big": 12345.678})), {"a"})

    def test_unknown_parameters_are_ignored(self):
        f = self.F()
        f.reset({"a": 1.0})
        self.assertEqual(f.changes({"zzz": 5.0}), {})

    def test_bool_is_not_confused_with_numbers(self):
        f = self.F()
        f.reset({"b": True})
        self.assertEqual(f.changes({"b": False}), {"b": False})

    def test_circuit_breaker_trips_on_runaway_forwarding_and_resets(self):
        f = self.F(max_per_second=10)
        f.reset({"a": 0.0})
        t = 0.0
        for i in range(1, 30):
            ch = f.changes({"a": float(i)})
            if not ch:
                break
            f.commit("a", float(i), t)
            t += 0.01
        self.assertTrue(f.tripped)
        self.assertLessEqual(i, 12)
        self.assertEqual(f.changes({"a": 999.0}), {})
        f.reset({"a": 0.0})
        self.assertFalse(f.tripped)

    def test_human_speed_edits_never_trip(self):
        f = self.F(max_per_second=120)
        f.reset({"a": 0.0})
        t = 0.0
        for i in range(1, 500):
            ch = f.changes({"a": float(i)})
            self.assertEqual(len(ch), 1)
            f.commit("a", float(i), t)
            t += 1 / 30
        self.assertFalse(f.tripped)


if __name__ == "__main__":
    unittest.main()
