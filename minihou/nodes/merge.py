import numpy as np

from ..geometry import Geometry, Topology
from ..sdk import Input, node


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
    npts = [g.npoints for g in geos]
    nvs = [g.nvertices for g in geos]
    nps = [g.nprims for g in geos]
    pt_base = np.concatenate([[0], np.cumsum(npts)[:-1]]).astype(np.int64)
    vt_base = np.concatenate([[0], np.cumsum(nvs)[:-1]]).astype(np.int64)
    vp = np.concatenate([g.topology.vertex_point.astype(np.int64) + b for g, b in zip(geos, pt_base)]).astype(np.int32)
    po = np.concatenate([g.topology.prim_offsets[:-1].astype(np.int64) + b for g, b in zip(geos, vt_base)]
                        + [np.array([sum(nvs)], np.int64)]).astype(np.int32)
    out = Geometry(sum(npts), Topology(vp, po))
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
    return out
