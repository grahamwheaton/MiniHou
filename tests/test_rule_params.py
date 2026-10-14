"""Repeatable row schema persistence and custom SDK compatibility."""
import unittest
from minihou import Graph, ParamError, REGISTRY
from minihou.sdk import Rules, Str, Bool, Enum, Registry, node

class RuleParamTests(unittest.TestCase):
    def test_nested_coercion_legacy_json_and_validation(self):
        p=Rules('rules',[Bool('enabled',True),Str('name','g'),Rules('operations',[Enum('op',['union','subtract']),Str('group','')])])
        value=p.coerce('[{"name":"a","operations":[{"group":"b"}]}]')
        self.assertEqual(value,[{'enabled':True,'name':'a','operations':[{'op':'union','group':'b'}]}])
        for bad in ('{}',[{'typo':1}],[{'operations':[{'op':'wrong'}]}],[3]):
            with self.assertRaises(ParamError):p.coerce(bad)
        self.assertEqual(p.describe()['fields'][2]['kind'],'rules')
    def test_graph_copy_serialization_and_default_isolation(self):
        g=Graph.from_dict({'nodes':{'b':{'type':'box'},'c':{'type':'group_combine','inputs':{'geo':'b'},'params':{'rules':[{'domain':'points','name':'all','base':'*'}]}}}})
        c=g.copy();c.nodes['c'].params['rules'][0]['name']='other'
        self.assertEqual(g.nodes['c'].params['rules'][0]['name'],'all')
        d=g.to_dict();d['nodes']['c']['params']['rules'][0]['name']='exported'
        self.assertEqual(g.nodes['c'].params['rules'][0]['name'],'all')
        self.assertEqual(Graph.from_dict(g.to_dict()).to_dict(),g.to_dict())
        defaults=REGISTRY.get('group_combine').default_params();defaults['rules'].append({'name':'x'})
        self.assertEqual(REGISTRY.get('group_combine').default_params()['rules'],[])
    def test_custom_node_rows_are_optional_and_described(self):
        reg=Registry()
        @node('custom',params=[Str('text','hello'),Rules('rows',[Str('item','a')])],registry=reg)
        def custom(text,rows):return None
        self.assertEqual(reg.get('custom').coerce_params({})['text'],'hello')
        self.assertEqual(reg.get('custom').describe()['params'][1]['fields'][0]['name'],'item')

    def test_expression_rows_follow_frame_and_cache_keys(self):
        from minihou import Evaluator
        from minihou import timectx
        g=Graph.from_dict({'nodes':{'b':{'type':'box'},'e':{'type':'group_expression','inputs':{'geo':'b'},'params':{'rules':[{'name':'time','expression':'@Frame > 2'}]}}},'display':'e'})
        ev=Evaluator()
        with timectx.at(1,24):self.assertFalse(ev.cook(g).point['group_time'].any())
        with timectx.at(3,24):self.assertTrue(ev.cook(g).point['group_time'].all())
        self.assertFalse(timectx.params_use_time({'rules':[{'expression':'@P.z > 0'}]}))
