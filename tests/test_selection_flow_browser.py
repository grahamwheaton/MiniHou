"""Real WebGL viewport selection conversion and bounded ngon edge-flow regressions."""
from pathlib import Path
import math
import unittest
import numpy as np
from minihou import Geometry, preview
from tests.test_nodes import run
try:
    from playwright.sync_api import sync_playwright
except ImportError:
    sync_playwright=None

@unittest.skipIf(sync_playwright is None,'playwright unavailable')
class SelectionFlowBrowserTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.pw=sync_playwright().start()
        cls.browser=cls.pw.chromium.launch()
    @classmethod
    def tearDownClass(cls):
        cls.browser.close();cls.pw.stop()
    def setUp(self):
        self.page=self.browser.new_page()
        self.addCleanup(self.page.close)
        self.errors=[]
        self.page.on('pageerror',lambda e:self.errors.append(str(e)))
        self.page.set_content('<div style="position:relative;width:600px;height:500px"><canvas width="600" height="500" style="width:600px;height:500px" tabindex="0"></canvas></div>')
        root=Path(__file__).resolve().parents[1]/'minihou/web/static'
        for name in ['interaction.js','selection-tools.js','preview.js']:
            self.page.add_script_tag(path=str(root/name))
        self.page.evaluate("window.pv=MiniHouPreview.create(document.querySelector('canvas'),()=>{})")
        if not self.page.evaluate('pv.ok'):
            self.skipTest('WebGL2 unavailable')
    def tearDown(self):
        self.assertEqual(self.errors,[])
    def upload(self,geo):
        head,arrays=preview.arrays_for(geo)
        self.page.evaluate('''([head,a])=>{const arrays={};for(const [k,v]of Object.entries(a))arrays[k]=(k==='P'||k==='Cd'?new Float32Array(v):new Uint32Array(v));pv.set(head,arrays);}''',[head,{k:v.tolist() for k,v in arrays.items()}])
    def selection(self):
        return self.page.evaluate('pv.selection()')
    def alt_click(self,edge,ring=False):
        rect=self.page.locator('canvas').bounding_box()
        p=self.page.evaluate('e=>pv.screenOf("edge",e)',edge)
        self.page.keyboard.down('Alt')
        if ring:self.page.keyboard.down('Control')
        self.page.mouse.click(rect['x']+p[0],rect['y']+p[1])
        if ring:self.page.keyboard.up('Control')
        self.page.keyboard.up('Alt')
    def test_box_face_edge_point_conversion_preserves_full_components(self):
        box=run('box')
        self.upload(box)
        vp=box.topology.vertex_point;offs=box.topology.prim_offsets
        face=int(max(range(box.nprims),key=lambda f:box.P[vp[offs[f]:offs[f+1]]].mean(0)[2]))
        vertices=sorted(vp[offs[face]:offs[face+1]].tolist())
        self.page.evaluate('f=>pv.selectElements("prim",[f],"replace")',face)
        self.page.evaluate('pv.setMode("edge")')
        selected=self.selection()['edges']
        self.assertEqual(len(selected),4)
        self.assertEqual(sorted(set(sum(selected,[]))),vertices)
        self.page.evaluate('pv.setMode("point")')
        self.assertEqual(self.selection()['points'],vertices)
        self.page.evaluate('pv.setMode("face")')
        self.assertEqual(self.selection()['faces'],[face])
        self.page.evaluate('pv.setMode("face")')
        self.assertEqual(self.selection()['faces'],[face])
        self.page.evaluate('pv.setMode("view");pv.setMode("edge")')
        self.assertEqual(self.selection()['edges'],selected)
    def test_partial_points_or_edges_do_not_select_whole_faces(self):
        self.upload(run('box'))
        self.page.evaluate('pv.selectElements("point",[0],"replace");pv.setMode("edge")')
        self.assertEqual(self.selection()['edges'],[])
        self.page.evaluate('pv.setMode("face")')
        self.assertEqual(self.selection()['faces'],[])
        self.page.evaluate('pv.restoreSelection({mode:"edge",edges:[[0,1]],points:[],faces:[]});pv.setMode("face")')
        self.assertEqual(self.selection()['faces'],[])
    def test_tube_ngon_alt_click_follows_only_cap_perimeter(self):
        self.upload(run('tube',columns=8,rows=1,caps=True))
        self.page.evaluate('pv.setMode("edge")')
        self.alt_click([8,9])
        expected=sorted([sorted([8+i,8+(i+1)%8]) for i in range(8)])
        self.assertEqual(self.selection()['edges'],expected)
        self.page.evaluate('pv.setMode("point")')
        self.assertEqual(self.selection()['points'],list(range(8,16)))
        self.page.evaluate('pv.setMode("face")')
        self.assertEqual(self.selection()['faces'],[9])
    def test_isolated_ngon_alt_click_selects_closed_boundary(self):
        points=np.array([[math.cos(i*math.pi/4),math.sin(i*math.pi/4),0] for i in range(8)],np.float32)
        self.upload(Geometry.from_arrays(points,np.arange(8,dtype=np.int32),np.array([0,8],np.int32)))
        self.page.evaluate('pv.setMode("edge")')
        self.alt_click([0,1])
        self.assertEqual(len(self.selection()['edges']),8)
    def test_ambiguous_wire_branch_stops_and_quad_ring_is_preserved(self):
        points=np.array([[-1,0,0],[0,0,0],[1,.4,0],[1,-.4,0]],np.float32)
        self.upload(Geometry.from_arrays(points,np.array([0,1,1,2,1,3],np.int32),np.array([0,2,4,6],np.int32)))
        self.page.evaluate('pv.setMode("edge")')
        self.alt_click([0,1])
        self.assertEqual(self.selection()['edges'],[[0,1]])
        self.upload(run('box'))
        self.assertEqual(self.page.evaluate('pv.quadStrip([0,1],true).length'),4)
        self.assertEqual(self.page.evaluate('pv.quadStrip([0,1],false).length'),1)
