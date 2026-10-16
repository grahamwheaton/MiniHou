"""Python-only oracles for the native volume, Voronoi and Ray ports. Never builds Rust or writes param_meta.json."""
import json
import sys
from pathlib import Path
import numpy as np

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
import minihou  # noqa: F401
from minihou import accel, rust, topo
from minihou.geometry import Geometry
from minihou.sdk import REGISTRY
from tools.dump_rust_modeling_fixtures import encode, source


def cook(kind, *args, **params):
    spec = REGISTRY.get(kind)
    return spec.fn(*args, **spec.coerce_params(params))


def main():
    cases = []

    def add(name, kind, inputs=(), params=None, manifold=False, error=False):
        spec = REGISTRY.get(kind)
        params = params or {}
        case = dict(name=name, type=kind, inputs=[None if g is None else encode(g) for g in inputs],
                    params=params, defaults=spec.coerce_params({}), manifold=manifold)
        try:
            expected = cook(kind, *inputs, **params)
            expected.validate()
            if manifold:
                edges = topo.edges(expected)
                assert np.all(edges.count == 2), f'{name}: Python oracle is not edge manifold'
                vp = expected.topology.vertex_point
                winding = np.where(vp < vp[topo.next_vertex(expected)], 1, -1)
                assert np.all(np.bincount(edges.of_vertex, weights=winding, minlength=len(edges.points)) == 0), name
            case['expected'] = encode(expected)
            assert not error, name
        except (ValueError, RuntimeError) as exc:
            if not error:
                raise
            case['error'] = str(exc)
        cases.append(case)

    with accel.disabled(), rust.disabled():
        empty = Geometry.from_arrays(np.zeros((0, 3), np.float32))
        for shape in ('sphere', 'box', 'cylinder', 'capsule', 'ellipsoid', 'torus', 'round_cone', 'prism'):
            for smooth in (0, 1):
                add(f'shape-{shape}-{smooth}', 'sdf_shape', params=dict(shape=shape, voxel_size=.19,
                    smooth=smooth, rotate=[17, 23, -31], center=[.13, -.17, .21],
                    size=[.8, 1.1, 1.4], bevel=.07), manifold=True)
        for kind in ('box', 'sphere'):
            if kind == 'box':
                a, b = source('box'), source('box')
                b.point['P'] = b.P + np.array([.31, .23, .17], np.float32)
            else:
                a = cook('sphere', rows=6, columns=9, radius=.7)
                b = cook('sphere', rows=6, columns=9, radius=.6, center=[.41, .13, .07])
                a.point['Cd'] = np.tile([1., .2, .3], (a.npoints, 1)).astype(np.float32)
                b.point['Cd'] = np.tile([.1, .2, 1.], (b.npoints, 1)).astype(np.float32)
            for operation in ('union', 'subtract', 'intersect'):
                for smooth in (0., .23):
                    for transfer in (False, True):
                        add(f'sdf-{kind}-{operation}-{smooth}-{transfer}', 'sdf_boolean', [a, b],
                            dict(operation=operation, smooth=smooth, transfer_attribs=transfer, voxel_size=.21), True)
        add('sdf-empty', 'sdf_boolean', [empty, empty])
        add('sdf-empty-b', 'sdf_boolean', [a, empty], dict(voxel_size=.21))
        add('sdf-invalid-spacing', 'sdf_shape', params=dict(voxel_size=0), error=True)
        add('sdf-budget', 'sdf_shape', params=dict(voxel_size=.0001), error=True)

        box = source('box')
        points = Geometry.from_arrays(np.array([[-.21, 0, 0], [.29, .11, 0], [.09, -.33, .17]], np.float32))
        points.point['cluster'] = np.array([2, 2, -1], np.int32)
        groups = dict(outgrp_cutpoints=True, outgrp_cutedges=True)
        for gap in (0., .06):
            add(f'fracture-{gap}', 'voronoi_fracture', [box, points], dict(gap=gap, outgrp_outside=True, **groups), True)
        for opts in (dict(cluster_pieces=True), dict(create_inside=False), dict(inside_group='caps'),
                     dict(inside_group=''), dict(outgrp_inside=False)):
            add(f'fracture-{opts}', 'voronoi_fracture', [box, points], dict(**groups, **opts))
        add('fracture-no-points', 'voronoi_fracture', [box, empty])
        add('fracture-duplicate-seeds', 'voronoi_fracture', [box, Geometry.from_arrays(np.zeros((2, 3), np.float32))])
        for kind in ('quad', 'concave', 'box'):
            add(f'split-{kind}', 'voronoi_split', [source(kind), points], groups)
        add('split-empty', 'voronoi_split', [empty, points])
        fractured = cook('voronoi_fracture', box, points)
        for extra in (None, points):
            add(f'adjacency-extra-{extra is not None}', 'voronoi_adjacency', [fractured, extra],
                dict(outgrp_connected=True, outgrp_isolated=True, outgrp_strong=True))
        gapped = cook('voronoi_fracture', box, points, gap=.04)
        add('adjacency-gap', 'voronoi_adjacency', [gapped, None], dict(max_gap=.05, min_area=.02))
        add('adjacency-connectivity', 'voronoi_adjacency', [fractured, None], dict(piece_attrib=''))
        add('adjacency-short-points', 'voronoi_adjacency', [fractured, Geometry.from_arrays([[0., 0., 0.]])], error=True)
        add('adjacency-empty', 'voronoi_adjacency', [empty, None], dict(piece_attrib=''))

        target = source('concave')
        target.prim['label'] = np.array([17], np.int32)
        origins = Geometry.from_arrays(np.array([[.21, .34, 1.], [1.43, 1.21, .8], [1.47, .31, -2.], [.13, 1.41, 2.]], np.float32))
        origins.point['N'] = np.tile([0, 0, -1], (origins.npoints, 1)).astype(np.float32)
        origins.point['keepme'] = np.arange(origins.npoints, dtype=np.int32)
        for method in ('direction', 'normal', 'minimum_distance'):
            for both in (False, True):
                add(f'ray-{method}-{both}', 'ray', [origins, target], dict(method=method, both_ways=both,
                    import_attribs='temperature id uv_* corner_id label', scale=.7, lift=.13))
        for opts in (dict(max_distance=.7), dict(delete_misses=True), dict(group='0 2'), dict(transform_points=False),
                     dict(direction=[.17, -.09, -1.]), dict(import_attribs='[ti]*')):
            add(f'ray-{opts}', 'ray', [origins, target], opts)
        add('ray-empty', 'ray', [origins, empty], dict(import_attribs='*', delete_misses=True))
        add('ray-empty-query', 'ray', [empty, target], error=True)
        add('ray-zero-direction', 'ray', [origins, target], dict(direction=[0, 0, 0]), error=True)
        add('ray-missing-normal', 'ray', [empty, target], dict(method='normal'), error=True)
    path = ROOT / 'rust/crates/minihou-core/src/nodes/test_data/volume_fracture_parity.json'
    path.write_text(json.dumps(cases, separators=(',', ':')) + '\n', encoding='utf-8')
    print(f'Wrote {len(cases)} Python oracles to {path}')


if __name__ == '__main__':
    main()
