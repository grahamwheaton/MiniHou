"""Real browser keyboard and multi-object preview regressions."""
import unittest
from tests import test_blender_selection_integration as fixture

class ViewportFixesTests(unittest.TestCase):
    setUpClass=classmethod(fixture.BlenderSelectionIntegrationTests.setUpClass.__func__)
    tearDownClass=classmethod(fixture.BlenderSelectionIntegrationTests.tearDownClass.__func__)
    setUp=fixture.BlenderSelectionIntegrationTests.setUp
    tearDown=fixture.BlenderSelectionIntegrationTests.tearDown
    node=fixture.BlenderSelectionIntegrationTests.node
    box_scene=fixture.BlenderSelectionIntegrationTests.box_scene
    view_xy=fixture.BlenderSelectionIntegrationTests.view_xy
    selection=fixture.BlenderSelectionIntegrationTests.selection

    def hover(self):
        b=self.page.locator('#pv-canvas').bounding_box()
        self.page.mouse.move(b['x']+b['width']/2,b['y']+b['height']/2)

    def test_opposite_views_and_input_focus(self):
        self.box_scene();self.hover()
        for name in ['front','back','left','right','top','bottom']:
            self.page.evaluate('(name)=>minihou.preview.viewAxis(name,false,true)',name)
            before=self.page.evaluate('({...minihou.preview.view})')
            self.page.keyboard.press('Numpad9')
            after=self.page.evaluate('({...minihou.preview.view})')
            self.assertTrue(after['ortho'])
            self.assertNotEqual((before['yaw'],before['pitch']),(after['yaw'],after['pitch']))
            self.page.keyboard.press('Numpad9')
            again=self.page.evaluate('({...minihou.preview.view})')
            import math
            self.assertAlmostEqual(math.sin(before['yaw']),math.sin(again['yaw']),places=5)
            self.assertAlmostEqual(before['pitch'],again['pitch'],places=5)
        self.page.evaluate("document.querySelector('#pv-canvas').parentElement.appendChild(Object.assign(document.createElement('input'),{id:'focus-test'}))")
        self.page.locator('#focus-test').focus();before=self.page.evaluate('JSON.stringify(minihou.preview.view)')
        self.page.keyboard.press('Numpad9')
        self.assertEqual(before,self.page.evaluate('JSON.stringify(minihou.preview.view)'))

    def test_a_modes_only_active_mesh_and_dim_preserves_geometry(self):
        self.box_scene()
        self.page.evaluate("minihou.edit([{op:'add',id:'other',type:'box',params:{center:[4,0,0]}},{op:'flag',id:'other',flag:'display',value:true,additive:true}])")
        self.page.wait_for_function('minihou.preview.points===16')
        for mode,key,count in [('point','points',8),('edge','edges',12),('face','faces',6),('piece','faces',12)]:
            self.page.click(f'#preview [data-mode="{mode}"]');self.hover();self.page.keyboard.press('a')
            self.assertEqual(len(self.selection()[key]),count)
        before=self.page.evaluate('minihou.preview.geometry().P.length')
        self.page.click('[data-pv-menu="pv-menu-view"]')
        self.page.check('#pv-isolated')
        self.assertTrue(self.page.evaluate('minihou.preview.opt.isolated'))
        self.assertEqual(before,self.page.evaluate('minihou.preview.geometry().P.length'))
        self.assertEqual(self.page.evaluate('minihou.preview.measurementGeometry().map(o=>o.P.length)'),[24,24])

    def test_flip_selected_faces_keeps_selection_and_undo(self):
        self.box_scene();self.page.evaluate('minihou.preview.selectElements("prim",[0,2])');self.hover()
        self.page.keyboard.press('Alt+n')
        self.page.wait_for_function("Object.values(minihou.S.graph.nodes).some(n=>n.type==='reverse')")
        self.page.wait_for_function('minihou.preview.selection().faces.length===2')
        self.assertEqual(self.selection()['faces'],[0,2])
        self.page.keyboard.press('Control+z')
        self.page.wait_for_function("!Object.values(minihou.S.graph.nodes).some(n=>n.type==='reverse')")

    def test_isolated_pixels_follow_picked_object_and_nonselectable_reference(self):
        import base64
        self.box_scene()
        self.page.evaluate("minihou.edit([{op:'add',id:'other',type:'box',params:{center:[4,0,0]}},{op:'flag',id:'other',flag:'display',value:true,additive:true},{op:'add',id:'reference',type:'box',params:{center:[-4,0,0]}},{op:'flag',id:'reference',flag:'template',value:true,additive:true},{op:'add',id:'hidden',type:'box',params:{center:[8,0,0]}},{op:'flag',id:'hidden',flag:'display',value:false,additive:true}])")
        self.page.evaluate("minihou.edit([{op:'flag',id:'hidden',flag:'display',value:false,additive:true}])")
        self.page.wait_for_function('minihou.preview.points===16 && minihou.preview.templatePoints===8')
        self.page.evaluate("minihou.preview.viewAxis('top',false,true);minihou.preview.frameAll();minihou.preview.opt.grid=false;minihou.preview.opt.axes=false")
        self.page.click('#preview [data-mode="piece"]')
        self.hover()
        self.page.keyboard.press('a')
        self.assertEqual(len(self.selection()['faces']),12)
        self.assertEqual(self.page.evaluate('minihou.preview.previewObjects.map(o=>o.node)'),['b','other'])
        self.page.evaluate('minihou.preview.clearSelection()')
        def pixels():
            self.page.evaluate('minihou.preview.redraw()')
            self.page.wait_for_timeout(60)
            png=base64.b64encode(self.page.locator('#pv-canvas').screenshot()).decode('ascii')
            coords=self.page.evaluate('[[.2,.2,1],[4.2,.2,.5],[-4,-.5,.5]].map(p=>minihou.preview.project(p))')
            return self.page.evaluate('''async ([png,points])=>{const img=new Image();img.src='data:image/png;base64,'+png;await img.decode();const c=document.createElement('canvas');c.width=img.width;c.height=img.height;const ctx=c.getContext('2d');ctx.drawImage(img,0,0);return points.map(p=>Array.from(ctx.getImageData(Math.round(p[0]),Math.round(p[1]),1,1).data).slice(0,3));}''',[png,coords])
        # Deliberately interleave object triangle owners, as Morton chunk ordering does.
        # Primitive colouring unwelds this order; isolation must use owner IDs, not a prefix.
        self.page.evaluate("""()=>{const pv=minihou.preview,g=pv.geometry(),T=g.tri.slice(),F=g.triPrim.slice(),order=[];for(let i=0;i<12;i++)order.push(i,i+12);order.forEach((t,i)=>{g.tri.set(T.subarray(t*3,t*3+3),i*3);g.triPrim[i]=F[t];});const values=new Float32Array(12*3);for(let i=0;i<12;i++)values.set([.62,.68,.76],i*3);pv.setAttrView({domain:'prim',comps:3,values,min:0,max:1});}""")
        normal=pixels()
        self.page.click('[data-pv-menu="pv-menu-view"]');self.page.check('#pv-isolated');self.page.keyboard.press('Escape')
        dim=pixels()
        self.assertEqual(normal[0],dim[0])
        self.assertGreater(sum(normal[1]),sum(dim[1])+30)
        self.assertNotEqual(normal[2],dim[2], "Reference wire should also dim")
        # Real object pick transfers active identity without changing graph display flags.
        x,y=self.view_xy('face',self.top+6)
        self.page.mouse.click(x,y)
        self.assertEqual(self.page.evaluate('minihou.preview.activeObject'),'other')
        self.page.evaluate('minihou.preview.clearSelection()')
        switched=pixels()
        self.assertEqual(normal[1],switched[1])
        self.assertGreater(sum(normal[0]),sum(switched[0])+30)
        self.page.click('#preview [data-mode="point"]');self.hover();self.page.keyboard.press('a')
        self.assertEqual(self.selection()['points'],list(range(8,16)))

    def test_flip_extra_object_face_only(self):
        import numpy as np
        from minihou import Evaluator
        self.box_scene()
        self.page.evaluate("minihou.edit([{op:'add',id:'other',type:'box',params:{center:[4,0,0]}},{op:'flag',id:'other',flag:'display',value:true,additive:true}])")
        self.page.wait_for_function('minihou.preview.points===16')
        before=self.s.preview('b',templates=True)['geo']
        self.page.evaluate('minihou.preview.selectElements("prim",[6])');self.hover();self.page.keyboard.press('Alt+n')
        self.page.wait_for_function("Object.values(minihou.S.graph.nodes).some(n=>n.type==='reverse')")
        self.page.wait_for_function('minihou.preview.selection().faces.length===1')
        self.assertEqual(self.selection()['faces'],[6])
        target=next(n.id for n in self.s.graph.nodes.values() if n.type=='reverse')
        after=Evaluator().cook(self.s.graph,target)
        off=before.topology.prim_offsets
        for i in range(before.nprims):
            a,b=off[i:i+2]
            expected=before.topology.vertex_point[a:b]
            np.testing.assert_array_equal(after.topology.vertex_point[a:b],expected[::-1] if i==6 else expected)
        np.testing.assert_array_equal(after.P,before.P)

    def test_primary_instance_shader_dims_when_other_displayed_mesh_is_active(self):
        self.box_scene()
        self.s.edit([{"op":"replace","graph":{"nodes":{
            "p":{"type":"point","params":{}},
            "i":{"type":"instance","inputs":{"points":"p"},"params":{"objects":"Tree","scale_min":1,"scale_max":1}},
            "other":{"type":"box","params":{"center":[4,0,0]},"display_extra":True}},"display":"i"}}])
        self.page.wait_for_function('minihou.preview.instanceStats().on && minihou.preview.tris===12')
        self.page.evaluate("minihou.preview.viewAxis('top',false,true);minihou.preview.frameAll();minihou.preview.opt.grid=false;minihou.preview.opt.axes=false;minihou.preview.opt.points=false;minihou.preview.redraw()")
        def samples():
            return self.page.evaluate("""()=>new Promise(resolve=>{const pv=minihou.preview,c=document.querySelector('#pv-canvas'),gl=c.getContext('webgl2');pv.redraw();requestAnimationFrame(()=>{const out=[[.15,.15,.5],[4.15,.15,.5]].map(p=>{const s=pv.project(p),v=new Uint8Array(4);gl.readPixels(Math.round(s[0]*c.width/c.clientWidth),c.height-1-Math.round(s[1]*c.height/c.clientHeight),1,1,gl.RGBA,gl.UNSIGNED_BYTE,v);return Array.from(v).slice(0,3);});resolve(out);});})""")
        normal=samples()
        self.page.click('[data-pv-menu="pv-menu-view"]');self.page.check('#pv-isolated');self.page.keyboard.press('Escape')
        dim=samples()
        self.assertEqual(normal[0],dim[0])
        self.assertGreater(sum(normal[1]),sum(dim[1])+30)
        self.page.click('#preview [data-mode="piece"]')
        self.page.mouse.click(*self.view_xy('face',self.top))
        self.assertEqual(self.page.evaluate('minihou.preview.activeObject'),'other')
        self.page.evaluate('minihou.preview.clearSelection()')
        switched=samples()
        self.assertGreater(sum(normal[0]),sum(switched[0])+30)
        self.assertEqual(normal[1],switched[1])
