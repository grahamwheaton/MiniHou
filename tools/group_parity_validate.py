"""Run scoped group parity checks against this checkout, even with embedded _pth."""
import argparse
from pathlib import Path
import os
import sys
import unittest
ROOT=Path(__file__).resolve().parents[1]
sys.path.insert(0,str(ROOT))
import minihou
assert Path(minihou.__file__).resolve().parent == ROOT/'minihou',minihou.__file__
ENGINE=['test_rule_params','test_sdk_graph','test_geometry','test_evaluator','test_groups','test_group_filters','test_group_expand','test_group_rules','test_group_helpers','test_group_extended','test_group_examples','test_modeling_tools','test_modeling_workflows','test_model','test_edge_tools','test_wrangle','test_paint','test_outputs','test_output_groups','test_select_group_api','test_preview','test_guides','test_link','test_graph_splice','test_library','test_graph_library','test_node_menu','test_shelf','test_pie_menu','test_measurement_data','test_length_fields.LengthFieldSchemaTests']
ENGINE += ['test_accel_diagnostics']
BROWSER=['test_measurement_layout.MeasurementLayoutTests','test_group_editor_e2e.GroupEditorE2E','test_measurement_browser.MeasurementBrowserTests','test_measurement_browser.MeasurementEditorTests','test_selection_tools_browser.SelectionToolsTests','test_selection_flow_browser.SelectionFlowBrowserTests','test_viewport_user_fixes.ViewportFixesTests','test_viewport_followups.ViewportFollowups','test_length_fields.LengthFieldBrowserTests']
BROWSER += ['test_editor_e2e.EditorE2E.'+m for m in ['test_shelf_click_adds_sources_unwired_and_inserts_modifiers_after_the_selection','test_pie_menu_hold_flick_release_inserts_a_tool_after_the_selected_node','test_painting_points_is_one_edit_one_undo_step_and_erase_goes_back','test_painting_faces_and_edges_and_a_vector_colour','test_preview_refresh_does_not_erase_an_active_paint_stroke','test_f_frames_viewport_selection','test_control_shortest_path_and_alt_ring']]
p=argparse.ArgumentParser();p.add_argument('--suite',choices=['engine','browser'],default='engine');p.add_argument('--log',type=Path);a=p.parse_args()
if a.suite=='browser':
 os.environ.setdefault('PLAYWRIGHT_BROWSERS_PATH',str(ROOT.parent/'runtime'/'browsers'))
modules=ENGINE if a.suite=='engine' else BROWSER
print('MINIHOU_IMPORT',minihou.__file__,flush=True)
log=a.log or ROOT/'build'/'group-parity-validation'/('group-'+a.suite+'-test.log')
log.parent.mkdir(parents=True,exist_ok=True)
with log.open('w',encoding='utf-8') as f:
 f.write('MINIHOU_IMPORT '+minihou.__file__+'\n');f.flush()
 result=unittest.TextTestRunner(stream=f,verbosity=2).run(unittest.defaultTestLoader.loadTestsFromNames(['tests.'+m for m in modules]))
print(a.suite.upper(),'tests',result.testsRun,'failures',len(result.failures),'errors',len(result.errors),'skips',len(result.skipped),flush=True)
sys.exit(0 if result.wasSuccessful() and not result.skipped else 1)
