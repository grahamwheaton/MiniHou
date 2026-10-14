"""The explicit spatial starter is three dimensional while implicit legacy rules stay planar."""
import unittest
from pathlib import Path
import numpy as np
from minihou import Evaluator, Graph, REGISTRY

class LSystemStarterTests(unittest.TestCase):
    def test_explicit_starter_is_spatial_and_deterministic(self):
        graph=Graph.load(Path(__file__).resolve().parents[1]/'examples/lsystem_3d.json')
        a=Evaluator().cook(graph);b=Evaluator().cook(graph)
        np.testing.assert_array_equal(a.P,b.P)
        self.assertEqual(np.linalg.matrix_rank(a.P-a.P.mean(axis=0)),3)
        self.assertGreater(float(np.ptp(a.P,axis=0).min()),.1)
        self.assertLess(a.npoints,10000)

    def test_implicit_saved_graph_still_uses_legacy_planar_grammar(self):
        graph=Graph.from_dict({'nodes':{'grow':{'type':'lsystem'}},'display':'grow'})
        geo=Evaluator().cook(graph)
        np.testing.assert_array_equal(geo.P[:,1],0)
        self.assertEqual(REGISTRY.get('lsystem').param('rules').default,'F=FF-[-F+F+F]+[+F-F-F]')
