"""Desktop discovery and safe window lifecycle without requiring a GUI in CI."""
import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch, MagicMock
from minihou import desktop

class DesktopTests(unittest.TestCase):
    def test_discovery_ignores_foreign_urls_and_corrupt_records(self):
        with tempfile.TemporaryDirectory() as folder:
            root=Path(folder)
            (root/'a.json').write_text('{',encoding='utf-8')
            (root/'b.json').write_text(json.dumps({'editor_url':'https://example.com/'}),encoding='utf-8')
            with patch('minihou.link.discovery.directory',return_value=root), patch.object(desktop,'urlopen') as opened:
                self.assertFalse(desktop.reuse(root))
                opened.assert_not_called()

    def test_matching_workspace_reuses_local_window(self):
        with tempfile.TemporaryDirectory() as folder:
            root=Path(folder)
            (root/'a.json').write_text(json.dumps({'editor_url':'http://127.0.0.1:1234/'}),encoding='utf-8')
            response=MagicMock()
            response.__enter__.return_value.read.return_value=b'{"desktop":true}'
            with patch('minihou.link.discovery.directory',return_value=root), patch.object(desktop,'urlopen',return_value=response) as opened:
                self.assertTrue(desktop.reuse(root))
                request=opened.call_args.args[0]
                self.assertEqual(json.loads(request.data)['workspace'],str(root.resolve()))

if __name__=='__main__': unittest.main()
