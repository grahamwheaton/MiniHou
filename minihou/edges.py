"""Derived-edge selections stored on existing outgoing vertex/half-edge rows.

No edge topology or wire domain is introduced. Edge ids are lexicographically
sorted endpoint pairs from topo.edges; persistent selections use edge_group_*.
"""
import fnmatch
import re
import numpy as np
from . import topo, groups

PREFIX = 'edge_group_'
CREASE = 'creaseweight'


def values(geo, name):
    e = topo.edges(geo)
    a = geo.vertex.get(name)
    out = np.zeros(len(e.points), np.float32)
    if a is not None:
        if a.ndim != 1 or not np.isfinite(a).all():
            raise ValueError(f'{name} must be a finite scalar vertex attribute')
        if name == CREASE and np.any(a < 0):
            raise ValueError('creaseweight must be nonnegative')
        valid = e.of_vertex >= 0
        np.maximum.at(out, e.of_vertex[valid], a[valid])
    return out


def write(geo, name, data):
    e = topo.edges(geo)
    data = np.asarray(data)
    if data.shape != (len(e.points),):
        raise ValueError('edge attribute length does not match derived edges')
    a = np.zeros(geo.nvertices, data.dtype)
    valid = e.of_vertex >= 0
    a[valid] = data[e.of_vertex[valid]]
    out = geo.copy()
    out.vertex[name] = a
    return out


def mask(geo, pattern):
    """Edge ids/ranges, pA-B endpoint pairs, names/wildcards and ^exclusions."""
    e = topo.edges(geo)
    tokens = (pattern or '').replace(',', ' ').split()
    if not tokens:
        return np.ones(len(e.points), bool)
    out = np.full(len(e.points), tokens[0].startswith('^'), bool)
    known = [n[len(PREFIX):] for n in geo.vertex if n.startswith(PREFIX)]
    for token in tokens:
        subtract = token.startswith('^')
        token = token[1:] if subtract else token
        m = np.zeros(len(e.points), bool)
        pair = re.fullmatch(r'p(\d+)-(\d+)', token)
        r = groups._RANGE.fullmatch(token)
        if pair:
            a, b = sorted(map(int, pair.groups()))
            m = (e.points[:, 0] == a) & (e.points[:, 1] == b)
            if not m.any():
                raise ValueError(f'edge {token!r} does not exist')
        elif r:
            lo, hi = int(r[1]), int(r[2] or r[1])
            if r[3] is not None and (not r[3].isdigit() or int(r[3]) <= 0):
                raise ValueError('edge range stride must be a positive integer')
            m[lo:min(hi + 1, len(m)):int(r[3] or 1)] = True
        elif token == '*':
            m[:] = True
        else:
            hits = fnmatch.filter(known, token)
            if not hits and not any(c in token for c in '*?['):
                raise ValueError(f'no edge group {token!r}; have {sorted(known)}')
            for name in hits:
                m |= values(geo, PREFIX + name) != 0
        out = out & ~m if subtract else out | m
    return out


def remap(before, after, point_sources=None):
    """Preserve attributes only on surviving endpoint pairs; new edges start at zero.

    point_sources optionally maps new point ids to old ids (e.g. duplicated profiles).
    """
    old, new = topo.edges(before), topo.edges(after)
    pairs = new.points if point_sources is None else np.sort(np.asarray(point_sources)[new.points], axis=1)
    lookup = {tuple(p): i for i, p in enumerate(old.points)}
    ix = np.array([lookup.get(tuple(p), -1) for p in pairs], np.int64)
    out = after
    for name in before.vertex:
        if name.startswith(PREFIX) or name == CREASE:
            a = values(before, name)
            b = np.zeros(len(pairs), before.vertex[name].dtype)
            valid = ix >= 0
            b[valid] = a[ix[valid]]
            out = write(out, name, b)
    return out


def chain(geo, selected):
    """Order a single nonbranching chain/loop. Return points and closed flag."""
    pairs = topo.edges(geo).points[np.asarray(selected, bool)]
    adj = {}
    for a, b in pairs:
        if a == b:
            raise ValueError('zero-length topological edge')
        adj.setdefault(int(a), []).append(int(b))
        adj.setdefault(int(b), []).append(int(a))
    if not adj or any(len(v) > 2 for v in adj.values()):
        raise ValueError('select one nonempty, nonbranching edge chain or loop')
    ends = sorted(p for p, nb in adj.items() if len(nb) == 1)
    if len(ends) not in (0, 2):
        raise ValueError('edge chain has invalid endpoints')
    start = ends[0] if ends else min(adj)
    path, seen, prev, cur = [], set(), None, start
    while cur not in seen:
        path.append(cur)
        seen.add(cur)
        choices = sorted(p for p in adj[cur] if p != prev)
        if not choices:
            break
        prev, cur = cur, choices[0]
    if len(path) != len(adj):
        raise ValueError('selection contains disconnected edge chains')
    return path, not ends
