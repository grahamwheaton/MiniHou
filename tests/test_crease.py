import unittest
import numpy as np
from minihou import edges,topo
from tests.test_edge_tools import run


class CreaseTests(unittest.TestCase):
    def test_full_creases_pin_box_corners_and_decay(self):
        box=run('box')
        box=run('edge_group',box,name='original')
        box=run('crease',box,sharpness=2)
        box.freeze()
        out=run('subdivide',box)
        np.testing.assert_allclose(out.P[:box.npoints],box.P)
        self.assertEqual(edges.mask(out,'original').sum(),24)
        weights=edges.values(out,'creaseweight')
        self.assertEqual(np.count_nonzero(weights),24)
        self.assertEqual(weights.max(),1)
        twice=run('subdivide',out)
        self.assertFalse(edges.values(twice,'creaseweight').any())
        self.assertTrue(np.all(topo.edges(twice).count==2))
        self.assertEqual(edges.values(box,'creaseweight').max(),2)

    def test_fractional_and_disabled_creases(self):
        box=run('box')
        smooth=run('subdivide',box)
        half=run('subdivide',run('crease',box,sharpness=.5))
        sharp=run('subdivide',run('crease',box,sharpness=2))
        self.assertGreater(np.linalg.norm(half.P[:8]),np.linalg.norm(smooth.P[:8]))
        self.assertLess(np.linalg.norm(half.P[:8]),np.linalg.norm(sharp.P[:8]))
        disabled=run('subdivide',run('crease',box),use_creases=False)
        np.testing.assert_allclose(disabled.P,smooth.P)

    def test_cornerweight_and_uvs(self):
        box=run('box')
        box.vertex['uv']=box.P[box.topology.vertex_point,:2]
        creased=run('crease',box,domain='points',group='0',sharpness=2)
        out=run('subdivide',creased)
        np.testing.assert_allclose(out.P[0],box.P[0])
        self.assertEqual(out.point['cornerweight'][0],1)
        self.assertEqual(np.count_nonzero(out.point['cornerweight']),1)
        self.assertIn('uv',out.vertex if 'uv' in box.vertex else out.point)
        with self.assertRaises(ValueError):
            run('crease',box,sharpness=-1)


if __name__=='__main__':
    unittest.main()
