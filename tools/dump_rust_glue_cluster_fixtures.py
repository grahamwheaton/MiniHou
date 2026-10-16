"""Generate golden test fixtures for glue_cluster node."""
import json
import os
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))

from minihou import Evaluator, Graph
from minihou.sdk import REGISTRY


def encode(g):
    tables = {}
    for domain in ('point', 'vertex', 'prim'):
        tables[domain] = {
            name: {
                'width': 1 if a.ndim == 1 else a.shape[1],
                'dtype': 'float32' if a.dtype.kind == 'f' else 'int32',
                'data': a.reshape(-1).tolist()
            }
            for name, a in g.table(domain).items()
        }
    return dict(
        vp=g.topology.vertex_point.tolist(),
        offsets=g.topology.prim_offsets.tolist(),
        closed=g.topology.closed_mask().tolist(),
        detail=json.loads(json.dumps(g.detail, default=list)),
        **tables
    )


def main():
    ev = Evaluator()

    def get_asm(nodes, target='asm'):
        g = Graph.from_dict({'nodes': nodes, 'display': target})
        return ev.cook(g, target)

    asm_v = get_asm({
        'b': {'type': 'box', 'params': {'size': [2, 2, 2]}},
        'fp': {'type': 'voronoi_fracture_points', 'inputs': {'geo': 'b'}, 'params': {'count': 4, 'seed': 1}},
        'f': {'type': 'voronoi_fracture', 'inputs': {'geo': 'b', 'points': 'fp'}},
        'asm': {'type': 'assemble', 'inputs': {'geo': 'f'}},
    })

    asm_3b = get_asm({
        'b1': {'type': 'box', 'params': {'center': [0.5, 0.5, 0.5], 'size': [1, 1, 1]}},
        'b2': {'type': 'box', 'params': {'center': [1.5, 0.5, 0.5], 'size': [1, 1, 1]}},
        'b3': {'type': 'box', 'params': {'center': [2.5, 0.5, 0.5], 'size': [1, 1, 1]}},
        'm': {'type': 'merge', 'inputs': {'geos': ['b1', 'b2', 'b3']}},
        'asm': {'type': 'assemble', 'inputs': {'geo': 'm'}, 'params': {'source': 'connectivity', 'piece_attrib': ''}},
    })

    asm_gap = get_asm({
        'b1': {'type': 'box', 'params': {'center': [0.5, 0.5, 0.5], 'size': [1, 1, 1]}},
        'b2': {'type': 'box', 'params': {'center': [1.52, 0.5, 0.5], 'size': [1, 1, 1]}},
        'm': {'type': 'merge', 'inputs': {'geos': ['b1', 'b2']}},
        'asm': {'type': 'assemble', 'inputs': {'geo': 'm'}, 'params': {'source': 'connectivity', 'piece_attrib': ''}},
    })

    asm_single = get_asm({
        'b': {'type': 'box', 'params': {'size': [1, 1, 1]}},
        'asm': {'type': 'assemble', 'inputs': {'geo': 'b'}, 'params': {'source': 'connectivity', 'piece_attrib': ''}},
    })

    cases = [
        {
            'name': 'voronoi_constraints',
            'input': encode(asm_v),
            'params': {'output': 'constraints', 'strength': 5e5, 'strength_variation': 0.25, 'seed': 7, 'clusters': 2, 'weak': 0.8, 'outgrp_internal': True, 'outgrp_external': True, 'outgrp_weak': True},
        },
        {
            'name': 'voronoi_pieces',
            'input': encode(asm_v),
            'params': {'output': 'pieces', 'clusters': 2, 'outgrp_glued': True, 'outgrp_loose': True, 'outgrp_interface': True},
        },
        {
            'name': 'three_boxes_constraints_nocluster',
            'input': encode(asm_3b),
            'params': {'output': 'constraints', 'strength': 1000.0, 'strength_variation': 0.2, 'seed': 42, 'clusters': 0, 'weak': 0.9, 'outgrp_internal': True, 'outgrp_external': True, 'outgrp_weak': True},
        },
        {
            'name': 'three_boxes_pieces_cluster2',
            'input': encode(asm_3b),
            'params': {'output': 'pieces', 'clusters': 2, 'outgrp_glued': True, 'outgrp_loose': True, 'outgrp_interface': True},
        },
        {
            'name': 'gapped_boxes_no_gap',
            'input': encode(asm_gap),
            'params': {'output': 'constraints', 'max_gap': 0.0, 'outgrp_internal': True, 'outgrp_external': True},
        },
        {
            'name': 'gapped_boxes_with_gap',
            'input': encode(asm_gap),
            'params': {'output': 'constraints', 'max_gap': 0.05, 'outgrp_internal': True, 'outgrp_external': True},
        },
        {
            'name': 'single_box_constraints',
            'input': encode(asm_single),
            'params': {'output': 'constraints', 'outgrp_internal': True, 'outgrp_external': True, 'outgrp_weak': True},
        },
        {
            'name': 'single_box_pieces',
            'input': encode(asm_single),
            'params': {'output': 'pieces', 'outgrp_glued': True, 'outgrp_loose': True, 'outgrp_interface': True},
        },
    ]

    nt = REGISTRY.get('glue_cluster')
    for c in cases:
        p = nt.coerce_params(c['params'])
        in_geo = asm_v if 'voronoi' in c['name'] else (asm_3b if 'three_boxes' in c['name'] else (asm_gap if 'gapped' in c['name'] else asm_single))
        out = nt.fn(in_geo, **p)
        c['expected'] = encode(out)

    out_path = ROOT / 'rust' / 'crates' / 'minihou-core' / 'src' / 'nodes' / 'test_data' / 'glue_cluster_golden.json'
    out_path.parent.mkdir(parents=True, exist_ok=True)
    out_path.write_text(json.dumps(cases, indent=2), encoding='utf-8')
    print(f'Wrote {len(cases)} cases to {out_path}')


if __name__ == '__main__':
    main()
