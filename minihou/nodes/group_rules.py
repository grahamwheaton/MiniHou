"""Common operations on persistent point, primitive, edge and vertex groups.

Edges use derived edge indices and ``edge_group_*`` on outgoing vertices;
vertex groups use ``group_*`` on vertex rows. All operations copy geometry.
"""
from __future__ import annotations

import fnmatch
import re

import numpy as np

from .. import edges, groups, topo


_DOMAINS = {"point": "point", "points": "point", "prim": "prim", "primitive": "prim",
            "primitives": "prim", "edge": "edge", "edges": "edge", "vertex": "vertex",
            "vertices": "vertex"}


def domain_name(value):
    try:
        return _DOMAINS[value]
    except (KeyError, TypeError):
        raise groups.GroupError(f"unsupported group domain {value!r}") from None


def count(geo, dom):
    dom = domain_name(dom)
    if dom == "point": return geo.npoints
    if dom == "prim": return geo.nprims
    if dom == "vertex": return geo.nvertices
    return len(topo.edges(geo).points)


def key(dom, name):
    groups.attr_name(name)
    return (edges.PREFIX if domain_name(dom) == "edge" else groups.PREFIX) + name


def names(geo, dom):
    dom = domain_name(dom)
    prefix = edges.PREFIX if dom == "edge" else groups.PREFIX
    table = geo.vertex if dom == "edge" else geo.table(dom)
    return sorted(n[len(prefix):] for n in table if n.startswith(prefix))


def membership(geo, dom, name):
    dom = domain_name(dom)
    k = key(dom, name)
    if dom == "edge":
        return edges.values(geo, k) != 0 if k in geo.vertex else None
    a = geo.table(dom).get(k)
    return None if a is None else (a if a.ndim == 1 else a[:, 0]) != 0


def select(geo, dom, pattern):
    """Group field on one domain. Empty means all; missing wildcard matches none."""
    dom = domain_name(dom)
    if dom == "edge":
        return edges.mask(geo, pattern)
    if dom in ("point", "prim"):
        m = groups.mask(geo, dom, pattern)
        return np.ones(count(geo, dom), bool) if m is None else m
    if dom == "vertex" and (re.match(r"^(?:\(\s*)*@", (pattern or "").strip()) or
                            any(op in (pattern or "") for op in ("<", ">", "==", "!="))):
        return groups.expression_mask(geo, "vertex", pattern)
    tokens = (pattern or "").replace(",", " ").split()
    if not tokens:
        return np.ones(count(geo, dom), bool)
    out = np.full(count(geo, dom), tokens[0].startswith("^"), bool)
    for token in tokens:
        sub = token.startswith("^")
        token = token.lstrip("^")
        m = np.zeros(len(out), bool)
        match = groups._RANGE.fullmatch(token)
        if match:
            lo = int(match[1]); hi = int(match[2] or match[1])
            stride = match[3]
            if stride is not None and (not stride.isdigit() or int(stride) <= 0):
                raise groups.GroupError(f"invalid vertex range stride {token!r}")
            m[lo:min(hi + 1, len(out)):int(stride or 1)] = True
        elif token == "*":
            m[:] = True
        else:
            hits = fnmatch.filter(sorted(set().union(*(names(geo,d) for d in ("point","prim","edge","vertex")))), token)
            if not hits and not any(c in token for c in "*?["):
                raise groups.GroupError(f"no vertex group {token!r}")
            for name in hits:
                m |= groups.member(geo, dom, name)
        out = out & ~m if sub else out | m
    return out


def mask(geo, dom, pattern):
    """Alias used by the editor selection endpoint."""
    return select(geo, dom, pattern)


def write(geo, dom, name, mask, combine="replace"):
    dom = domain_name(dom)
    m = np.asarray(mask, bool)
    if m.shape != (count(geo, dom),):
        raise groups.GroupError(f"{dom} group {name!r} needs {count(geo, dom)} members, got {m.shape}")
    if combine not in ("replace", "union", "intersect", "subtract"):
        raise groups.GroupError(f"unsupported group merge {combine!r}")
    old = membership(geo, dom, name)
    if old is not None:
        if combine == "union": m = old | m
        elif combine == "intersect": m = old & m
        elif combine == "subtract": m = old & ~m
    elif combine in ("intersect", "subtract"):
        m = np.zeros_like(m)
    if dom == "edge":
        return edges.write(geo, key(dom, name), m.astype(np.int32))
    out = geo.copy()
    out.table(dom)[key(dom, name)] = m.astype(np.int32)
    return out


