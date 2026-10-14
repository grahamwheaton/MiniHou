"""Bundled helper delivery, source/frozen discovery, and user-node precedence."""
import json
import shutil
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch
from minihou import REGISTRY, Evaluator, Graph
from minihou import app
from minihou.library import NodeLibrary
from tools import build_app

ROOT=Path(__file__).resolve().parents[1]
HELPERS={'array','circle_array','grid_array','array_between_two_positions','array_linear_points','array_circle_points','array_grid_points','array_between_points'}

class BundledLibraryTests(unittest.TestCase):
    def test_source_discovery_and_same_directory_deduplication(self):
        with patch.object(sys,'frozen',False,create=True):
            self.assertEqual(app.bundled_nodes_source(),ROOT/'nodes')
            self.assertEqual(app.node_library_dirs(ROOT),[str((ROOT/'nodes').resolve())])

    def test_frozen_startup_loads_helpers_without_copying_user_nodes(self):
        with tempfile.TemporaryDirectory() as folder:
            base=Path(folder);bundle=base/'bundle';bundle.mkdir()
            shutil.copytree(ROOT/'nodes',bundle/'bundled_nodes')
            workspace=base/'old-workspace';user=workspace/'nodes';user.mkdir(parents=True)
            sentinel=user/'notes.txt';sentinel.write_text('keep',encoding='utf-8')
            with patch.object(sys,'frozen',True,create=True),patch.object(sys,'_MEIPASS',str(bundle),create=True):
                self.assertEqual(app.bundled_nodes_source(),bundle/'bundled_nodes')
                with patch.object(app,'examples_source',return_value=None):app.prepare_workspace(workspace,fresh=False)
                lib=NodeLibrary(app.node_library_dirs(workspace));self.addCleanup(lib.unload);lib.refresh()
                self.assertFalse([f for f in lib.status()['files'] if f['error']])
                self.assertTrue(HELPERS <= {nt.name for nt in REGISTRY})
                self.assertEqual([p.name for p in user.iterdir()],['notes.txt'])
                self.assertEqual(sentinel.read_text(encoding='utf-8'),'keep')
                geo=Evaluator().cook(Graph.from_dict({'nodes':{'a':{'type':'array','params':{'count':2}}},'display':'a'}))
                self.assertEqual(geo.npoints,16)

    def test_app_passes_both_libraries_to_startup(self):
        with tempfile.TemporaryDirectory() as folder:
            root=Path(folder)
            with patch('minihou.__main__._cmd_serve',return_value=0) as serve,patch.object(app,'_port_free',return_value=True),patch.object(app,'examples_source',return_value=None):
                self.assertEqual(app.main(['--workspace',str(root),'--no-browser','--port','0','--http-port','0']),0)
                ns=serve.call_args.args[0]
                self.assertEqual(ns.nodes,app.node_library_dirs(root))
                self.assertEqual(len(ns.nodes),2)

    def test_build_includes_read_only_library_data(self):
        with patch.object(build_app.subprocess,'run') as run:
            build_app.build(False)
        command=run.call_args.args[0]
        data=[command[i+1] for i,x in enumerate(command[:-1]) if x=='--add-data']
        self.assertTrue(any(str(ROOT/'nodes') in d and d.endswith('bundled_nodes') for d in data))

    def test_user_same_stem_wins_python_json_and_cross_format(self):
        with tempfile.TemporaryDirectory() as folder:
            root=Path(folder);user=root/'user';bundle=root/'bundle';user.mkdir();bundle.mkdir()
            for stem,user_suffix,bundle_suffix in [('delivery_py','.py','.py'),('delivery_json','.json','.json'),('delivery_cross','.json','.py')]:
                def write(directory,suffix,size):
                    path=directory/(stem+suffix)
                    if suffix=='.json':path.write_text(json.dumps({'nodes':{'b':{'type':'box','params':{'size':[size,size,size]}}},'display':'b'}),encoding='utf-8')
                    else:path.write_text('from minihou.sdk import node\nfrom minihou.nodes.primitives import box\n@node("'+stem+'", params=[])\ndef make():\n    return box(size=('+str(size)+',)*3)\n',encoding='utf-8')
                write(user,user_suffix,2);write(bundle,bundle_suffix,9)
            lib=NodeLibrary([user,bundle]);self.addCleanup(lib.unload);lib.refresh()
            self.assertFalse([f for f in lib.status()['files'] if f['error']])
            self.assertTrue(all(p.parent==user.resolve() for p in lib.scan()))
            for name in ['delivery_py','delivery_json','delivery_cross']:
                geo=Evaluator().cook(Graph.from_dict({'nodes':{'n':{'type':name}},'display':'n'}))
                self.assertAlmostEqual(float(geo.P[:,0].max()),1.)
            # Removing an override exposes the bundled file on the next refresh.
            (user/'delivery_json.json').unlink();lib.refresh()
            geo=Evaluator().cook(Graph.from_dict({'nodes':{'n':{'type':'delivery_json'}},'display':'n'}))
            self.assertAlmostEqual(float(geo.P[:,0].max()),4.5)

    def test_builtin_name_collision_still_reports_error(self):
        with tempfile.TemporaryDirectory() as folder:
            root=Path(folder);(root/'box.json').write_text(json.dumps({'nodes':{}}),encoding='utf-8')
            lib=NodeLibrary([root]);self.addCleanup(lib.unload);lib.refresh()
            self.assertIn('already exists',lib.status()['files'][0]['error'])
            self.assertEqual(REGISTRY.get('box').source,'')
