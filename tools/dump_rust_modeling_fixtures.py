"""Refresh Rust modeling unit-test oracles from the Python implementation (no Rust build)."""
import json
import sys
from pathlib import Path
import numpy as np
ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
import minihou
from minihou import accel, rust
from minihou.geometry import Geometry
from minihou.sdk import REGISTRY
from minihou.nodes.normal import normal
from minihou import edges


def encode(g):
    tables = {}
    for domain in ('point', 'vertex', 'prim'):
        tables[domain] = {name: {'width': 1 if a.ndim == 1 else a.shape[1],
                                'dtype': 'float32' if a.dtype.kind == 'f' else 'int32',
                                'data': a.reshape(-1).tolist()}
                          for name, a in g.table(domain).items()}
    return dict(vp=g.topology.vertex_point.tolist(), offsets=g.topology.prim_offsets.tolist(),
                closed=g.topology.closed_mask().tolist(), detail=dict(g.detail), **tables)


def source(kind):
    if kind == 'quad':
        g = Geometry.from_arrays(np.array([[0,0,0],[2,0,0],[2,2,0],[0,2,0]],np.float32), [0,1,2,3], [0,4])
    elif kind == 'concave':
        g = Geometry.from_arrays(np.array([[0,0,0],[2,0,0],[2,1,0],[1,1,0],[1,2,0],[0,2,0]],np.float32), list(range(6)), [0,6])
    elif kind == 'profiles':
        g = Geometry.from_arrays(np.array([[0,0,0],[2,0,0],[0,2,0],[2,2,0]],np.float32),[0,1,2,3],[0,2,4],closed=[False,False])
    elif kind == 'triangles':
        g = Geometry.from_arrays(np.array([[0,0,0],[2,0,0],[2,2,0],[0,2,0]],np.float32),[0,1,2,0,2,3],[0,3,6])
    else:
        spec = REGISTRY.get('box' if kind == 'box' else 'grid')
        opts = {'divisions': 2} if kind == 'box' else {'columns': 3, 'rows': 3, 'size_x':2, 'size_y':2}
        g = spec.fn(**spec.coerce_params(opts))
    g = g.copy()
    g.point['temperature'] = np.linspace(.25,1.75,g.npoints).astype(np.float32)
    g.point['id'] = np.arange(g.npoints,dtype=np.int32)
    g.point['group_pick'] = (np.arange(g.npoints)%2==0).astype(np.int32)
    g.vertex['uv_test'] = (np.arange(g.nvertices*2,dtype=np.float32)/7).reshape(-1,2)
    g.vertex['corner_id'] = np.arange(g.nvertices,dtype=np.int32)
    g.prim['material_id'] = np.arange(g.nprims,dtype=np.int32)%2
    g.prim['group_faces'] = (np.arange(g.nprims)%2==0).astype(np.int32)
    g.detail = {'reference': 'modeling Python oracle', 'version': 1}
    return g


