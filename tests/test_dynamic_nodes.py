"""Exercise the shipped transient graph solver without changing saved positions."""
import json
from pathlib import Path
import shutil
import subprocess
import unittest

ROOT = Path(__file__).resolve().parents[1]
NODE = shutil.which('node') or 'C:/Program Files/nodejs/node.exe'

class DynamicNodesTests(unittest.TestCase):
    def js(self, body):
        script = "require('./minihou/web/static/dynamic-nodes.js');" + body
        return json.loads(subprocess.check_output([NODE, '-e', script], cwd=ROOT, text=True))

    def test_default_enabled_bounded_and_no_layout_mutation(self):
        result = self.js("""
        const nodes=[{id:'drag',x:0,y:0},{id:'near',x:1,y:0}], before=JSON.stringify(nodes);
        const d=MiniDynamicNodes();d.update({nodes,dragging:['drag'],pointer:[0,0]});
        let max=0;for(let i=0;i<600;i++){d.step(.032);max=Math.max(max,Math.hypot(...d.offset('near')))}
        console.log(JSON.stringify([max,d.offset('drag'),before===JSON.stringify(nodes)]));
        """)
        self.assertGreater(result[0], 8)
        self.assertLessEqual(result[0], 12)
        self.assertEqual(result[1], [0, 0])
        self.assertTrue(result[2])

    def test_release_settles_selected_pins_and_off_resets(self):
        result = self.js("""
        const d=MiniDynamicNodes(),nodes=[{id:'a',x:10,y:0},{id:'b',x:20,y:0}];
        d.update({nodes,dragging:['x'],pointer:[0,0],selected:['a']});
        for(let i=0;i<100;i++)d.step();const pinned=d.offset('a'), displaced=d.offset('b');
        d.update({dragging:[]});for(let i=0;i<300;i++)d.step();const settled=d.offset('b');
        d.update({dragging:['x']});d.step();d.update({enabled:false});
        console.log(JSON.stringify([pinned,displaced,settled,d.offset('b')]));
        """)
        self.assertEqual(result[0], [0, 0]);self.assertGreater(result[1][0], 0)
        self.assertEqual(result[2:], [[0, 0], [0, 0]])

    def test_accessibility_and_large_graph_and_pause(self):
        result = self.js("""
        const run=(options,settings)=>{const d=MiniDynamicNodes(options);d.update({nodes:[{id:'a',x:1,y:0}],dragging:['x'],pointer:[0,0],...settings});for(let i=0;i<30;i++)d.step();return d.offset('a')};
        console.log(JSON.stringify([run({reducedMotion:true},{}),run({},{paused:true}),run({},{nodes:Array.from({length:501},(_,i)=>({id:i?'n'+i:'a',x:i,y:0}))})]));
        """)
        self.assertEqual(result, [[0, 0]] * 3)

    def test_only_48_neighbours_and_raf_stops_at_rest(self):
        result = self.js("""
        let pending=null,count=0,latest;
        const d=MiniDynamicNodes({requestFrame:fn=>{pending=fn;return 1},cancelFrame:()=>pending=null,onChange:x=>latest=x});
        d.update({nodes:Array.from({length:100},(_,i)=>({id:''+i,x:i,y:0})),dragging:['other'],pointer:[0,0]});
        while(pending&&count<1000){const f=pending;pending=null;f(++count*16)}
        console.log(JSON.stringify([latest.size,count,!!pending]));
        """)
        self.assertEqual(result[0], 48);self.assertLess(result[1], 1000);self.assertFalse(result[2])

if __name__ == '__main__':
    unittest.main()
