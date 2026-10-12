"""Cook the modelling workflows; --write also regenerates their editor JSON graphs.

Works with the bundled runtime even when its _pth names another checkout.
"""
import json
import pathlib
import sys

ROOT = pathlib.Path(__file__).resolve().parents[1]
sys.path.insert(0,str(ROOT))
from minihou import Evaluator,Graph


def node(kind,inputs=None,**params):
    return {'type':kind,'inputs':inputs or {},'params':params}


WORKFLOWS = {
    'modeling_bridge': ({
        'bottom':node('circle',divisions=12),
        'flip':node('reverse',{'geo':'bottom'}),
        'top':node('circle',divisions=12,center=[0,0,2]),
        'merge':node('merge',{'geos':['flip','top']}),
        'source':node('edge_group',{'geo':'merge'},name='bottom',method='primitives',pattern='0'),
        'target':node('edge_group',{'geo':'source'},name='top',method='primitives',pattern='1'),
        'bridge':node('polybridge',{'geo':'target'},source='bottom',destination='top',segments=4),
        'normal':node('normal',{'geo':'bridge'}),
    },'bridge','segments'),
    'modeling_dissolve': ({
        'grid':node('grid',columns=4,rows=4),
        'boundary':node('edge_group',{'geo':'grid'},name='boundary',method='boundary'),
        'dissolve':node('dissolve_edges',{'geo':'boundary'},group='^boundary'),
    },'dissolve','angle'),
    'modeling_skin': ({
        'low':node('circle',divisions=16),
        'middle':node('circle',divisions=16,radius=.6,center=[0,0,1]),
        'high':node('circle',divisions=16,radius=.9,center=[0,0,2]),
        'profiles':node('merge',{'geos':['low','middle','high']}),
        'skin':node('skin_profiles',{'geo':'profiles'}),
        'normal':node('normal',{'geo':'skin'}),
    },'middle','radius'),
    'modeling_expand2d': ({
        'profile':node('circle',divisions=6,rotate=[25,15,0]),
        'offset':node('polyexpand2d',{'geo':'profile'},distance=.25),
    },'offset','distance'),
    'modeling_soft_transform': ({
        'grid':node('grid',columns=20,rows=20,size_x=4,size_y=4),
        'soft':node('soft_transform',{'geo':'grid'},radius=2,translate=[0,0,1]),
        'normal':node('normal',{'geo':'soft'}),
    },'soft','translate'),
    'modeling_lattice': ({
        'box':node('box',divisions=5),
        'rest':node('lattice_grid',columns=3,rows=3,layers=3),
        'edited':node('soft_transform',{'geo':'rest'},center=[0,0,1],radius=2,translate=[.7,0,.3]),
        'lattice':node('lattice',{'geo':'box','rest':'rest','deformed':'edited'}),
        'normal':node('normal',{'geo':'lattice'}),
    },'edited','translate'),
    'modeling_point_deform': ({
        'sphere':node('sphere',rows=8,columns=12),
        'rest':node('box'),
        'edited':node('transform',{'geo':'rest'},rotate=[20,10,30],scale=[1,1,1.5]),
        'deform':node('point_deform',{'geo':'sphere','rest':'rest','deformed':'edited'}),
        'normal':node('normal',{'geo':'deform'}),
    },'edited','rotate'),
    'modeling_creases': ({
        'box':node('box'),
        'crease':node('crease',{'geo':'box'},sharpness=1.5),
        'subdivide':node('subdivide',{'geo':'crease'},depth=3),
        'normal':node('normal',{'geo':'subdivide'}),
    },'crease','sharpness'),
}


def graphs():
    for name,(nodes,exposed_node,param) in WORKFLOWS.items():
        for i,n in enumerate(nodes.values()):
            n['pos']=[(i%3)*240,(i//3)*160]
        yield name,{'version':1,'doc':'Initial modelling workflow; see docs/modeling_tools.md for limits.',
                    'nodes':nodes,'display':list(nodes)[-1],
                    'exposed':[{'name':param.replace('_',' ').title(),'node':exposed_node,'param':param}]}


if __name__=='__main__':
    for name,data in graphs():
        geo=Evaluator().cook(Graph.from_dict(data))
        geo.validate()
        print(f'{name}: {geo.npoints} points, {geo.nprims} primitives')
        if '--write' in sys.argv:
            (ROOT/'examples'/'graphs'/f'{name}.json').write_text(json.dumps(data,indent=2)+'\n',encoding='utf-8')
