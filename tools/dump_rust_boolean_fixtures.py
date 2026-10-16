"""Refresh Boolean / PolyReduce / PushPull Rust oracles from pure Python.

Inputs are serialized so these tests isolate the ports from unrelated generator,
wrangle and transform ports. No Rust executable or editor is used.
"""
import json
import sys
from pathlib import Path
import numpy as np

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
import minihou
from minihou import accel, rust, Evaluator, Graph
from minihou.sdk import REGISTRY
from minihou.nodes.bevel import twins
from minihou.nodes._util import fan_triangulate
from dump_rust_modeling_fixtures import encode


def run(ty, *inputs, **params):
    spec = REGISTRY.get(ty)
    return spec.fn(*inputs, **spec.coerce_params(params))


def stats(g):
    tris, _ = fan_triangulate(g.topology)
    t = g.P[g.topology.vertex_point[tris]].astype(np.float64)
    volume = float(np.einsum('ij,ij->i', t[:, 0], np.cross(t[:, 1], t[:, 2])).sum() / 6)
    from minihou import topo
    edges = topo.edges(g)
    links = [[] for _ in range(g.npoints)]
    for f in range(g.nprims):
        vs = g.topology.vertex_point[g.topology.prim_offsets[f]:g.topology.prim_offsets[f+1]]
        for k, v in enumerate(vs):
            links[v].append((int(vs[k-1]), int(vs[(k+1) % len(vs)])))
    vertex_manifold = True
    for link in links:
        adj = {}
        for a, b in link:
            adj.setdefault(a, []).append(b)
            adj.setdefault(b, []).append(a)
        if not adj:
            continue
        seen, todo = set(), [next(iter(adj))]
        while todo:
            v = todo.pop()
            if v not in seen:
                seen.add(v)
                todo.extend(adj[v])
        vertex_manifold &= len(seen) == len(adj) and all(len(v) <= 2 for v in adj.values())
        vertex_manifold &= sum(len(v) == 1 for v in adj.values()) in (0, 2)
    return dict(points=g.npoints, faces=g.nprims, volume=volume,
                manifold=bool(np.all(twins(g) >= 0)),
                vertex_manifold=bool(vertex_manifold), boundary_edges=int((edges.count == 1).sum()),
                nonmanifold_edges=int((edges.count > 2).sum()),
                euler=g.npoints - len(edges.points) + g.nprims)


def decorated(g, side=0):
    g = g.copy()
    g.point['temperature'] = (g.P[:, 2] + 0.25 + side).astype(np.float32)
    g.point['id'] = np.arange(g.npoints, dtype=np.int32) + 100 * side
    g.vertex['uv_test'] = np.arange(g.nvertices * 2, dtype=np.float32).reshape(-1, 2) / 7
    g.vertex['corner_id'] = np.arange(g.nvertices, dtype=np.int32)
    g.prim['source_id'] = np.arange(g.nprims, dtype=np.int32) + 100 * side
    g = run('normal', g, domain='point')
    g.vertex['N'] = np.tile([0, 0, 1], (g.nvertices, 1)).astype(np.float32)
    g.prim['N'] = np.tile([0, 1, 0], (g.nprims, 1)).astype(np.float32)
    return g


