"""The bpy-free planner that decides which material goes in which slot."""
import unittest

from tests.addon_loader import pure_module

ms = pure_module("material_slots")


class PlanTests(unittest.TestCase):
    def test_names_fill_slots_and_empty_names_leave_them_alone(self):
        self.assertEqual(ms.plan(["Base", "Glass"], ["", "Steel", "Wood"]), ["Base", "Steel", "Wood"])

    def test_slots_are_added_never_removed_or_reordered(self):
        self.assertEqual(ms.plan(["A", "B", "C"], ["X"]), ["X", "B", "C"])
        self.assertEqual(ms.plan([], ["", "Wood"]), [None, "Wood"])

    def test_no_names_no_change(self):
        for names in (None, [], "nope", 3):
            self.assertEqual(ms.plan(["A"], names), ["A"])
        self.assertFalse(ms.changes(["A", "B"], ["A", ""]))
        self.assertTrue(ms.changes(["A", "B"], ["A", "C"]))

    def test_non_string_entries_become_strings(self):
        self.assertEqual(ms.usable([1, "x"]), ["1", "x"])


if __name__ == "__main__":
    unittest.main()
