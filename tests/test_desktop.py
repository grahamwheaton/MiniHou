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


class DesktopFileTests(unittest.TestCase):
    """The native dialog and the drop handler, with a stand-in for pywebview."""

    def test_the_picker_asks_the_window_for_an_open_or_save_dialog(self):
        import os
        import tempfile
        from types import SimpleNamespace
        calls = []

        class Window:
            def create_file_dialog(self, kind, **kw):
                calls.append((kind, kw))
                return ("C:/x/a.usd",) if kind == "open" else "C:/x/b.usdc"
        web = SimpleNamespace(FileDialog=SimpleNamespace(OPEN="open", SAVE="save"))
        pick = desktop.make_picker(Window(), web)
        with tempfile.TemporaryDirectory() as d:
            self.assertEqual(pick("open", [".usd", ".usda"], "Pick", d), {"path": "C:/x/a.usd"})
            self.assertEqual(calls[0][0], "open")
            self.assertEqual(calls[0][1]["directory"], d)
            self.assertEqual(calls[0][1]["file_types"][0], "Supported files (*.usd;*.usda)")
            self.assertEqual(pick("save", [".usdc"], "Save", os.path.join(d, "out.usdc")), {"path": "C:/x/b.usdc"})
            self.assertEqual(calls[1][1]["save_filename"], "out.usdc")
            self.assertEqual(calls[1][1]["directory"], d)
        web = SimpleNamespace(OPEN_DIALOG="o", SAVE_DIALOG="s")                       # an older pywebview
        Window.create_file_dialog = lambda self, kind, **kw: None                      # cancelled
        self.assertEqual(desktop.make_picker(Window(), web)("open", [], "", ""), {"path": ""})

    def test_dropped_files_with_real_paths_reach_the_page(self):
        sent = []
        window = type("W", (), {"evaluate_js": lambda self, js: sent.append(js)})()
        on_drop = desktop.drop_handler(window)
        on_drop({"dataTransfer": {"files": [{"name": "a.usd", "pywebviewFullPath": "C:/m/a.usd"}, {"name": "b"}]}, "clientX": 10, "clientY": 20})
        self.assertEqual(sent, ['window.minihou && window.minihou.droppedPaths(["C:/m/a.usd"], 10, 20)'])
        on_drop({"dataTransfer": {"files": [{"name": "a.usd"}]}})
        on_drop(None)
        self.assertEqual(len(sent), 1)

    def test_webview2_is_asked_for_the_fast_gpu_unless_told_not_to(self):
        self.assertEqual(desktop.browser_arguments({}), "--force_high_performance_gpu")
        self.assertEqual(desktop.browser_arguments({"WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS": "--foo"}),
                         "--foo --force_high_performance_gpu")
        already = {"WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS": "--force_high_performance_gpu --foo"}
        self.assertEqual(desktop.browser_arguments(already), already["WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS"])
        self.assertEqual(desktop.browser_arguments({"MINIHOU_GPU": "default", "WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS": "--foo"}),
                         "--foo")