def delete(geo, dom, name):
    dom = domain_name(dom)
    k = key(dom, name)
    table = geo.vertex if dom == "edge" else geo.table(dom)
    if k not in table:
        return geo
    out = geo.copy()
    del (out.vertex if dom == "edge" else out.table(dom))[k]
    return out


def representative_positions(geo, dom):
    dom = domain_name(dom)
    if dom == "point": return geo.P
    if dom == "vertex": return geo.P[geo.topology.vertex_point]
    if dom == "edge": return geo.P[topo.edges(geo).points].mean(axis=1)
    return topo.prim_centroids(geo)


def _incidence(geo):
    """Element incidence on point/primitive/edge/vertex through polygon corners."""
    vp = geo.topology.vertex_point.astype(np.int64)
    pv = topo.prim_of_vertex(geo)
    ed = topo.edges(geo)
    all_inc = {d: [set() for _ in range(count(geo, d))] for d in ("point", "prim", "edge", "vertex")}
    for v in range(geo.nvertices):
        ids = {"point": int(vp[v]), "prim": int(pv[v]), "vertex": v,
               "edge": int(ed.of_vertex[v])}
        for src, a in ids.items():
            if a < 0: continue
            for dst, b in ids.items():
                if b >= 0: all_inc[src][a].add((dst, b))
    # A vertex is a polygon corner. Both the incoming and outgoing edge
    # touch it; a promoted edge group must include both endpoint corners.
    offsets = geo.topology.prim_offsets
    closed_mask = geo.topology.closed_mask()
    for prim in range(geo.nprims):
        lo, hi = int(offsets[prim]), int(offsets[prim + 1])
        if hi <= lo: continue
        closed = bool(closed_mask[prim])
        for v in range(lo, hi if closed else hi - 1):
            nxt = lo if v == hi - 1 else v + 1
            edge = int(ed.of_vertex[v])
            if edge < 0: continue
            all_inc["edge"][edge].add(("vertex", nxt))
            all_inc["vertex"][nxt].add(("edge", edge))
    # Edge endpoints can exist without outgoing vertex ownership in degenerate geometry.
    for e, pair in enumerate(ed.points):
        for p in pair:
            all_inc["edge"][e].add(("point", int(p)))
            all_inc["point"][int(p)].add(("edge", e))
    return all_inc


def convert(geo, mask, src, dst, contained=True):
    src, dst = domain_name(src), domain_name(dst)
    m = np.asarray(mask, bool)
    if m.shape != (count(geo, src),):
        raise groups.GroupError(f"{src} mask has wrong length")
    if src == dst: return m.copy()
    if src == 'edge' and dst == 'vertex' and contained:
        # Houdini associates each corner only with its outgoing half-edge in
        # fully-contained mode; incoming edges participate in the any mode.
        outgoing=topo.edges(geo).of_vertex
        result=np.zeros(geo.nvertices,bool);valid=outgoing>=0
        result[valid]=m[outgoing[valid]]
        return result
    if src == "prim" and dst == "point":
        return groups.convert(geo, m, src, dst)
    if src == "point" and dst == "prim":
        if contained:
            return groups.convert(geo, m, src, dst)
        pv = topo.prim_of_vertex(geo)
        return np.bincount(pv, weights=m[geo.topology.vertex_point], minlength=geo.nprims) > 0
    if src == "point" and dst == "vertex":
        return m[geo.topology.vertex_point].copy()
    if src == "prim" and dst == "vertex":
        return m[topo.prim_of_vertex(geo)].copy()
    inc = _incidence(geo)
    out = np.zeros(count(geo, dst), bool)
    for i, links in enumerate(inc[dst]):
        members = [j for d, j in links if d == src]
        if members:
            out[i] = all(m[j] for j in members) if contained else any(m[j] for j in members)
    return out


def matching_names(geo, dom, pattern):
    """Resolve space/comma separated name patterns, without treating bare '*' as all elements."""
    tokens = (pattern or "").replace(",", " ").split()
    return [n for n in names(geo, dom) if any(fnmatch.fnmatchcase(n, token) for token in tokens)]


def substituted_name(source, pattern, replacement):
    """Map wildcard captures to '*' in the replacement, or keep the source name."""
    if not replacement: return source
    for token in (pattern or '').replace(',',' ').split():
        if fnmatch.fnmatchcase(source,token):
            pattern=token;break
    if not any(ch in pattern for ch in "*?"):
        return replacement
    regex = "^" + "".join("(.*?)" if ch == "*" else "(.)" if ch == "?" else re.escape(ch) for ch in pattern) + "$"
    hit = re.match(regex, source)
    if hit is None: return replacement
    captures = iter(hit.groups())
    return "".join(next(captures, "") if c == "*" else c for c in replacement)
