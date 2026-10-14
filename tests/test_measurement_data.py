"""Graph annotation persistence and display unit invariants."""
import copy
import os
import tempfile
import unittest
from unittest.mock import patch
from minihou import Graph
from minihou.edit import apply_ops, EditError
from minihou.link.session import Session
from minihou import user_preferences

class MeasurementDataTests(unittest.TestCase):
    def test_graph_roundtrip_identity_undo_delete(self):
        graph=Graph.from_dict({'nodes':{'box':{'type':'box'}}})
        session=Session(graph)
        self.addCleanup(session.stop)
        item={'id':'ruler','points':[[0,0,0],[1,0,0]],'refs':[{'node':'box','topology':'mesh','indices':[0,1],'weights':[.25,.75]},None],'label':'Width','label_offset':[12,-20]}
        session.edit([{'op':'measurements','values':[item]}])
        identity=session.graph.measurements[0]['refs'][0]['identity']
        self.assertEqual(session.graph.nodes['box'].meta['measurement_identity'],identity)
        restored=Graph.from_dict(session.graph.to_dict())
        self.assertEqual(restored.measurements,session.graph.measurements)
        copied=restored.copy();copied.measurements[0]['label']='Changed'
        self.assertEqual(restored.measurements[0]['label'],'Width')
        session.edit([{'op':'measurements','values':[]}])
        self.assertEqual(session.graph.measurements,[])
        session.undo();self.assertEqual(session.graph.measurements[0]['label'],'Width')
        session.redo();self.assertEqual(session.graph.measurements,[])
        session.undo();session.edit([{'op':'remove','id':'box'}])
        self.assertEqual(session.graph.measurements[0]['refs'][0]['identity'],identity)
        self.assertNotIn('box',session.graph.nodes)
        session.undo();self.assertEqual(session.graph.nodes['box'].meta['measurement_identity'],identity)

    def test_invalid_measurements_are_atomic(self):
        graph=Graph.from_dict({'nodes':{}})
        valid={'id':'a','points':[[0,0,0],[1,1,1]]}
        for value in ([valid,valid],[dict(valid,points=[[float('nan'),0,0],[0,0,0]])],[dict(valid,refs=[{'node':'missing','topology':'x','indices':[0],'weights':[2]},None])]):
            with self.assertRaises(EditError):apply_ops(graph,[{'op':'measurements','values':value}])
        self.assertEqual(graph.measurements,[])

    def test_units_preferences_do_not_rescale_graph(self):
        with tempfile.TemporaryDirectory() as root,patch.dict(os.environ,{'MINIHOU_PREFERENCES_FILE':os.path.join(root,'prefs.json')}):
            graph=Graph.from_dict({'nodes':{'box':{'type':'box','params':{'size':[1,2,3]}}}})
            original=copy.deepcopy(graph.to_dict())
            self.assertEqual(user_preferences.read()['length_unit'],'mm')
            self.assertEqual(user_preferences.read()['unit_scale'],1)
            user_preferences.update({'length_unit':'cm','unit_scale':.001,'isolated_knockback':.5})
            self.assertEqual(user_preferences.read()['unit_scale'],.001)
            self.assertEqual(graph.to_dict(),original)
            for value in (0,-1,float('nan'),True):
                with self.assertRaises(ValueError):user_preferences.update({'unit_scale':value})
            with self.assertRaises(ValueError):user_preferences.update({'length_unit':'parsec'})
