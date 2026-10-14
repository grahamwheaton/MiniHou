"""CLI cooking preserves unfinished editor branches and validates requested outputs."""
import json,subprocess,sys,tempfile,unittest
from pathlib import Path
from minihou import Graph,GraphError
ROOT=Path(__file__).resolve().parents[1]
class SavedGraphCook(unittest.TestCase):
 def setUp(self):
  self.tmp=tempfile.TemporaryDirectory();self.addCleanup(self.tmp.cleanup)
  self.path=Path(self.tmp.name)/'saved.json'
  self.path.write_text(json.dumps({'nodes':{'box':{'type':'box'},'loose':{'type':'attribute_transfer'}},'display':'box'}),encoding='utf-8')
  self.original=self.path.read_bytes()
 def command(self,*extra):
  return subprocess.run([sys.executable,str(ROOT/'tools/worktree_cli.py'),'cook',str(self.path),*extra],cwd=ROOT,capture_output=True,text=True,encoding='utf-8')
 def test_cli_cooks_display_without_rewriting_unfinished_branch(self):
  with self.assertRaises(GraphError):Graph.load(self.path)
  result=self.command();self.assertEqual(result.returncode,0,result.stderr)
  self.assertIn('cook 1:',result.stdout);self.assertEqual(self.path.read_bytes(),self.original)
 def test_cli_reports_missing_inputs_when_unfinished_node_is_requested(self):
  result=self.command('--node','loose');self.assertEqual(result.returncode,2,result.stderr)
  self.assertIn("input 'geo' is not connected",result.stderr)
  self.assertNotIn('Traceback',result.stderr);self.assertEqual(self.path.read_bytes(),self.original)