def main():
    jobs = []
    def add(ty,kind='quad',params=None,**extra): jobs.append((ty,kind,params or {},extra))
    add('reverse'); add('reverse','grid',{'group':'faces'});add('reverse','grid',{'group':'pick'})
    add('facet',params={'compute_normals':True});add('facet','grid',{'unique_points':False,'compute_normals':True})
    add('divide');add('divide','concave');add('divide','profiles')
    add('mirror',params={'origin':[0,0,0],'normal':[1,0,0],'outgrp_seam':True,'outgrp_mirroredpoints':True});add('mirror','grid',{'keep_original':False})
    add('clip',params={'origin':[1,0,0],'normal':[1,0,0],'keep':'both','outgrp_below':True,'outgrp_cutpolys':True,'outgrp_cutpoints':True,'outgrp_cutedges':True})
    add('clip','grid',{'normal':[1,1,0]});add('polyfill',params={'fill_group':'cap'});add('polyfill','box')
    add('polyextrude',params={'distance':.7,'output_back':True,'outgrp_back':True})
    add('polyextrude','box',{'group':'0','distance':-.2});add('polyextrude','grid',{'group':'0 1','mode':'individual','inset':.1})
    add('polyextrude','grid',{'distance':0});add('polyextrude','grid',{'inset':.1})
    add('inset');add('outset');add('inset','quad',{'amount':1.1},error=True)
    add('solidify',params={'outgrp_top':True,'outgrp_bottom':True,'outgrp_side':True})
    add('solidify','box',{'thickness':-.1,'offset':.3});add('solidify','profiles');add('solidify','grid',{'rim':False,'even':False,'group':'0'})
    add('crease',params={'group':'p0-1','sharpness':2.5});add('crease',params={'domain':'points','group':'0 2','sharpness':.5})
    add('edge_group',params={'method':'boundary'});add('edge_group','triangles',{'method':'pattern','pattern':'p0-2'})
    add('edge_group','grid',{'method':'points','pattern':'pick'});add('edge_group','grid',{'method':'primitives','pattern':'faces'})
    add('edge_group_promote',params={'to':'points'});add('edge_group_promote','triangles',{'pattern':'p0-2','to':'primitives'})
    add('polybridge','profiles',{'source':'p0-1','destination':'p2-3','segments':3,'outgrp_bridge':True})
    add('dissolve_edges','triangles',{'group':'p0-2','preserve_seams':False});add('dissolve_edges','triangles',{'group':'p0-2'},error=True)
    add('subdivide',params={'algorithm':'linear'});add('subdivide','grid',{'depth':2});add('subdivide','box',{'depth':2})
    add('subdivide','box',{'creases':'p0-1','crease_weight':2.5,'override_crease_weight':True,'outgrp_creases':True})
    add('twist','box',{'strength':57},normals=True);add('linear_taper','box',{'direction':[1,1,0],'up':[1,1,0],'symmetric':True},normals=True)
    add('squash_stretch','box',{'amount':2,'direction':[1,2,3]},normals=True);add('soft_peak','grid',{'group':'4','radius':1.1})
    add('group_range','grid',{'domain':'points','every':3,'select':2,'offset':-1,'outgrp_unused':False})
    jobs[-1][2].pop('outgrp_unused')
    add('edge_collapse','grid',{'group':'p1-4','outgrp_collapsed':True});add('edge_flip','triangles',{'group':'p0-2','outgrp_flipped':True})
    add('edge_cusp','box',{'cusp_angle':30});add('polysplit',params={'split_edges':'p0-1','outgrp_points':True,'outgrp_edges':True})
    add('polysplit','grid',{'split_edges':'p0-1','loops':3});add('polysplit',params={'mode':'points','split_points':'0-2','outgrp_edges':True})
    add('skin_profiles','profiles');add('polyexpand2d',params={'mode':'inside_outside','keep_input':True,'outattr_edgedist':True,'outattr_edgespeed':True,'outgrp_inside':True,'outgrp_outside':True})
    add('polyexpand2d','concave',error=True)
    for divisions in (1,2,3,4):add('polybevel','box',{'divisions':divisions,'outgrp_pointfilletpolys':True,'outgrp_offsetedges':True,'outgrp_offsetpoints':True})
    add('polybevel','box',{'edges':'0-1','divisions':2});add('polybevel','grid',{'distance':0,'outgrp_offsetpoints':True})
    cuts=json.dumps([dict(matrix=[1,0,0,0,0,1,0,0,0,0,1,0,0,0,0,1],a=[1,-1],b=[1,3],through=True)])
    # 61_knife_quad is overridden by the Rust knife fix; preserve its expected block after regeneration.
    add('knife',params={'cuts':cuts});add('knife','triangles',{'cuts':cuts})
    out=[]
    with rust.disabled(),accel.disabled():
        for i,(ty,kind,params,extra) in enumerate(jobs):
            g=source(kind)
            if extra.get('normals'):
                g=normal(g,'point')
                g.vertex['N']=np.tile([0,0,1],(g.nvertices,1)).astype(np.float32)
                g.prim['N']=np.tile([1,0,0],(g.nprims,1)).astype(np.float32)
            spec=REGISTRY.get(ty)
            kwargs=spec.coerce_params(params)
            case=dict(name=f'{i:02d}_{ty}_{kind}',type=ty,params=params,defaults={p.name:p.to_json_value(p.default) for p in spec.params},inputs=[encode(g)])
            try:
                result=spec.fn(g,**kwargs)
                if extra.get('error'):raise AssertionError('expected Python error')
                result.validate();case['expected']=encode(result)
            except ValueError as exc:
                if not extra.get('error'):raise
                case['error']=str(exc)
            out.append(case)
    dest=ROOT/'rust/crates/minihou-core/src/nodes/test_data/modeling_parity.json'
    dest.parent.mkdir(exist_ok=True)
    dest.write_text(json.dumps(out,separators=(',',':'))+'\n',encoding='utf-8')
    print(f'wrote {len(out)} Python modeling oracles to {dest}')

if __name__=='__main__':main()
