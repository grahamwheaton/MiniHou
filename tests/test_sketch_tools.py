"""Small deterministic checks for sketch shapes and inference using the shipped JS."""
import json
from pathlib import Path
import shutil
import subprocess
import unittest
import numpy as np
from minihou.nodes.curves import curve
ROOT = Path(__file__).resolve().parents[1]
NODE = shutil.which('node') or 'C:/Program Files/nodejs/node.exe'
class SketchTests(unittest.TestCase):
    def js(self, expression):
        script = "require('./minihou/web/static/sketch-tool.js'); const M=MiniSketchMath; console.log(JSON.stringify(" + expression + "));"
        return json.loads(subprocess.check_output([NODE, '-e', script], cwd=ROOT, text=True, encoding='utf-8'))
    def test_shapes_cook_with_existing_curve(self):
        for mode, count in [('line',2),('rectangle',4),('circle',32)]:
            for plane in ['XY','XZ','YZ']:
                end={'XY':[2,3,0],'XZ':[2,0,3],'YZ':[0,2,3]}[plane]
                spec=self.js(f"M.specification('{mode}',[[0,0,0],{json.dumps(end)}],'{plane}')")
                geo=curve(**spec['params']);geo.validate()
                self.assertEqual(geo.npoints,count)
                self.assertEqual(bool(geo.topology.closed_mask()[0]),mode!='line')
                self.assertTrue(np.isfinite(geo.P).all())
    def test_reject_degenerate_and_crossed_outlines(self):
        for points in [[[0,0,0],[0,0,0]],[[0,0,0],[1,0,0],[2,0,0]],[[0,0,0],[3,3,0],[0,2,0],[3,0,0]]]:
            result=self.js("(()=>{try{M.specification('polygon',"+json.dumps(points)+");return false}catch(e){return true}})()")
            self.assertTrue(result)
    def test_endpoint_midpoint_plane_and_axis_inference(self):
        result=self.js("M.infer([.9,.9,0],null,p=>p.map(v=>v*100),[101,99],{P:[1,1,0,3,1,0,1,1,5],edge:[0,1]},{grid:0})")
        self.assertEqual(result,{'point':[1,1,0],'label':'Endpoint'})
        result=self.js("M.infer([2,1,0],null,p=>p.map(v=>v*100),[200,100],{P:[1,1,0,3,1,0],edge:[0,1]},{grid:0})")
        self.assertEqual(result['label'],'Midpoint')
        result=self.js("M.infer([2.1,.04,0],[0,0,0],p=>p.map(v=>v*100),[210,4],null,{grid:1})")
        self.assertEqual(result['label'],'Axis')
        result=self.js("M.infer([2.1,.4,0],[0,0,0],p=>p,[2.1,.4],null,{snap:false,axis:0})")
        self.assertEqual(result['point'],[2.1,0,0])
    def test_circle_radius_and_clamp(self):
        points=self.js("M.shape('circle',[[0,0,0],[2,0,0]],null,'XY',1000)")
        self.assertEqual(len(points),256)
        np.testing.assert_allclose(np.linalg.norm(points,axis=1),2,atol=1e-8)
if __name__=='__main__':unittest.main()
