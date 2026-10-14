"""Group controls exercised in actual Chromium against this worktree server."""
import unittest
import numpy as np
from minihou import Evaluator, Graph
from minihou.nodes import group_rules as R
from tests.test_editor_e2e import EditorE2E, until, sync_playwright

@unittest.skipIf(sync_playwright is None,'playwright not installed')
class GroupEditorE2E(unittest.TestCase):
    setUpClass=classmethod(EditorE2E.setUpClass.__func__)
    tearDownClass=classmethod(EditorE2E.tearDownClass.__func__)
    setUp=EditorE2E.setUp
    tearDown=EditorE2E.tearDown
    node=EditorE2E.node
    box_scene=EditorE2E.box_scene
    view_xy=EditorE2E.view_xy
    stroke=EditorE2E.stroke
    pie_slot_xy=EditorE2E.pie_slot_xy
    empty_spot=EditorE2E.empty_spot

    def scene(self,nodes,display):
        self.s.edit([{'op':'replace','graph':{'nodes':nodes,'display':display}}])
        self.page.wait_for_selector('.node[data-id="%s"]'%display)
        self.node(display).locator('.title').click()
        self.page.wait_for_function("document.querySelector('#pv-node').innerText.includes(%r)"%display)
    def row(self,index=0):return self.page.locator('#params > .prow[data-param="rules"] > .rule-list > .rule-row').nth(index)
    def field(self,row,name):return row.locator('> .rule-field[data-field="%s"] [data-field="%s"]'%(name,name)).first
    def set_text(self,loc,value):loc.fill(value);loc.press('Tab')
    def output(self,node):return self.s.evaluator.cook(self.s.graph,node)

    def test_shelf_pie_and_search_are_real_tools(self):
        self.box_scene()
        self.page.click('#shelf .sh-tab[data-tab="Groups"]')
        labels=['Group by Lasso','Group Paint','Group Combine','Group Delete','Group Rename','Group Expression','Group by Cd Colour','Group by Normal','Group by Proximity']
        for label in labels:self.assertEqual(self.page.locator('#shelf .sh-tool[data-label="%s"]'%label).count(),1)
        self.page.locator('#shelf .sh-tool[data-label="Group Combine"]').click()
        self.assertTrue(until(lambda:any(n.type=='group_combine' for n in self.s.graph.nodes.values())))
        self.page.locator('#canvas').focus();self.page.mouse.move(*self.empty_spot());self.page.keyboard.press('c');self.page.wait_for_selector('#pie:not(.hidden)')
        self.page.keyboard.press('4');self.page.wait_for_function("MiniPie.state().page==='group'")
        self.page.mouse.click(*self.pie_slot_xy(2))
        self.assertTrue(until(lambda:any(n.type=='group_paint' for n in self.s.graph.nodes.values())))
        self.assertEqual(self.page.evaluate("MiniPie.find('group_expression').type"),'group_expression')
        self.assertIn('Cd colour',self.page.evaluate("minihou.S.types.group_by_color.keywords"))

    def test_rule_add_remove_reorder_enable_undo_reload(self):
        self.scene({'b':{'type':'box'},'a':{'type':'group_paint','inputs':{'geo':'b'},'params':{'name':'a','selection':'0-2'}},'d':{'type':'group_delete','inputs':{'geo':'a'}}},'d')
        add=self.page.locator('#params > .prow[data-param="rules"] > .rule-list > .add-rule')
        add.click();self.assertTrue(until(lambda:len(self.s.graph.nodes['d'].params.get('rules',[]))==1))
        self.set_text(self.field(self.row(),'name'),'a')
        add.click();self.assertTrue(until(lambda:len(self.s.graph.nodes['d'].params['rules'])==2))
        self.set_text(self.field(self.row(1),'name'),'unused*')
        self.field(self.row(1),'enabled').uncheck()
        self.row(1).get_by_label('Move rule up',exact=True).click()
        self.assertTrue(until(lambda:self.s.graph.nodes['d'].params['rules'][0]['name']=='unused*'))
        self.assertFalse(self.s.graph.nodes['d'].params['rules'][0]['enabled'])
        self.row(0).get_by_role('button',name='Remove rule',exact=True).click()
        self.assertTrue(until(lambda:len(self.s.graph.nodes['d'].params['rules'])==1))
        self.page.click('#undo-btn');self.assertTrue(until(lambda:len(self.s.graph.nodes['d'].params.get('rules',[]))==2))
        self.page.reload();self.page.wait_for_selector('.node[data-id="d"]');self.node('d').locator('.title').click()
        self.assertEqual(self.row(0).locator('input[data-field="name"]').input_value(),'unused*')
        saved=Graph.load(self.path,check=False)
        self.assertEqual(saved.nodes['d'].params['rules'],self.s.graph.nodes['d'].params['rules'])

    def test_create_promote_combine_transform_and_blast(self):
        self.scene({'b':{'type':'box'},'c':{'type':'group_create','inputs':{'geo':'b'},'params':{'name':'seed','pattern':'0','domain':'points'}},'p':{'type':'group_promote','inputs':{'geo':'c'}},'k':{'type':'group_combine','inputs':{'geo':'p'}},'t':{'type':'transform','inputs':{'geo':'k'},'params':{'group':'result','translate':[0,0,1]}},'x':{'type':'blast','inputs':{'geo':'k'},'params':{'group':'result','domain':'primitives'}}},'p')
        self.page.locator('#params .add-rule').first.click()
        self.field(self.row(),'to').select_option('primitives')
        self.set_text(self.field(self.row(),'name'),'seed')
        self.set_text(self.field(self.row(),'new_name'),'faces')
        self.field(self.row(),'keep_original').check()
        self.assertTrue(until(lambda:self.s.status()['ok']))
        self.assertGreater(R.membership(self.output('p'),'prim','faces').sum(),0)
        self.node('k').locator('.title').click();self.page.locator('#params .add-rule').first.click()
        self.field(self.row(),'domain').select_option('primitives')
        self.set_text(self.field(self.row(),'name'),'result');self.set_text(self.field(self.row(),'base'),'faces')
        self.assertTrue(until(lambda:self.s.graph.nodes['k'].params.get('rules',[{}])[0].get('base')=='faces'))
        self.assertTrue(until(lambda:self.s.status()['ok']))
        base=self.output('k');m=R.membership(base,'prim','result');self.assertGreater(m.sum(),0)
        moved=self.output('t');self.assertGreater(np.linalg.norm(moved.P-base.P),0)
        self.assertEqual(self.output('x').nprims,base.nprims-int(m.sum()))
        self.node('t').locator('.title').click();field=self.page.locator('#params .prow[data-param="group"] input');field.focus()
        self.page.wait_for_function("document.querySelector('#params .prow[data-param=group] datalist').options.length>0")
        self.page.locator('#params .prow[data-param="group"]').get_by_role('button',name='Select',exact=True).click()
        self.page.wait_for_function("minihou.preview.selection().points.length>0")

    def test_combine_nested_operations_and_expression_controls(self):
        self.scene({'b':{'type':'box'},'a':{'type':'group_paint','inputs':{'geo':'b'},'params':{'name':'a','selection':'0-2'}},'k':{'type':'group_combine','inputs':{'geo':'a'},'params':{'rules':[{'domain':'points','name':'r','base':'a'}]}},'e':{'type':'group_expression','inputs':{'geo':'k'}}},'k')
        self.row().locator('.rule-field[data-field="operations"] .add-rule').click()
        operation=self.row().locator('.rule-field[data-field="operations"] .rule-row')
        operation.locator('select[data-field="op"]').select_option('subtract')
        self.set_text(operation.locator('input[data-field="group"]'),'0')
        self.assertTrue(until(lambda:self.s.graph.nodes['k'].params['rules'][0]['operations'][0]['group']=='0'))
        self.assertEqual(np.flatnonzero(R.membership(self.output('k'),'point','r')).tolist(),[1,2])
        self.node('e').locator('.title').click();self.page.locator('#params .add-rule').click()
        self.set_text(self.field(self.row(),'name'),'above');self.set_text(self.field(self.row(),'expression'),'@P.z > 0')
        self.assertTrue(until(lambda:self.s.graph.nodes['e'].params.get('rules',[{}])[0].get('expression')=='@P.z > 0'))
        self.assertEqual(R.membership(self.output('e'),'point','above').sum(),4)

    def test_lasso_records_membership_and_vertex_select(self):
        self.box_scene()
        self.s.edit([{'op':'insert','after':'b','id':'l','type':'group_lasso','params':{'domain':'vertices','name':'corners'}}])
        self.page.wait_for_selector('.node[data-id="l"]');self.node('l').locator('.title').click()
        self.page.wait_for_function("minihou.preview.geometry()?.vertexPoint?.length===24")
        self.page.click('#params button:text-is("Lasso membership")')
        # Use the real lasso tool across the complete canvas interior.
        b=self.page.locator('#pv-canvas').bounding_box();x,y,w,h=b['x'],b['y'],b['width'],b['height']
        self.page.mouse.move(x+10,y+10);self.page.mouse.down()
        for xx,yy in [(x+w-10,y+10),(x+w-10,y+h-10),(x+10,y+h-10),(x+10,y+10)]:self.page.mouse.move(xx,yy,steps=3)
        self.page.mouse.up()
        self.page.click('#params button:text-is("Use viewport selection")')
        self.assertTrue(until(lambda:bool(self.s.graph.nodes['l'].params.get('selection'))))
        self.assertGreater(R.membership(self.output('l'),'vertex','corners').sum(),0)
        self.page.reload();self.page.wait_for_selector('.node[data-id="l"]');self.node('l').locator('.title').click()
        self.assertTrue(self.page.locator('#params .prow[data-param="selection"] input').input_value())

    def test_group_brush_persists_one_undoable_stroke(self):
        self.scene({'g':{'type':'grid','params':{'columns':10,'rows':10,'size_x':4,'size_y':4}},'p':{'type':'group_paint','inputs':{'geo':'g'},'params':{'name':'brushed'}}},'p')
        self.page.click('#params button:text-is("Paint membership")')
        self.page.wait_for_function("minihou.paint.active() && minihou.paint.values()?.n===100")
        self.page.locator('#attr-card .pa-radius').evaluate("e=>{e.value=100;e.dispatchEvent(new Event('input'));}")
        b=self.page.locator('#pv-canvas').bounding_box();cx,cy=b['width']/2,b['height']/2
        self.stroke([(cx-60,cy),(cx+60,cy)])
        self.assertTrue(until(lambda:bool(self.s.graph.nodes['p'].params.get('selection'))),str(self.page.evaluate("({state:minihou.paint.state,stats:minihou.paint.stats,vals:Array.from(minihou.paint.values()?.vals||[]),node:document.querySelector('#pv-node').textContent})")))
        self.assertFalse(any(n.type=='attrib_paint' for n in self.s.graph.nodes.values()))
        self.assertGreater(R.membership(self.output('p'),'point','brushed').sum(),0)
        self.page.keyboard.press('Control+z');self.assertTrue(until(lambda:not self.s.graph.nodes['p'].params.get('selection')))
        self.page.keyboard.press('Control+Shift+z');self.assertTrue(until(lambda:bool(self.s.graph.nodes['p'].params.get('selection'))))
        self.page.click('#pv-max')
        self.page.reload();self.page.wait_for_selector('.node[data-id="p"]');self.node('p').locator('.title').click()
        self.assertTrue(self.page.locator('#params .prow[data-param="selection"] input').input_value())

    def test_group_brush_four_domains(self):
        for domain,short in [('points','point'),('primitives','prim'),('edges','edge'),('vertices','vertex')]:
            with self.subTest(domain=domain):
                self.scene({'g':{'type':'grid','params':{'columns':8,'rows':8,'size_x':4,'size_y':4}},'p':{'type':'group_paint','inputs':{'geo':'g'},'params':{'name':'painted','domain':domain}}},'p')
                self.page.click('#params button:text-is("Paint membership")')
                self.page.wait_for_function("minihou.paint.active() && minihou.paint.values()?.node==='p' && minihou.paint.state.domain===%r"%short)
                self.page.locator('#attr-card .pa-radius').evaluate("e=>{e.value=100;e.dispatchEvent(new Event('input'));}")
                b=self.page.locator('#pv-canvas').bounding_box();cx,cy=b['width']/2,b['height']/2
                self.stroke([(cx-45,cy),(cx+45,cy)])
                self.assertTrue(until(lambda:bool(self.s.graph.nodes['p'].params.get('selection'))))
                self.assertGreater(R.membership(self.output('p'),short,'painted').sum(),0)
                self.page.keyboard.press('Escape');self.page.click('#pv-max')

    def test_rename_rules_and_helper_lengths(self):
        self.scene({'b':{'type':'box'},'a':{'type':'group_paint','inputs':{'geo':'b'},'params':{'name':'old','selection':'0'}},'r':{'type':'group_rename','inputs':{'geo':'a'}}},'r')
        self.page.locator('#params .add-rule').click()
        self.set_text(self.field(self.row(),'name'),'old');self.set_text(self.field(self.row(),'new_name'),'new')
        self.assertTrue(until(lambda:self.s.graph.nodes['r'].params.get('rules',[{}])[0].get('new_name')=='new'))
        self.assertEqual(R.names(self.output('r'),'point'),['new'])
        self.s.edit([{'op':'insert','after':'r','id':'h','type':'group_by_proximity','params':{'name':'near'}}])
        self.page.wait_for_selector('.node[data-id="h"]');self.node('h').locator('.title').click()
        self.page.evaluate("MiniUnits.configure({length_unit:'mm',unit_scale:1})")
        row=self.page.locator('#params .prow[data-param="distance"]')
        self.assertIn('(mm)',row.locator('.pl').inner_text())
        self.set_text(row.locator('input[type=number]'),'250')
        self.assertTrue(until(lambda:self.s.graph.nodes['h'].params.get('distance')==.25))
        self.assertIn('(mm)',self.page.locator('#params .prow[data-param="position"] .pl').inner_text())
        self.assertNotIn('(mm)',self.page.locator('#params .prow[data-param="invert"] .pl').inner_text())

    def test_exact_corner_select_capture_does_not_broaden_seam(self):
        self.scene({'b':{'type':'box'},'l':{'type':'group_lasso','inputs':{'geo':'b'},'params':{'domain':'vertices','name':'seam','selection':'0'}}},'l')
        self.page.wait_for_function("minihou.preview.geometry()?.vertexPoint?.length===24")
        self.page.evaluate("minihou.preview.selectElements('vertex',[0],'replace')")
        self.assertEqual(self.page.evaluate('minihou.preview.vertexSelection()'),[0])
        other=self.page.evaluate('(()=>{const v=minihou.preview.geometry().vertexPoint;return Array.from(v.keys()).find(i=>i!==0&&v[i]===v[0]);})()')
        self.page.evaluate("i=>minihou.preview.selectElements('vertex',[i],'add')",other)
        self.assertEqual(set(self.page.evaluate('minihou.preview.vertexSelection()')),{0,other})
        self.page.evaluate("i=>minihou.preview.selectElements('vertex',[i],'subtract')",other)
        self.assertEqual(self.page.evaluate('minihou.preview.vertexSelection()'),[0])
        self.page.click('#params button:text-is("Use viewport selection")')
        self.assertTrue(until(lambda:self.s.graph.nodes['l'].params.get('selection')=='0'))
        self.assertEqual(R.membership(self.output('l'),'vertex','seam').sum(),1)
        self.page.evaluate("minihou.preview.selectElements('point',[0],'replace')")
        self.assertIsNone(self.page.evaluate('minihou.preview.vertexSelection()'))

    def test_group_brush_erase_clears_membership(self):
        self.scene({'g':{'type':'grid','params':{'columns':8,'rows':8,'size_x':4,'size_y':4}},'p':{'type':'group_paint','inputs':{'geo':'g'},'params':{'name':'erase','selection':'*'}}},'p')
        self.page.click('#params button:text-is("Paint membership")')
        self.page.wait_for_function("minihou.paint.values()?.n===64 && minihou.paint.active()")
        self.page.locator('#attr-card .pa-radius').evaluate("e=>{e.value=120;e.dispatchEvent(new Event('input'));}")
        b=self.page.locator('#pv-canvas').bounding_box();cx,cy=b['width']/2,b['height']/2
        self.stroke([(cx-40,cy),(cx+40,cy)],shift=True)
        self.assertTrue(until(lambda:self.s.graph.nodes['p'].params.get('selection')!='*'))
        self.assertLess(R.membership(self.output('p'),'point','erase').sum(),64)
        self.page.click('#pv-max')
