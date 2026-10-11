"""Regenerate only the named RBD examples; does not touch existing demos."""
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def node(kind, params=None, **inputs):
    return dict(type=kind, params=params or {}, inputs=inputs)


def wall(columns=8, rows=6):
    nodes = {
        'brick': node('box', dict(size=[.9, .4, .45], center=[-(columns-1)*.5, 0, .225])),
        'brick_surface': node('wrangle', dict(run_over='primitives', snippet='i@sharp_face = 1; i@material_index = 0'), geo='brick'),
        'row': node('copy_transform', dict(copies=columns, translate=[1, 0, 0]), geo='brick_surface'),
        'wall': node('copy_transform', dict(copies=rows, translate=[0, 0, .46]), geo='row'),
        'bricks': node('rbd_configure', dict(mass=1, friction=.65), geo='wall'),
        'ball': node('sphere', dict(radius=.45, rows=8, columns=16, center=[.2, -5, 1.4])),
        'ball_surface': node('wrangle', dict(run_over='primitives', snippet='i@material_index = 1'), geo='ball'),
        'projectile': node('rbd_configure', dict(id_offset=10000, mass=12, velocity=[0, 12, 0],
                            angular_velocity=[2, 0, 0], restitution=.15), geo='ball_surface'),
        'bodies': node('merge', geos=['bricks', 'projectile']),
        'floor': node('box', dict(size=[80, 80, .5], center=[0, 0, -.25])),
        'floor_surface': node('wrangle', dict(run_over='primitives', snippet='i@sharp_face = 1; i@material_index = 2'), geo='floor'),
        'ground': node('rbd_configure', dict(active=False, friction=.8), geo='floor_surface'),
        'sim': node('rbd_solver', dict(substeps=12), bodies='bodies', colliders='ground'),
        'out': node('output', geo='sim'),
    }
    exposed = [('Columns', 'row', 'copies'), ('Rows', 'wall', 'copies'), ('Projectile velocity', 'projectile', 'velocity'),
               ('Projectile mass', 'projectile', 'mass'), ('Brick mass', 'bricks', 'mass'),
               ('Friction', 'bricks', 'friction'), ('Bounce', 'bricks', 'restitution'),
               ('Gravity', 'sim', 'gravity'), ('Substeps', 'sim', 'substeps'), ('Reset', 'sim', 'reset')]
    return dict(version=1, nodes=nodes, display='out', exposed=[dict(name=n, node=i, param=p) for n, i, p in exposed])


def falling():
    g = wall()
    g['nodes'] = {
        'shape': node('box', dict(size=[2, 2, 2], center=[0, 0, 4])),
        'seeds': node('scatter', dict(mode='volume', count=25, seed=17), geo='shape'),
        'pieces': node('voronoi_fracture', dict(gap=.025), geo='shape', points='seeds'),
        'configured': node('rbd_configure', dict(piece_attrib='piece', angular_velocity=[.2, .4, .1]), geo='pieces'),
        'floor': node('box', dict(size=[20, 20, .5], center=[0, 0, -.25])),
        'ground': node('rbd_configure', dict(active=False), geo='floor'),
        'sim': node('rbd_solver', bodies='configured', colliders='ground'),
        'out': node('output', geo='sim'),
    }
    g['exposed'] = [dict(name=n, node=i, param=p) for n, i, p in [
        ('Pieces', 'seeds', 'count'), ('Seed', 'seeds', 'seed'), ('Gap', 'pieces', 'gap'),
        ('Mass', 'configured', 'mass'), ('Friction', 'configured', 'friction'),
        ('Velocity', 'configured', 'velocity'), ('Spin', 'configured', 'angular_velocity'),
        ('Bounce', 'configured', 'restitution'), ('Gravity', 'sim', 'gravity'),
        ('Substeps', 'sim', 'substeps'), ('Reset', 'sim', 'reset')]]
    return g


def modifier_graph(graph):
    """Empty host input keeps demos valid stack graphs without reading Blender scene objects."""
    import copy
    graph = copy.deepcopy(graph)
    graph['nodes']['host'] = node('modifier_input')
    graph['nodes']['with_host'] = node('merge', geos=['sim', 'host'])
    graph['nodes']['out']['inputs']['geo'] = 'with_host'
    return graph


if __name__ == '__main__':
    for name, graph in [('rbd_wall', wall()), ('rbd_falling', falling()),
                        ('rbd_wall_modifier', modifier_graph(wall())),
                        ('rbd_falling_modifier', modifier_graph(falling()))]:
        (ROOT / 'examples' / (name + '.json')).write_text(json.dumps(graph, indent=2) + '\n', encoding='utf8')
