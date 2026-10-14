"""Selected native Blender modelling operations, plus standalone mesh transforms."""
import json
import math

import numpy as np

from .. import groups
from ..blender_mesh import execute, runtime_key
from ..blender_mesh_catalog import OPERATORS
from ..geometry import Geometry
from ..sdk import Bool, Enum, Float, Input, Int, Str, Vec3, node


def _move(geo, P):
    out = geo.copy()
    out.point['P'] = np.asarray(P, np.float32)
    for domain in ('point', 'vertex', 'prim'):
        table = getattr(out, domain)
        if 'N' in table: del table['N']
    return out


@node('to_sphere', inputs=['geo'], category='deform', params=[
    Str('group', ''), Float('factor', 1, min=0, max=1), Float('radius', 0, min=0, doc='Zero uses mean selected distance'),
    Vec3('center', (0, 0, 0)), Bool('use_centroid', True)],
    doc='Blend selected points toward a sphere. No Blender dependency. Points at the centre remain fixed.')
def to_sphere(geo, group, factor, radius, center, use_centroid):
    sel = groups.mask(geo, 'point', group)
    ids = np.arange(geo.npoints) if sel is None else np.flatnonzero(sel)
    if not len(ids) or factor == 0: return geo.copy()
    P = geo.P.astype(np.float64).copy()
    c = P[ids].mean(axis=0) if use_centroid else np.asarray(center, np.float64)
    v = P[ids]-c; lengths = np.linalg.norm(v, axis=1)
    r = radius if radius > 0 else float(lengths.mean())
    valid = lengths > 1e-12
    P[ids[valid]] += factor * (v[valid]*r/lengths[valid,None]-v[valid])
    return _move(geo, P)


@node('shear', inputs=['geo'], category='deform', params=[
    Str('group', ''), Enum('axis', ['x', 'y', 'z']), Enum('direction', ['y', 'x', 'z']),
    Float('factor', .5, soft_min=-2, soft_max=2), Vec3('center', (0, 0, 0))],
    doc='Offset one axis in proportion to another about a pivot. No Blender dependency; normal attributes are invalidated.')
def shear(geo, group, axis, direction, factor, center):
    if axis == direction: raise ValueError('Shear directions must differ')
    P = geo.P.copy(); sel = groups.mask(geo, 'point', group)
    ids = np.arange(geo.npoints) if sel is None else np.flatnonzero(sel)
    a, d = 'xyz'.index(axis), 'xyz'.index(direction)
    P[ids,a] += factor*(P[ids,d]-center[d])
    return _move(geo, P)


@node('blender_mesh', inputs=[Input('geo', optional=True)], category='model', key_extra=runtime_key, params=[
    Enum('operation', OPERATORS, default='remove_doubles'), Enum('domain', ['point','edge','face'], default='point'),
    Str('group', ''), Str('result_group','native_selection',doc='Name for native selected-result groups; empty disables them'), Str('options', '{}', kind='code', doc='Native Blender RNA keyword arguments as JSON; no Python code')],
    doc='Execute an allowlisted native Blender mesh operator on a disposable mesh. Requires Blender on PATH or BLENDER_EXE. Advanced headless operations; interactive Knife/Rip and scene operations are excluded.')
def blender_mesh(geo, operation, domain, group, result_group, options):
    try: options = json.loads(options)
    except (ValueError, TypeError) as e: raise ValueError('Native Blender options must be valid JSON') from e
    if not isinstance(options, dict): raise ValueError('Native Blender options must be a JSON object')
    return execute(geo if geo is not None else Geometry.empty(), operation, domain, group, options, result_group)


