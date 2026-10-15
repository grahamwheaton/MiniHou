"""The native app's parameter metadata (kinds, ranges, options) must match the Python node registry it was generated from."""
import importlib.util
import json
import os
import unittest

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SCRIPT = os.path.join(ROOT, "tools", "dump_rust_param_meta.py")
FILE = os.path.join(ROOT, "rust", "crates", "minihou-app", "src", "param_meta.json")


class ParamMetaTests(unittest.TestCase):
    @unittest.skipUnless(os.path.exists(FILE), "the native app is not in this checkout")
    def test_the_file_matches_the_registry(self):
        spec = importlib.util.spec_from_file_location("dump_rust_param_meta", SCRIPT)
        mod = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(mod)
        with open(FILE, encoding="utf-8") as f:
            on_disk = json.load(f)
        self.assertEqual(json.loads(json.dumps(mod.meta())), on_disk,
                         "rust/crates/minihou-app/src/param_meta.json is out of date: run `py tools/dump_rust_param_meta.py`")


if __name__ == "__main__":
    unittest.main()
