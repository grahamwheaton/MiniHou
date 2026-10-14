import tempfile
from pathlib import Path
import unittest
from unittest.mock import patch
from minihou.nodes import font
class FontDiscovery(unittest.TestCase):
 def setUp(self):font._font_path_cache.clear()
 def tearDown(self):font._font_path_cache.clear()
 def test_nested_linux_default_fallback_and_explicit_missing(self):
  with tempfile.TemporaryDirectory() as d:
   nested=Path(d)/'truetype'/'dejavu';nested.mkdir(parents=True)
   face=nested/'DejaVuSans.ttf';face.write_bytes(b'font fixture only discovery')
   with patch.object(font,'_candidate_font_dirs',return_value=[d]):
    self.assertEqual(font._find_font('Lato Bold'),str(face))
    self.assertEqual(font._find_font('DejaVu Sans'),str(face))
    self.assertIsNone(font._find_font('Definitely Missing Family'))
    self.assertIsNone(font._find_font(str(Path(d)/'missing.ttf')))