# Common tools expose editable typed controls instead of requiring operator JSON.
TOOLS = [
    ('merge_vertices', 'merge', 'point', [Enum('method',['CENTER','COLLAPSE'])], lambda p: {'type':p['method']}, 'Merge selected vertices at their centre or collapse connected edges.'),
    ('merge_by_distance', 'remove_doubles', 'point', [Float('distance', .0001, min=0), Bool('use_unselected',False)], lambda p: {'threshold':p['distance'],'use_unselected':p['use_unselected']}, 'Merge nearby selected vertices using native Blender topology.'),
    ('split_geometry', 'split', 'face', [], lambda p: {}, 'Disconnect selected geometry from its neighbours without deleting it.'),
    ('split_edges', 'edge_split', 'edge', [], lambda p: {}, 'Split selected edges and duplicate the needed vertices.'),
    ('connect_vertices', 'vert_connect_path', 'point', [], lambda p: {}, 'Connect selected vertices across faces. Select at least two vertices.'),
    ('make_faces', 'edge_face_add', 'point', [], lambda p: {}, 'Make an edge or polygon from selected vertices/boundary edges.'),
    ('grid_fill', 'fill_grid', 'edge', [Int('span',1,min=1),Int('offset',0),Bool('simple',False)], lambda p: {'span':p['span'],'offset':p['offset'],'use_interp_simple':p['simple']}, 'Fill a suitable closed boundary with a quad grid.'),
    ('native_solidify', 'solidify', 'face', [Float('thickness',.1,soft_min=-1,soft_max=1)], lambda p: {'thickness':p['thickness']}, 'Give selected faces thickness using Blender Solidify Faces (not the Solidify modifier).'),
    ('poke_faces', 'poke', 'face', [Float('offset',0)], lambda p: {'offset':p['offset']}, 'Insert a centre vertex and triangulate selected faces.'),
    ('triangles_to_quads', 'tris_convert_to_quads', 'face', [Float('face_angle',40,min=0,max=180),Float('shape_angle',40,min=0,max=180),Bool('compare_uvs',False),Bool('compare_materials',True)], lambda p: {'face_threshold':math.radians(p['face_angle']),'shape_threshold':math.radians(p['shape_angle']),'uvs':p['compare_uvs'],'materials':p['compare_materials']}, 'Join compatible adjacent selected triangles into quads. Corner or point uv attributes also initialise a native UV layer for seam comparison.'),
    ('limited_dissolve', 'dissolve_limited', 'edge', [Float('angle',5,min=0,max=180),Bool('dissolve_boundaries',False)], lambda p: {'angle_limit':math.radians(p['angle']),'use_dissolve_boundaries':p['dissolve_boundaries']}, 'Dissolve nearly planar edges using an angle limit.'),
    ('symmetrize', 'symmetrize', 'point', [Enum('direction',['NEGATIVE_X','POSITIVE_X','NEGATIVE_Y','POSITIVE_Y','NEGATIVE_Z','POSITIVE_Z']),Float('distance',.0001,min=0)], lambda p: {'direction':p['direction'],'threshold':p['distance']}, 'Cut and mirror a mesh from one side to the other, welding at the symmetry plane.'),
    ('spin', 'spin', 'point', [Int('steps',8,min=1,max=256),Float('angle',360),Vec3('center',(0,0,0)),Vec3('axis',(0,0,1)),Bool('duplicate',False)], lambda p: {'steps':p['steps'],'angle':math.radians(p['angle']),'center':p['center'],'axis':p['axis'],'dupli':p['duplicate']}, 'Spin selected vertices/edges around an axis, connecting steps or duplicating them.'),
    ('native_bevel', 'bevel', 'edge', [Float('amount',.1,min=0),Int('segments',1,min=1,max=32),Float('profile',.5,min=0,max=1),Bool('clamp_overlap',True)], lambda p: {'offset':p['amount'],'segments':p['segments'],'profile':p['profile'],'clamp_overlap':p['clamp_overlap']}, 'Bevel selected edges using Blender itself.'),
    ('native_extrude', 'extrude_region_move', 'face', [Vec3('offset',(0,0,.3))], lambda p: {'TRANSFORM_OT_translate':{'value':p['offset']}}, 'Extrude a selected face region and translate it by a world-space vector.'),
    ('native_inset', 'inset', 'face', [Float('amount',.1,min=0),Float('depth',0),Bool('individual',False)], lambda p: {'thickness':p['amount'],'depth':p['depth'],'use_individual':p['individual']}, 'Inset selected faces with Blender topology, including connected regions.'),
    ('native_bisect', 'bisect', 'face', [Vec3('origin',(0,0,0)),Vec3('normal',(0,0,1)),Bool('fill',False),Bool('clear_inner',False),Bool('clear_outer',False)], lambda p: {'plane_co':p['origin'],'plane_no':p['normal'],'use_fill':p['fill'],'clear_inner':p['clear_inner'],'clear_outer':p['clear_outer']}, 'Cut selected mesh geometry with a plane; optionally fill and remove either side.'),
    ('native_wireframe', 'wireframe', 'face', [Float('thickness',.02,min=0),Bool('replace',True)], lambda p: {'thickness':p['thickness'],'use_replace':p['replace']}, 'Turn selected mesh edges into solid wire strips.'),
]


def _register_tool(name, operation, domain, params, convert, doc):
    def cook(geo, group, result_group, **values):
        return execute(geo, operation, domain, group, convert(values), result_group)
    node(name, inputs=['geo'], params=[Str('group',''),Str('result_group','native_selection',doc='Selected-result point/face/edge groups; empty disables them')]+params, category='model', key_extra=runtime_key,
         doc=doc+' Requires Blender on PATH or BLENDER_EXE; input geometry is never edited.')(cook)


for _spec in TOOLS: _register_tool(*_spec)
