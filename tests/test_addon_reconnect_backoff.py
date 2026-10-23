"""Reconnect pacing for the Blender add-on (pure; no Blender needed)."""
import importlib.util
import unittest
from pathlib import Path

_PATH = Path(__file__).resolve().parents[1] / "blender_addon" / "minihou_link" / "discovery.py"
_spec = importlib.util.spec_from_file_location("minihou_addon_discovery", _PATH)
discovery = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(discovery)


class ReconnectBackoffTests(unittest.TestCase):
    def test_backoff_grows_from_two_to_thirty_seconds(self):
        delays, delay = [], 0.0
        for _ in range(7):
            delay = discovery.next_retry_delay(delay)
            delays.append(delay)
        self.assertEqual(delays, [2.0, 4.0, 8.0, 16.0, 30.0, 30.0, 30.0])

    def test_main_thread_attempt_budget_is_short(self):
        self.assertLessEqual(discovery.CONNECT_TIMEOUT, 0.2)
        self.assertLessEqual(discovery.CONNECT_BUDGET, 0.5)

    def test_rotation_reaches_every_candidate(self):
        choices = [("127.0.0.1", p) for p in (1, 2, 3)]
        firsts = {discovery.rotate(choices, i)[0] for i in range(3)}
        self.assertEqual(firsts, set(choices))
        self.assertEqual(discovery.rotate([], 5), [])


if __name__ == "__main__":
    unittest.main()
