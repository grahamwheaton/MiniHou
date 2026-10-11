"""Every example graph must load, validate and cook (keeps examples from rotting as nodes evolve)."""
import unittest
from pathlib import Path

from minihou import Evaluator, Graph
from minihou.library import NodeLibrary

ROOT = Path(__file__).resolve().parent.parent
EXAMPLES = sorted((ROOT / "examples" / "graphs").glob("*.json"))


class ExampleGraphTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.lib = NodeLibrary([ROOT / "nodes"])                     # examples may use the example node library
        cls.lib.refresh()
        assert all(f["error"] is None for f in cls.lib.status()["files"]), cls.lib.status()

    @classmethod
    def tearDownClass(cls):
        cls.lib.unload()

    def test_examples_exist(self):
        self.assertGreaterEqual(len(EXAMPLES), 3)

    def test_each_example_cooks(self):
        for path in EXAMPLES:
            with self.subTest(example=path.name):
                g = Graph.load(path)
                geo = Evaluator().cook(g)
                geo.validate()
                self.assertGreater(geo.npoints, 0)
                self.assertGreater(geo.nprims, 0)
                self.assertTrue(g.interface(), "examples should expose at least one parameter")
                Graph.from_json(g.to_json())                      # round-trips


if __name__ == "__main__":
    unittest.main()
