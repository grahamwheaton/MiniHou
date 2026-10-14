"""Validated graph-owned measurements (editor annotations, never cooked)."""
import copy
import math
from .graph import GraphError


def clean_measurements(value):
    if not isinstance(value, list) or len(value) > 1000:
        raise GraphError('measurements must be a list of at most 1000 items')
    out = copy.deepcopy(value)
    ids = set()
    def vector(v, size):
        return isinstance(v, list) and len(v) == size and all(isinstance(x, (int, float)) and not isinstance(x, bool) and math.isfinite(x) and abs(x) < 1e12 for x in v)
    for m in out:
        if not isinstance(m, dict) or not isinstance(m.get('id'), str) or not m['id'] or len(m['id']) > 100 or m['id'] in ids:
            raise GraphError('measurement IDs must be unique strings')
        ids.add(m['id'])
        if not isinstance(m.get('points'), list) or len(m['points']) != 2 or not all(vector(p, 3) for p in m['points']):
            raise GraphError('measurement needs two finite endpoints')
        if not vector(m.get('label_offset', [8, -8]), 2):
            raise GraphError('measurement label offset needs two finite values')
        if not isinstance(m.get('label', ''), str) or len(m.get('label', '')) > 500:
            raise GraphError('measurement label must be at most 500 characters')
        refs = m.get('refs', [None, None])
        if not isinstance(refs, list) or len(refs) != 2:
            raise GraphError('measurement needs two attachment slots')
        for ref in refs:
            if ref is None:
                continue
            if not isinstance(ref, dict) or not isinstance(ref.get('node'), str) or not isinstance(ref.get('topology'), str):
                raise GraphError('invalid measurement attachment')
            indices, weights = ref.get('indices'), ref.get('weights')
            if not isinstance(indices, list) or not 1 <= len(indices) <= 3 or not all(isinstance(i, int) and not isinstance(i, bool) and i >= 0 for i in indices):
                raise GraphError('invalid attachment indices')
            if not vector(weights, len(indices)) or any(w < -1e-6 or w > 1+1e-6 for w in weights) or abs(sum(weights)-1) > 1e-6:
                raise GraphError('invalid attachment interpolation weights')
    return out

def resolve_session(session):
    """Resolve saved coordinates even when a target is not the current preview.

    Only attachment coordinates are returned; no mesh buffers are copied to the client.
    Connectivity changes never rebind an endpoint to an unrelated element.
    """
    from . import proxy, timectx
    with session.lock:
        graph = session.graph.copy()
        revision = session._rev
        frame, fps = session.frame, session.fps
        feeds = session._modifier_feeds(graph)
    resolved, cache = {}, {}
    with session._cook_lock, timectx.at(frame, fps), proxy.using(session.view_level()):
        for item in graph.measurements:
            points = []
            for endpoint, ref in zip(item['points'], item.get('refs', [None, None])):
                result = {'point': endpoint, 'status': 'Free'}
                points.append(result)
                if not ref:
                    continue
                node = graph.nodes.get(ref['node'])
                if node is None or node.meta.get('measurement_identity') != ref.get('identity'):
                    result['status'] = 'Unavailable: target deleted or replaced'
                    continue
                try:
                    if ref['node'] not in cache:
                        cache[ref['node']] = session.evaluator.cook(graph, ref['node'], feeds=feeds)
                    geo = cache[ref['node']]
                    token = f'{geo.npoints * 3}:{geo.topology.digest}'
                    if token != ref['topology'] or any(i >= geo.npoints for i in ref['indices']):
                        result['status'] = 'Detached: topology changed'
                        continue
                    result['point'] = [sum(float(geo.P[i, k]) * w for i, w in zip(ref['indices'], ref['weights'])) for k in range(3)]
                    result['status'] = 'Attached: ' + ref['node']
                except (ValueError, RuntimeError, KeyError, IndexError) as error:
                    result['status'] = 'Unavailable: ' + str(error)[:160]
            resolved[item['id']] = points
    return {'rev': revision, 'measurements': resolved}
