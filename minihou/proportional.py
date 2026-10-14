"""Selection-centred falloffs. Queries never allocate a full point/seed matrix."""
import heapq
import numpy as np
from . import topo

FALLOFFS = ('smooth', 'sphere', 'root', 'sharp', 'linear', 'constant', 'random')

def falloff_weights(distance, radius, falloff='smooth'):
    if not np.isfinite(radius) or radius <= 0:
        raise ValueError('proportional radius must be positive and finite')
    mode = str(falloff).lower()
    if mode not in FALLOFFS:
        raise ValueError('unknown proportional falloff: ' + str(falloff))
    x = np.clip(1.0 - np.asarray(distance) / radius, 0, 1)
    if mode == 'smooth':
        w = x*x*(3-2*x)
    elif mode == 'sphere':
        w = np.sqrt(2*x-x*x)
    elif mode == 'root':
        w = np.sqrt(x)
    elif mode == 'sharp':
        w = x*x
    elif mode == 'constant':
        w = (np.asarray(distance) < radius).astype(float)
    elif mode == 'random':
        w = x*np.random.default_rng(0).random(x.shape)
    else:
        w = x
    return np.where(np.asarray(distance) < radius, w, 0.0)

def selection_weights(geo, selected, radius, falloff='smooth', connected=False,
                      projected=False, view_direction=(0, 0, 1)):
    P = geo.P.astype(np.float64)
    if not np.isfinite(P).all():
        raise ValueError('proportional editing requires finite positions')
    selected = np.asarray(selected, bool)
    metric = P
    if projected and not connected:
        normal = np.asarray(view_direction, float)
        length = np.linalg.norm(normal)
        if not np.isfinite(length) or length < 1e-12:
            raise ValueError('projected proportional editing requires a nonzero view direction')
        normal = normal/length
        metric = P - (P @ normal)[:, None]*normal
    distance = np.full(len(P), np.inf)
    distance[selected] = 0
    if selected.any() and not selected.all():
        if connected:
            adjacency = [[] for _ in P]
            for a, b in topo.edges(geo).points:
                length = float(np.linalg.norm(P[a]-P[b]))
                adjacency[a].append((int(b), length))
                adjacency[b].append((int(a), length))
            queue = [(0.0, int(i)) for i in np.flatnonzero(selected)]
            heapq.heapify(queue)
            while queue:
                d, i = heapq.heappop(queue)
                if d != distance[i] or d >= radius:
                    continue
                for j, length in adjacency[i]:
                    candidate = d + length
                    if candidate < radius and candidate < distance[j]:
                        distance[j] = candidate
                        heapq.heappush(queue, (candidate, j))
        else:
            seeds = metric[selected]
            try:
                from scipy.spatial import cKDTree
            except ImportError:
                for start in range(0, len(P), 256):
                    query = metric[start:start+256]
                    best = np.full(len(query), np.inf)
                    for seed_start in range(0, len(seeds), 256):
                        delta = query[:, None]-seeds[None, seed_start:seed_start+256]
                        best = np.minimum(best, np.min(np.einsum('ijk,ijk->ij', delta, delta), axis=1))
                    distance[start:start+len(query)] = np.sqrt(best)
            else:
                distance = cKDTree(seeds).query(metric, distance_upper_bound=radius)[0]
    weights = falloff_weights(distance, radius, falloff)
    weights[selected] = 1.0
    return weights

def weighted_rotate(vectors, angles, weights):
    """Weighted Euler XYZ angles preserve circular arcs and full turns."""
    result = np.asarray(vectors, float).copy()
    for axis, angle in enumerate(np.deg2rad(angles)):
        a, b = (axis+1) % 3, (axis+2) % 3
        c, s = np.cos(angle*weights), np.sin(angle*weights)
        va, vb = result[:, a].copy(), result[:, b].copy()
        result[:, a], result[:, b] = c*va-s*vb, s*va+c*vb
    return result
