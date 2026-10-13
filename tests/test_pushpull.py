import unittest
import numpy as np
from tests.test_nodes import run
from tests.test_boolean import volume, crossings
from minihou.nodes.bevel import twins
from minihou import handles


class PushPullTests(unittest.TestCase):
    def solid(self,g):
        g.validate()
        self.assertTrue(np.all(twins(g)>=0))
        self.assertEqual(crossings(g),0)
        self.assertTrue(np.isfinite(g.P).all())
        self.assertGreaterEqual(volume(g),0)

    def test_positive_negative_and_complete_removal(self):
        box=run('box',size=[2,2,2])
        original=box.P.copy()
        for distance,expected in ((.4,9.6),(-.4,6.4),(-3,0)):
            g=run('pushpull',box,group='1',distance=distance)
            self.solid(g)
            self.assertAlmostEqual(volume(g),expected,delta=1e-3)
        np.testing.assert_array_equal(box.P,original)

    def test_pocket_and_through_hole_are_watertight(self):
        box=run('box',size=[2,2,2])
        inset=run('inset',box,group='1',amount=.4)
        self.solid(inset)
        area=(2-2*.4/np.sqrt(2))**2
        for distance,expected,euler in ((-.4,8-area*.4,2),(-3,8-area*2,0)):
            g=run('pushpull',inset,group='extrudeFront',distance=distance)
            self.solid(g)
            self.assertAlmostEqual(volume(g),expected,delta=1e-3)
            # Euler characteristic 0 proves a through tunnel instead of a capped pocket.
            self.assertEqual(g.npoints-g.nvertices//2+g.nprims,euler)

    def test_all_box_faces_and_rotated_solid(self):
        box=run('box',size=[2,2,2])
        box=run('transform',box,rotate=[23,31,17],translate=[2,-3,1])
        for face in range(6):
            g=run('pushpull',box,group=str(face),distance=-.25)
            self.solid(g)
            self.assertAlmostEqual(volume(g),7,delta=2e-3)

    def test_reject_open_or_multiple_faces_and_zero_noop(self):
        from minihou.nodes.pushpull import pushpull
        box=run('box')
        for geo,group in ((run('grid'),'0'),(box,''),(box,'0 1')):
            with self.assertRaisesRegex(ValueError,'closed|exactly one'):
                pushpull(geo,group,.2)
        np.testing.assert_array_equal(pushpull(box,'0',0).P,box.P)

    def test_inset_outset_have_opposite_amount_handles(self):
        box=run('box')
        inside=run('inset',box,group='1',amount=.1)
        outside=run('outset',box,group='1',amount=.1)
        self.assertEqual(inside.nprims,outside.nprims)
        a=handles.handles_for('inset',[box],inside,{'group':'1'})[0]
        b=handles.handles_for('outset',[box],outside,{'group':'1'})[0]
        self.assertEqual(a['param'],'amount')
        np.testing.assert_allclose(a['dir'],-np.asarray(b['dir']))

    def test_invalid_boolean_result_is_rejected(self):
        from unittest.mock import patch
        with patch('minihou.nodes.pushpull.boolean', return_value=run('grid')) as solve:
            with self.assertRaisesRegex(ValueError, 'could not produce a watertight solid'):
                run('pushpull',run('box'),group='1',distance=.2)
            self.assertEqual(solve.call_count,4)

    def test_shipped_hole_example(self):
        from pathlib import Path
        from minihou import Graph, Evaluator
        graph=Graph.load(Path(__file__).resolve().parents[1]/'examples/graphs/pushpull_hole.json')
        evaluator=Evaluator()
        self.addCleanup(evaluator.close)
        self.solid(evaluator.cook(graph,graph.display))
