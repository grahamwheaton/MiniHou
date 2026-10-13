import unittest
from unittest.mock import patch
import numpy as np
from minihou.geometry import Geometry
from minihou.nodes.primitives import box
from minihou.nodes.copy_to_points import copy_to_points


class CopyMemoryTests(unittest.TestCase):
    def test_oversized_copies_rejected_before_position_allocation(self):
        from minihou import Evaluator, Graph
        source = Evaluator().cook(Graph.from_dict({'nodes': {'b': {'type': 'sphere'}}, 'display': 'b'}))
        points = Geometry.points_only(np.zeros((500000, 3), np.float32))
        with patch('minihou.nodes.copy_to_points._copy_positions_numba', side_effect=AssertionError('allocated before check')):
            with self.assertRaisesRegex(ValueError, 'safety limit'):
                copy_to_points(source, points, False, False)

    def test_small_copies_still_work(self):
        from minihou import Evaluator, Graph
        source = Evaluator().cook(Graph.from_dict({'nodes': {'b': {'type': 'box'}}, 'display': 'b'}))
        result = copy_to_points(source, Geometry.points_only(np.zeros((3, 3), np.float32)), False, False)
        self.assertEqual(result.npoints, source.npoints * 3)
