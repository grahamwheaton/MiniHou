"""Packaged smoke diagnostics report successful kernel calls without changing results."""
import tempfile
import unittest
from unittest.mock import patch
from minihou import accel, Graph
from minihou.link.session import Session
from minihou.web import WebServer
class AccelDiagnostics(unittest.TestCase):
 def test_success_counts_and_snapshot_isolation(self):
  def diagnostic_kernel(x):return x+1
  before=accel.diagnostics()['successful_calls'].get('diagnostic_kernel',0)
  with patch.object(accel,'AVAILABLE',True),patch.object(accel,'_disabled_reason',None),patch.object(accel,'_forced_off',0):
   self.assertEqual(accel.call(diagnostic_kernel,2),3)
  snapshot=accel.diagnostics();self.assertEqual(snapshot['successful_calls']['diagnostic_kernel'],before+1)
  snapshot['successful_calls']['diagnostic_kernel']=0
  self.assertEqual(accel.diagnostics()['successful_calls']['diagnostic_kernel'],before+1)
 def test_http_state_contains_backend_and_source_identity(self):
  with tempfile.TemporaryDirectory() as root:
   session=Session(Graph.from_dict({'nodes':{'b':{'type':'box'}},'display':'b'}))
   self.addCleanup(session.stop)
   state=WebServer(session,port=0,root=root).state()
   self.assertIn('acceleration',state['engine'])
   self.assertEqual(state['engine']['frozen'],False)
   self.assertIn('successful_calls',state['engine']['acceleration'])