def main():
    cases = []
    def add(name, ty, inputs, params=None, error=False):
        params = params or {}
        spec = REGISTRY.get(ty)
        c = dict(name=name, type=ty, inputs=[encode(g) for g in inputs], params=params,
                 defaults={p.name: p.to_json_value(p.default) for p in spec.params})
        try:
            g = run(ty, *inputs, **params)
            if error:
                raise AssertionError('expected Python rejection: ' + name)
            g.validate()
            c.update(expected=encode(g), stats=stats(g))
            print(name, c['stats'], flush=True)
        except (ValueError, RuntimeError) as exc:
            if not error:
                raise
            c['error'] = str(exc)
        cases.append(c)

    with rust.disabled(), accel.disabled():
        box = run('box', size=[2, 2, 2])
        pairs = [
            ('offset', box, run('box', size=[2, 2, 2], center=[1, 1, 1])),
            ('flush', box, run('box', size=[1, 1, 2], center=[.5, .5, 0])),
            ('nested', box, run('box', size=[1, 1, 1])),
            ('apart', box, run('box', center=[4, 0, 0])),
            ('identical', box, box),
            ('face_touch', box, run('box', size=[2, 2, 2], center=[2, 0, 0])),
            ('edge_touch', box, run('box', size=[2, 2, 2], center=[2, 2, 0])),
            ('point_touch', box, run('box', size=[2, 2, 2], center=[2, 2, 2])),
            ('sphere_box', box, run('sphere', radius=1.2, rows=12, columns=24)),
            ('coplanar_ring', box, run('sphere', radius=.6, rows=16, columns=32, center=[.2, .1, 1.])),
            ('bore_hole', run('box', size=[2, 2, 2], divisions=2),
             run('tube', radius_bottom=.1, radius_top=.1, height=3, columns=12, center=[.6, -.4, 0])),
            ('torus_holes', box, run('torus', rows=8, columns=20, center=[.3, .2, .9])),
        ]
        for name, a, b in pairs:
            for op in ('union', 'intersect', 'subtract'):
                add(name + '_' + op, 'boolean', [a, b], dict(operation=op))
        a, b = decorated(box), decorated(pairs[0][2], 1)
        a.prim['only_a'] = np.arange(a.nprims, dtype=np.int32) + 1
        a = run('set_material', a, material='Red')
        b = run('set_material', b, material='Blue')
        a.detail['usd_materials'] = {'Red': {'path': '/Red'}}
        b.detail['usd_materials'] = {'Blue': {'path': '/Blue'}}
        for op in ('union', 'intersect', 'subtract'):
            add('attributes_' + op, 'boolean', [a, b], dict(operation=op, outgrp_seam=True,
                outgrp_seampoints=True, outgrp_from_a_name='left', outgrp_from_b_name='right'))
        add('disabled_groups', 'boolean', [box, pairs[0][2]], dict(outgrp_from_a=False, outgrp_from_b_name=''))
        from minihou.geometry import Geometry
        empty = Geometry.from_arrays(np.zeros((0, 3), np.float32), [], [0])
        outline = np.array([[0, 0], [2, 0], [2, 1], [1, 1], [1, 2], [0, 2]], np.float32)
        positions = np.concatenate([np.c_[outline, np.full(6, -1)], np.c_[outline, np.ones(6)]])
        faces = [list(reversed(range(6))), list(range(6, 12))]
        faces += [[i, (i+1) % 6, (i+1) % 6 + 6, i+6] for i in range(6)]
        lprism = Geometry.from_arrays(positions, np.concatenate(faces), np.r_[0, np.cumsum([len(f) for f in faces])])
        bent = box.copy()
        bent.point['P'] = bent.P.copy()
        bent.P[0, 2] += .2
        for name, src, cutter in [('concave_caps', lprism, run('box', center=[.75, .75, .4], size=[.8, .8, 3])),
                                  ('bent_polygons', bent, pairs[0][2])]:
            for op in ('union', 'intersect', 'subtract'):
                add(name + '_' + op, 'boolean', [src, cutter], dict(operation=op))
        for op in ('union', 'intersect', 'subtract'):
            for swap in (False, True):
                add(f'empty_{op}_{swap}', 'boolean', [empty, box] if swap else [box, empty], dict(operation=op))
        add('boolean_open_error', 'boolean', [run('helix'), box], error=True)
        rng = np.random.default_rng(5)
        for trial in range(6):
            a = run('noise', run('sphere', rows=10, columns=20), amplitude=[.15]*3, seed=trial)
            b = run('transform', run('torus', rows=8, columns=20),
                    rotate=rng.uniform(0, 360, 3).tolist(), translate=rng.uniform(-.8, .8, 3).tolist())
            for op in ('union', 'intersect', 'subtract'):
                add(f'random_{trial}_{op}', 'boolean', [a, b], dict(operation=op))
        # Shipped multi-hole/chain example, each Boolean's actual Python inputs.
        graph = Graph.load(ROOT / 'examples/graphs/boolean_part.json')
        evaluator = Evaluator()
        try:
            for name in ('cut', 'joined'):
                node = graph.nodes[name]
                inputs = [evaluator.cook(graph, node.inputs[port][0] if isinstance(node.inputs[port], list) else node.inputs[port]) for port in ('a', 'b')]
                add('part_' + name, 'boolean', inputs, node.params)
        finally:
            evaluator.close()
        for opts in ({'percentage': 10}, {'target': 200}, {'percentage': 100}, {'percentage': 0}):
            add('reduce_sphere_' + str(opts), 'polyreduce', [decorated(run('sphere', rows=16, columns=32))], opts)
        add('reduce_box_flat', 'polyreduce', [decorated(run('box', divisions=10))], dict(percentage=10))
        for keep in (True, False):
            add('reduce_grid_' + str(keep), 'polyreduce', [decorated(run('grid', columns=20, rows=20))], dict(percentage=5, keep_borders=keep))
        add('reduce_torus_attributes', 'polyreduce', [decorated(run('torus', rows=12, columns=24))], dict(percentage=20))
        for opts in ({'percentage': 10}, {'target': 200}):
            add('reduce_dense_sphere_' + str(opts), 'polyreduce', [decorated(run('sphere', rows=48, columns=96))], opts)
        add('reduce_dense_grid', 'polyreduce', [decorated(run('grid', rows=40, columns=40))], dict(percentage=5))
        add('reduce_empty', 'polyreduce', [empty])
        add('reduce_open_error', 'polyreduce', [run('helix')], error=True)
        # Two independent cutters and the remainder, plus overlapping cutters.
        c1 = run('sphere', radius=.7, rows=10, columns=20, center=[.6, 0, 0])
        c2 = run('box', size=[.8, 3, .8], center=[-.6, 0, 0])
        from minihou.nodes.merge import merge
        cutters = merge([c1, c2])
        for keep in ('both', 'inside', 'outside'):
            for mode in ('each_cutter', 'union'):
                add(f'fracture_{keep}_{mode}', 'boolean_fracture', [box, cutters], dict(keep=keep, mode=mode))
        add('fracture_groups', 'boolean_fracture', [box, cutters], dict(outgrp_outside=True,
            outgrp_cutter=True, outgrp_remainder=True, outgrp_cutpoints=True, outgrp_cutedges=True))
        add('fracture_no_attributes', 'boolean_fracture', [box, cutters], dict(piece_attrib='', cutter_attrib='', outgrp_inside=False))
        overlap = merge([c1, run('sphere', radius=.7, rows=10, columns=20, center=[.9, 0, 0])])
        add('fracture_overlap', 'boolean_fracture', [box, overlap])
        add('fracture_miss', 'boolean_fracture', [box, run('box', center=[9, 0, 0])])
        add('fracture_empty', 'boolean_fracture', [box, empty])
        add('fracture_open_error', 'boolean_fracture', [run('helix'), cutters], error=True)
        colored = merge([run('set_material', decorated(c1), material='Blue'),
                         run('set_material', decorated(c2, 1), material='Green')])
        add('fracture_material_attributes', 'boolean_fracture',
            [run('set_material', decorated(box), material='Red'), colored], dict(outgrp_cutedges=True))
        for distance in (.4, -.4, -3, 0):
            add('pushpull_' + str(distance), 'pushpull', [box], dict(group='1', distance=distance))
        inset = run('inset', box, group='1', amount=.4)
        for distance in (-.4, -3):
            add('pushpull_inset_' + str(distance), 'pushpull', [inset], dict(group='extrudeFront', distance=distance))
        rotated = run('transform', box, rotate=[23, 31, 17], translate=[2, -3, 1])
        for face in range(6):
            add('pushpull_rotated_' + str(face), 'pushpull', [rotated], dict(group=str(face), distance=-.25))
        for name, g, group in [('open', run('grid'), '0'), ('all', box, ''), ('multiple', box, '0 1'),
                                ('inward', run('reverse', box), '0'), ('concave', lprism, '1')]:
            add('pushpull_reject_' + name, 'pushpull', [g], dict(group=group), error=True)
    dest = ROOT / 'rust/crates/minihou-core/src/nodes/test_data/boolean_parity.json'
    dest.write_text(json.dumps(cases, separators=(',', ':')) + '\n', encoding='utf-8')
    print(f'Wrote {len(cases)} cases to {dest}')


if __name__ == '__main__':
    main()
