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
    def test_viewport_transform_follows_endpoints_with_undo_redo_and_reload(self):
        from minihou import Evaluator
        from minihou.measurements import resolve_session
        graph = Graph.from_dict({'nodes': {'box': {'type': 'box'}}, 'display': 'box'})
        geo = Evaluator().cook(graph)
        item = {'id': 'edge', 'points': geo.P[:2].tolist(), 'refs': [
            {'node': 'box', 'topology': f'{geo.npoints*3}:{geo.topology.digest}',
             'indices': [i], 'weights': [1]} for i in range(2)]}
        session = Session(graph)
        self.addCleanup(session.stop)
        session.edit([{'op': 'measurements', 'values': [item]}])
        before = session.graph.to_dict()
        session.edit([{'op': 'insert', 'after': 'box', 'type': 'transform', 'id': 'moved',
                       'follow_measurements': True, 'params': {'group': '0', 'translate': [3, 0, 0]}}])
        values = resolve_session(session)['measurements']['edge']
        self.assertAlmostEqual(values[0]['point'][0], geo.P[0, 0] + 3)
        self.assertEqual(values[1]['point'], geo.P[1].tolist())
        self.assertTrue(all(p['status'].startswith('Attached') for p in values))
        moved = session.graph.to_dict()
        session.undo(); self.assertEqual(session.graph.to_dict(), before)
        session.redo(); self.assertEqual(session.graph.to_dict(), moved)
        loaded = Session(Graph.from_dict(moved))
        self.addCleanup(loaded.stop)
        self.assertEqual(resolve_session(loaded)['measurements']['edge'], values)

    def test_combining_live_input_and_point_preserves_attachment_indices(self):
        from minihou import Evaluator
        from minihou.measurements import edit_context
        base = Graph.from_dict({'nodes': {'box': {'type': 'box'}}})
        geo = Evaluator().cook(base, 'box')
        graph = Graph.from_dict({'nodes': {'i': {'type': 'modifier_input'},
            'p': {'type': 'point', 'params': {'center': [4, 0, 0]}, 'display_extra': True}}, 'display': 'i'})
        point = Evaluator().cook(graph, 'p')
        item = {'id': 'gap', 'points': [geo.P[0].tolist(), [4, 0, 0]], 'refs': [
            {'node': node, 'topology': f'{g.npoints*3}:{g.topology.digest}', 'indices': [0], 'weights': [1]}
            for node, g in [('i', geo), ('p', point)]]}
        graph, _ = apply_ops(graph, [{'op': 'measurements', 'values': [item]}])
        with edit_context({'i': ('prefix', geo)}, 12, 24):
            merged, result = apply_ops(graph, [{'op': 'merge_templates', 'id': 'i', 'follow_measurements': True}])
        refs = merged.measurements[0]['refs']
        self.assertEqual([r['indices'] for r in refs], [[0], [geo.npoints]])
        self.assertTrue(all(r['node'] == result[0]['id'] for r in refs))
        cooked = Evaluator().cook(merged, feeds={'i': ('prefix', geo)})
        self.assertEqual(refs[0]['topology'], f'{cooked.npoints*3}:{cooked.topology.digest}')
        self.assertEqual(cooked.P[refs[1]['indices'][0]].tolist(), [4, 0, 0])

    def test_flip_then_move_follows_but_topology_changes_stay_detached(self):
        from minihou import Evaluator
        from minihou.measurements import resolve_session
        graph = Graph.from_dict({'nodes': {'box': {'type': 'box'}}, 'display': 'box'})
        geo = Evaluator().cook(graph)
        session = Session(graph)
        self.addCleanup(session.stop)
        session.edit([{'op': 'measurements', 'values': [{'id': 'v',
            'points': [geo.P[0].tolist(), geo.P[1].tolist()], 'refs': [
                {'node': 'box', 'topology': f'{geo.npoints*3}:{geo.topology.digest}',
                 'indices': [0], 'weights': [1]}, None]}]}])
        session.edit([{'op': 'insert', 'after': 'box', 'type': 'reverse', 'id': 'flipped',
                       'follow_measurements': True, 'params': {'group': '0'}}])
        self.assertTrue(resolve_session(session)['measurements']['v'][0]['status'].startswith('Attached'))
        session.edit([{'op': 'insert', 'after': 'flipped', 'type': 'transform', 'id': 'moved',
                       'follow_measurements': True, 'params': {'translate': [2, 0, 0]}}])
        self.assertAlmostEqual(resolve_session(session)['measurements']['v'][0]['point'][0], geo.P[0, 0]+2)
        session.edit([{'op': 'insert', 'after': 'moved', 'type': 'inset', 'id': 'inset',
                       'follow_measurements': True, 'params': {'group': '0', 'amount': .1}}])
        self.assertTrue(resolve_session(session)['measurements']['v'][0]['status'].startswith('Detached'))
        session.edit([{'op': 'insert', 'after': 'inset', 'type': 'transform', 'id': 'again',
                       'follow_measurements': True, 'params': {'translate': [1, 0, 0]}}])
        self.assertTrue(resolve_session(session)['measurements']['v'][0]['status'].startswith('Detached'))

    def test_graph_roundtrip_identity_undo_delete(self):
        graph=Graph.from_dict({'nodes':{'box':{'type':'box'}}})
        session=Session(graph)
        self.addCleanup(session.stop)
        item={'id':'ruler','points':[[0,0,0],[1,0,0]],'refs':[{'node':'box','topology':'mesh','indices':[0,1],'weights':[.25,.75]},None],'label':'Width','label_offset':[12,-20]}
        session.edit([{'op':'measurements','values':[item]}])
        identity=session.graph.measurements[0]['refs'][0]['identity']
        self.assertEqual(session.graph.nodes['box'].meta['measurement_identity'],identity)
        session.edit([{'op':'rename','id':'box','to':'renamed'}])
        self.assertEqual(session.graph.measurements[0]['refs'][0]['node'],'renamed')
        session.undo()
        self.assertEqual(session.graph.measurements[0]['refs'][0]['node'],'box')
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
