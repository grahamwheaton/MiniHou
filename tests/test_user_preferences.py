import os
import tempfile
import unittest
from unittest.mock import patch
from pathlib import Path
from minihou import user_preferences
from minihou.nodes import python_node

class PreferencesTests(unittest.TestCase):
    def test_default_persistence_and_cli_override(self):
        with tempfile.TemporaryDirectory() as tmp, patch.dict(os.environ, {"MINIHOU_PREFERENCES_FILE":str(Path(tmp)/"prefs.json")}):
            previous=python_node._ENABLED
            self.addCleanup(python_node.set_allowed, previous)
            python_node.set_allowed(None)
            with patch.dict(os.environ):
                os.environ.pop("MINIHOU_PYTHON_NODES",None)
                self.assertFalse(python_node.allowed())
                user_preferences.set_python(True)
                self.assertTrue(python_node.allowed())
                user_preferences.set_python(False)
                self.assertFalse(python_node.allowed())
                python_node.set_allowed(True)
                self.assertTrue(python_node.allowed())
                python_node.set_allowed(None)
                os.environ["MINIHOU_PYTHON_NODES"]="1"
                self.assertTrue(python_node.allowed())
            with self.assertRaises(ValueError):user_preferences.set_python("true")
