import numpy as np

from ..geometry import Geometry, Topology
from ..materials import merge_sources, unify as unify_materials
from ..sdk import Input, Int, node


def _concat_table(parts, sizes, name_source):
    """Concatenate one attribute across inputs, zero-filling inputs that lack it."""
    ref = next(p[name_source] for p in parts if name_source in p)
    shapes = {p[name_source].shape[1:] for p in parts if name_source in p}
    if len(shapes) != 1:
        raise ValueError(f"attribute {name_source!r} has mismatched shapes across inputs: {sorted(shapes)}")
    dtype = np.result_type(*[p[name_source].dtype for p in parts if name_source in p])
    chunks = [p[name_source] if name_source in p else np.zeros((n,) + ref.shape[1:], dtype)
              for p, n in zip(parts, sizes)]
    return np.concatenate(chunks, axis=0).astype(dtype, copy=False)


@node("merge", inputs=[Input("geos", multi=True)], category="combine")
def merge(geos):
    """Concatenate any number of geometries. Attributes missing on an input are zero-filled."""
    if len(geos) == 1:
        return geos[0]
    geos = unify_materials(geos)                       # material indices of every input mean the same in the result
    npts = [g.npoints for g in geos]
    nvs = [g.nvertices for g in geos]
    nps = [g.nprims for g in geos]
    pt_base = np.concatenate([[0], np.cumsum(npts)[:-1]]).astype(np.int64)
    vt_base = np.concatenate([[0], np.cumsum(nvs)[:-1]]).astype(np.int64)
    vp = np.concatenate([g.topology.vertex_point.astype(np.int64) + b for g, b in zip(geos, pt_base)]).astype(np.int32)
    po = np.concatenate([g.topology.prim_offsets[:-1].astype(np.int64) + b for g, b in zip(geos, vt_base)]
                        + [np.array([sum(nvs)], np.int64)]).astype(np.int32)
    closed = None
    if any(g.topology.has_open for g in geos):
        closed = np.concatenate([g.topology.closed_mask() for g in geos])
    out = Geometry(sum(npts), Topology(vp, po, closed))
    for dom, sizes in (("point", npts), ("vertex", nvs), ("prim", nps)):
        names = []
        for g in geos:
            for n in g.table(dom):
                if n not in names:
                    names.append(n)
        tables = [g.table(dom) for g in geos]
        for n in names:
            out.table(dom)[n] = _concat_table(tables, sizes, n)
    for g in geos:
        for k, v in g.detail.items():
            out.detail.setdefault(k, v)
    merge_sources(geos, out)
    _merge_instances(geos, out)
    return out


def _merge_instances(geos, out):
    """Inputs from different instance nodes: one combined object list, each input's indices remapped into it."""
    lists = [list(g.detail.get("instance_objects") or []) for g in geos]
    if sum(1 for lst in lists if lst) < 2:
        return
    combined = []
    for lst in lists:
        for name in lst:
            if name not in combined:
                combined.append(name)
    parts = []
    for g, lst in zip(geos, lists):
        a = g.point.get("instance")
        if a is None or not lst:
            parts.append(np.zeros(g.npoints, np.int32))
            continue
        lut = np.array([0] + [combined.index(name) + 1 for name in lst], np.int32)   # 0 stays "no instance"
        parts.append(lut[np.clip(np.asarray(a).reshape(g.npoints, -1)[:, 0], 0, len(lst))])
    out.point["instance"] = np.concatenate(parts).astype(np.int32)
    out.detail["instance_objects"] = combined


@node("switch", inputs=[Input("geos", multi=True)], category="combine", params=[
    Int("index", 0, min=0, soft_max=10, doc="Which input to pass on (0 = the first; clamped to the last)"),
])
def switch(geos, index):
    """Pass on one of several inputs (Houdini's Switch): try variants, or expose the choice as a parameter."""
    if not geos:
        raise ValueError("connect at least one input")
    return geos[min(max(int(index), 0), len(geos) - 1)]
