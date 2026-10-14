"""Stream splicing preserves exact wires, graph identity and undo state."""
import unittest
from minihou.edit import apply_ops, EditError
from minihou.link.session import Session
from tests.helpers import graph_from


def fixture():
    g = graph_from({
        'a': {'type': 'box'}, 'b': {'type': 'grid'},
        'n': {'type': 'transform', 'inputs': {'geo': 'a'}, 'comment': 'keep'},
        'o': {'type': 'output', 'inputs': {'geo': 'n'}},
        'm': {'type': 'merge', 'inputs': {'geos': ['b', 'n', 'a', 'n']}},
    }, display='o')
    g.add_node('loose', 'subdivide')
    return g


class SpliceTests(unittest.TestCase):
    def test_detach_preserves_fanout_order_duplicates_metadata(self):
        g = fixture()
        g.render = 'n'
        out, result = apply_ops(g, [{'op': 'detach', 'id': 'n'}])
        self.assertEqual(out.nodes['o'].inputs, {'geo': 'a'})
        self.assertEqual(out.nodes['m'].inputs['geos'], ['b', 'a', 'a', 'a'])
        self.assertEqual(out.nodes['n'].inputs, {})
        self.assertEqual(out.nodes['n'].meta['comment'], 'keep')
        self.assertEqual((out.display, out.render), ('o', 'n'))
        self.assertEqual(result[0]['rewired'], 3)
        self.assertEqual(g.nodes['o'].inputs, {'geo': 'n'})

    def test_detach_ambiguous_inputs_atomic_even_with_move(self):
        g = fixture()
        before = g.to_dict()
        with self.assertRaisesRegex(EditError, 'Cannot detach safely'):
            apply_ops(g, [{'op': 'move', 'id': 'm', 'pos': [999, 999]}, {'op': 'detach', 'id': 'm'}])
        self.assertEqual(g.to_dict(), before)
        with self.assertRaises(EditError):
            apply_ops(g, [{'op': 'detach', 'id': 'a'}])

    def test_detach_single_connected_multi_input(self):
        g = fixture()
        g.nodes['m'].inputs['geos'] = ['b']
        g.nodes['o'].inputs['geo'] = 'm'
        out, _ = apply_ops(g, [{'op': 'detach', 'id': 'm'}])
        self.assertEqual(out.nodes['o'].inputs['geo'], 'b')
        self.assertEqual(out.nodes['m'].inputs, {})

    def test_insert_exact_occurrence_and_named_source(self):
        g = fixture()
        g.add_node('source_object', 'object_input', input_owner='a', input_label='Scatter')
        g.nodes['m'].inputs['geos'] = ['source_object', 'b', 'source_object']
        out, _ = apply_ops(g, [{'op': 'insert_existing', 'id': 'loose', 'from': 'source_object',
                               'to': 'm', 'port': 'geos', 'index': 2}])
        self.assertEqual(out.nodes['m'].inputs['geos'], ['source_object', 'b', 'loose'])
        self.assertEqual(out.nodes['loose'].inputs, {'geo': 'source_object'})
        self.assertEqual(out.nodes['o'].inputs, {'geo': 'n'})

    def test_insert_stale_wrong_index_and_not_loose_rejected(self):
        for extra in ({}, {'index': 0}, {'index': True}, {'index': -1}):
            with self.subTest(extra=extra), self.assertRaises(EditError):
                apply_ops(fixture(), [{'op': 'insert_existing', 'id': 'loose', 'from': 'n',
                                      'to': 'm', 'port': 'geos', **extra}])
        for node in ('n', 'a'):
            with self.assertRaises(EditError):
                apply_ops(fixture(), [{'op': 'insert_existing', 'id': node, 'from': 'n', 'to': 'o', 'port': 'geo'}])

    def test_required_secondary_input_rejected(self):
        g = fixture()
        g.add_node('boolean_node', 'boolean')
        with self.assertRaises(EditError):
            apply_ops(g, [{'op': 'insert_existing', 'id': 'boolean_node', 'from': 'n',
                           'to': 'o', 'port': 'geo', 'input': 'a'}])

    def test_port_types_and_optional_input_choice(self):
        from dataclasses import replace
        from minihou.sdk import Registry, Input, Output
        g = fixture()
        registry = Registry()
        for node_type in g.registry:
            registry.register(node_type)
        g.registry = registry
        base = registry.get('transform')
        registry.register(replace(base, name='typed', inputs=(Input('geo'),), outputs=(Output(type='other'),)))
        g.add_node('typed', 'typed')
        op = {'op': 'insert_existing', 'id': 'typed', 'from': 'n', 'to': 'o', 'port': 'geo'}
        with self.assertRaisesRegex(EditError, 'incompatible'):
            apply_ops(g, [op])
        registry.register(replace(base, name='choices', inputs=(Input('left', optional=True), Input('right', optional=True))))
        g.add_node('choices', 'choices')
        op['id'] = 'choices'
        with self.assertRaisesRegex(EditError, 'ambiguous'):
            apply_ops(g, [op])
        out, _ = apply_ops(g, [{**op, 'input': 'right'}])
        self.assertEqual(out.nodes['choices'].inputs, {'right': 'n'})
        g.nodes['n'].inputs['geo'] = 'typed'
        with self.assertRaisesRegex(EditError, 'incompatible'):
            apply_ops(g, [{'op': 'detach', 'id': 'n'}])

    def test_session_undo_redo_one_batch_and_roundtrip(self):
        session = Session(fixture())
        before = session.graph.to_dict()
        session.edit([{'op': 'detach', 'id': 'n'}, {'op': 'move', 'id': 'n', 'pos': [400, 200]}])
        after = session.graph.to_dict()
        session.undo()
        self.assertEqual(session.graph.to_dict(), before)
        session.redo()
        self.assertEqual(session.graph.to_dict(), after)
        session.edit([{'op': 'insert_existing', 'id': 'n', 'from': 'a', 'to': 'o', 'port': 'geo'}])
        self.assertEqual(session.graph.nodes['o'].inputs['geo'], 'n')
        session.undo()
        self.assertEqual(session.graph.to_dict(), after)


if __name__ == '__main__':
    unittest.main()

