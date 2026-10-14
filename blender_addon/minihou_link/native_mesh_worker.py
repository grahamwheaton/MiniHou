"""Trusted standalone script executed only inside a fresh background Blender.

JSON contains geometry and allowlisted operator parameters, never Python code.
Temporary scalar BMesh layers carry all supported MiniHou attribute components.
"""
import json
from pathlib import Path
import sys

import bpy
import bmesh

sys.path.insert(0, str(Path(__file__).resolve().parents[2]/'minihou'))
from blender_mesh_catalog import OPERATORS


def main(src, dst):
    request = json.loads(Path(src).read_text(encoding='utf-8'))
    operation = request['operation']
    if operation not in OPERATORS:
        raise ValueError('Operator is not allowed')
    bm = bmesh.new()
    verts = [bm.verts.new(p) for p in request['points']]
    faces = []
    face_sources = []
    loose_sources = {}
    for i, (indices, closed) in enumerate(zip(request['primitives'], request['closed'])):
        if closed:
            if len(indices) < 3 or len(set(indices)) != len(indices):
                raise ValueError('Native Blender modelling requires valid polygons without repeated points')
            faces.append(bm.faces.new([verts[v] for v in indices])); face_sources.append(i)
        else:
            for a, b in zip(indices, indices[1:]):
                key = tuple(sorted((a, b)))
                if a == b:
                    raise ValueError('Native Blender modelling requires nondegenerate edges')
                if not bm.edges.get((verts[a], verts[b])):
                    bm.edges.new((verts[a], verts[b]))
                loose_sources[key] = i
    bm.verts.index_update(); bm.edges.index_update(); bm.faces.index_update()
    face_index = {original: face for original, face in zip(face_sources, faces)}
    edge_index = {tuple(sorted((e.verts[0].index, e.verts[1].index))): e for e in bm.edges}
    specs = []
    for i, spec in enumerate(request['attributes']):
        spec = dict(spec)
        source = spec['domain']
        container = {'point': bm.verts, 'vertex': bm.loops, 'prim': bm.faces}[source]
        names = [f'mh_attr_{i}_{c}' for c in range(spec['components'])]
        layers = [(container.layers.float if spec['dtype'].startswith('float') else container.layers.int).new(n) for n in names]
        spec['layers'] = names
        specs.append(spec)
        # Adding custom layers can invalidate previously held BMesh element wrappers.
        verts = list(bm.verts); faces = list(bm.faces)
        face_index = dict(zip(face_sources, faces))
        edge_index = {tuple(sorted((e.verts[0].index, e.verts[1].index))): e for e in bm.edges}
        if source == 'point':
            elements = list(bm.verts); rows = spec['values']
        elif source == 'prim':
            elements = faces; rows = [spec['values'][i] for i in face_sources]
        else:
            elements = [loop for f in faces for loop in f.loops]
            offsets = [0]
            for p in request['primitives']: offsets.append(offsets[-1]+len(p))
            rows = [spec['values'][j] for i in face_sources for j in range(offsets[i], offsets[i+1])]
        for element, row in zip(elements, rows):
            row = row if spec['vector'] else [row]
            for layer, value in zip(layers, row):
                element[layer] = float(value) if spec['dtype'].startswith('float') else int(value)
    # Populate real UV and material data too, so native seam/material tests work.
    uv_spec = next((a for a in request['attributes'] if a['name'] == 'uv' and a['domain'] == 'vertex' and a['components'] >= 2), None)
    if uv_spec is None:
        uv_spec = next((a for a in request['attributes'] if a['name'] == 'uv' and a['domain'] == 'point' and a['components'] >= 2), None)
    if uv_spec is not None:
        uv = bm.loops.layers.uv.new('MiniHouUV')
        offset = 0
        for i, indices in enumerate(request['primitives']):
            if i in face_index:
                for j, loop in enumerate(face_index[i].loops):
                    row = uv_spec['values'][offset+j if uv_spec['domain'] == 'vertex' else indices[j]]
                    loop[uv].uv = row[:2]
            offset += len(indices)
    material = next((a for a in request['attributes'] if a['name'] == 'material_index' and a['domain'] == 'prim'), None)
    if material is not None:
        for f, i in zip(faces, face_sources): f.material_index = int(material['values'][i])
    edge_specs = []
    for i, spec in enumerate(request['edge_attributes']):
        name = f'mh_edge_{i}'
        layer = bm.edges.layers.float.new(name)
        edge_specs.append(dict(spec, layer=name))
        edge_index = {tuple(sorted((e.verts[0].index, e.verts[1].index))): e for e in bm.edges}
        for pair, value in zip(request['edges'], spec['values']):
            edge_index[tuple(pair)][layer] = float(value)
    verts = list(bm.verts); faces = list(bm.faces)
    face_index = dict(zip(face_sources, faces))
    edge_index = {tuple(sorted((e.verts[0].index, e.verts[1].index))): e for e in bm.edges}
    for name, field in [('edge_group_seam','seam'), ('edge_group_sharp','smooth')]:
        spec = next((a for a in request['edge_attributes'] if a['name'] == name), None)
        if spec is not None:
            for pair, value in zip(request['edges'], spec['values']):
                setattr(edge_index[tuple(pair)], field, not bool(value) if field == 'smooth' else bool(value))
    domain = request['domain']
    bm.select_mode = { {'point': 'VERT', 'edge': 'EDGE', 'face': 'FACE'}[domain] }
    for sequence in (bm.faces, bm.edges, bm.verts):
        for element in sequence: element.select = False
    for i in request['selected']:
        element = verts[i] if domain == 'point' else edge_index[tuple(request['edges'][i])] if domain == 'edge' else face_index.get(i)
        if element is not None:
            element.select_set(True)
            bm.select_history.add(element)
    bm.select_flush_mode()
    bm.normal_update()
    mesh = bpy.data.meshes.new('MiniHouDisposableMesh')
    bm.to_mesh(mesh); bm.free()
    obj = bpy.data.objects.new('MiniHouDisposableObject', mesh)
    bpy.context.collection.objects.link(obj)
    for other in bpy.context.selected_objects: other.select_set(False)
    obj.select_set(True); bpy.context.view_layer.objects.active = obj
    bpy.context.tool_settings.mesh_select_mode = tuple(domain == d for d in ('point', 'edge', 'face'))
    bpy.ops.object.mode_set(mode='EDIT')
    op = getattr(bpy.ops.mesh, operation)
    if not op.poll():
        raise ValueError(f'mesh.{operation} is unavailable in background edit mode')
    result = op('EXEC_DEFAULT', **request['options'])
    if 'FINISHED' not in result:
        raise ValueError(f'mesh.{operation} cancelled: check the selection and parameters')
    bm = bmesh.from_edit_mesh(mesh)
    bm.verts.ensure_lookup_table(); bm.verts.index_update(); bm.edges.index_update(); bm.faces.index_update()
    output_faces = list(bm.faces)
    loose = [e for e in bm.edges if not e.link_faces]
    primitives = [[v.index for v in f.verts] for f in output_faces] + [[v.index for v in e.verts] for e in loose]
    attributes = []
    for spec in specs:
        container = {'point': bm.verts, 'vertex': bm.loops, 'prim': bm.faces}[spec['domain']]
        kind = container.layers.float if spec['dtype'].startswith('float') else container.layers.int
        layers = [kind.get(n) for n in spec['layers']]
        if any(layer is None for layer in layers):
            raise ValueError(f'Blender removed attribute {spec["name"]!r}')
        elements = list(bm.verts) if spec['domain'] == 'point' else output_faces if spec['domain'] == 'prim' else [l for f in output_faces for l in f.loops]
        values = [[e[layer] for layer in layers] for e in elements]
        if spec['domain'] != 'point':
            values += [[0]*spec['components'] for _ in range(len(loose) * (2 if spec['domain'] == 'vertex' else 1))]
        if spec['name'] == 'uv' and spec['domain'] == 'vertex' and spec['components'] >= 2:
            uv_layer = bm.loops.layers.uv.get('MiniHouUV')
            if uv_layer is not None:
                for row, loop in zip(values, elements): row[:2] = list(loop[uv_layer].uv)
        if not spec['vector']: values = [v[0] for v in values]
        attributes.append({k: v for k,v in dict(spec, values=values).items() if k not in ('layers',)})
    if uv_spec is not None and uv_spec['domain'] == 'point':
        uv_layer = bm.loops.layers.uv.get('MiniHouUV')
        values = [list(l[uv_layer].uv) for f in output_faces for l in f.loops] + [[0.,0.] for _ in range(len(loose)*2)]
        attributes.append({'domain':'vertex','name':'uv','dtype':'float32','components':2,'vector':True,'values':values})
    if operation in ('faces_shade_flat','faces_shade_smooth'):
        attributes.append({'domain':'prim','name':'shade_smooth','dtype':'bool','components':1,'vector':False,'values':[f.smooth for f in output_faces]+[False]*len(loose)})
    edge_attributes = []
    output_edges = list(bm.edges)
    for spec in edge_specs:
        layer = bm.edges.layers.float.get(spec['layer'])
        if layer is None: raise ValueError(f'Blender removed edge attribute {spec["name"]!r}')
        edge_attributes.append({'name': spec['name'], 'dtype': spec['dtype'], 'values': [e[layer] for e in output_edges]})
    for name, op_name, values in [('edge_group_seam','mark_seam',[e.seam for e in output_edges]),
                                  ('edge_group_sharp','mark_sharp',[not e.smooth for e in output_edges])]:
        if operation == op_name or any(s['name'] == name for s in edge_specs):
            edge_attributes = [s for s in edge_attributes if s['name'] != name]
            edge_attributes.append({'name':name,'dtype':'bool','values':values})
    result_group = request.get('result_group', '')
    if result_group:
        name = 'group_'+result_group
        attributes = [a for a in attributes if a['name'] != name]
        attributes.append({'domain':'point','name':name,'dtype':'bool','components':1,'vector':False,'values':[v.select for v in bm.verts]})
        if output_faces:
            attributes.append({'domain':'prim','name':name,'dtype':'bool','components':1,'vector':False,'values':[f.select for f in output_faces]+[False]*len(loose)})
        edge_name = 'edge_group_'+result_group
        edge_attributes = [a for a in edge_attributes if a['name'] != edge_name]
        edge_attributes.append({'name':edge_name,'dtype':'bool','values':[e.select for e in output_edges]})
    output = {'version': bpy.app.version_string, 'points': [list(v.co) for v in bm.verts], 'primitives': primitives,
              'closed': [True]*len(output_faces)+[False]*len(loose), 'attributes': attributes,
              'edges': [sorted(v.index for v in e.verts) for e in output_edges], 'edge_attributes': edge_attributes}
    Path(dst).write_text(json.dumps(output, allow_nan=False), encoding='utf-8')


if __name__ == '__main__':
    args = sys.argv[sys.argv.index('--')+1:]
    main(*args)
