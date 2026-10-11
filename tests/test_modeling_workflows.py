import pathlib
import unittest
from minihou import Evaluator,Graph
from minihou.sdk import REGISTRY

ROOT=pathlib.Path(__file__).resolve().parents[1]
NODES=['edge_group','edge_group_promote','polybridge','dissolve','skin','polyexpand2d',
       'soft_transform','lattice_grid','lattice','point_deform','crease']


class ModelingWorkflowTests(unittest.TestCase):
    def test_registry_editor_descriptions_and_serialized_workflows(self):
        for name in NODES:
            desc=REGISTRY.get(name).describe()
            self.assertTrue(desc['doc'])
            self.assertIn(desc['category'],['group','model','deform','source'])
            self.assertTrue(desc['params'])
        paths=list((ROOT/'examples'/'graphs').glob('modeling_*.json'))
        self.assertEqual(len(paths),8)
        for path in paths:
            with self.subTest(workflow=path.name):
                graph=Graph.load(path)
                graph=Graph.from_json(graph.to_json())
                geo=Evaluator().cook(graph)
                geo.validate()
                self.assertGreater(geo.nprims,0)
                self.assertTrue(graph.interface())
