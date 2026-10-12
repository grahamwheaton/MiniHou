import os
import tempfile
import unittest
from unittest.mock import patch
from minihou.link import discovery

class DiscoveryTests(unittest.TestCase):
    def test_local_records_newest_first_and_bad_files_ignored(self):
        with tempfile.TemporaryDirectory() as root, patch.dict(os.environ, MINIHOU_HOME=root):
            first = discovery.publish("127.0.0.1", 1234, "http://127.0.0.1:1235/")
            second = discovery.publish("127.0.0.1", 4321, "http://127.0.0.1:4322/")
            (discovery.directory() / "broken.json").write_text("broken", encoding="utf-8")
            self.assertIsNone(discovery.publish("example.com", 9999, ""))
            self.assertEqual(discovery.candidates(), [("127.0.0.1",4321),("127.0.0.1",1234)])
            second.unlink()
            self.assertEqual(discovery.candidates(), [("127.0.0.1",1234)])
