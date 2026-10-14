"""Run inside Blender to list all native mesh operators and their MiniHou status.

blender --background --factory-startup --python tools/audit_blender_modelling.py -- /tmp/coverage.json
The inventory is versioned evidence, not an assertion that every operator was tested.
"""
import json
from pathlib import Path
import sys

import bpy

sys.path.insert(0, str(Path(__file__).resolve().parents[1]/'minihou'))
from blender_mesh_catalog import OPERATORS

INTERACTIVE = {
    'knife_tool', 'knife_project', 'rip', 'rip_move', 'rip_edge', 'rip_edge_move',
    'loopcut', 'loopcut_slide', 'dupli_extrude_cursor',
    'polybuild_delete_at_cursor', 'polybuild_dissolve_at_cursor', 'polybuild_extrude_at_cursor_move',
    'polybuild_face_at_cursor', 'polybuild_face_at_cursor_move', 'polybuild_split_at_cursor',
    'polybuild_split_at_cursor_move', 'polybuild_transform_at_cursor', 'polybuild_transform_at_cursor_move',
}
EQUIVALENTS = {
    'duplicate': 'copy_transform', 'duplicate_move': 'copy_transform', 'separate': 'blast (split graph branches)',
    'extrude_context': 'polyextrude', 'extrude_context_move': 'polyextrude',
    'extrude_edges_indiv': 'blender_mesh: extrude_edges_move', 'extrude_faces_indiv': 'blender_mesh: extrude_faces_move',
    'extrude_region': 'native_extrude', 'extrude_region_shrink_fatten': 'polyextrude',
    'extrude_verts_indiv': 'blender_mesh: extrude_vertices_move', 'dissolve_mode': 'dissolve_edges / limited_dissolve',
    'offset_edge_loops_slide': 'blender_mesh: offset_edge_loops (slide is not equivalent)',
}
rows = []
for name in dir(bpy.ops.mesh):
    op = getattr(bpy.ops.mesh, name)
    try: rna = op.get_rna_type()
    except Exception: continue
    if name in OPERATORS: status, route = 'native-backend', 'blender_mesh: '+name
    elif name in EQUIVALENTS: status, route = 'related-tool', EQUIVALENTS[name]
    elif name in INTERACTIVE: status, route = 'interactive-gap', 'Needs dedicated viewport interaction/context'
    elif name.startswith('select_') or name in {'edgering_select','edges_select_sharp','faces_select_linked_flat','loop_multi_select','loop_select','loop_to_region','region_to_loop','shortest_path_pick','shortest_path_select','hide','reveal'}:
        status, route = 'selection-workflow', 'Viewport selection / groups; not full operator parity'
    else: status, route = 'remaining-gap', 'Not yet exposed by the mesh backend'
    rows.append({'operator':name, 'label':rna.name, 'description':rna.description,
                 'status':status, 'route':route, 'parameters':[p.identifier for p in rna.properties if p.identifier != 'rna_type']})
output = {'blender_version':bpy.app.version_string,'operators':rows,'unavailable_backend_operators':sorted(set(OPERATORS)-{r['operator'] for r in rows})}
args = sys.argv[sys.argv.index('--')+1:] if '--' in sys.argv else []
if args: Path(args[0]).write_text(json.dumps(output,indent=2),encoding='utf-8')
print(json.dumps({'version':output['blender_version'],'counts':{s:sum(r['status']==s for r in rows) for s in sorted({r['status'] for r in rows})},'unavailable':output['unavailable_backend_operators']},indent=2))
