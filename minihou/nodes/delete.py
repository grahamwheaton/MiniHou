import numpy as np

from ..geometry import Geometry, Topology
from ..sdk import Bool, Str, node
from ..wrangle import WrangleError, compile_snippet

_FLAG = "__minihou_delete"


def point_mask(geo: Geometry, expr: str) -> np.ndarray:
    """Evaluate a wrangle *expression* over points (sandboxed like any snippet) -> bool (n,)."""
    if not expr.strip():
        return np.zeros(geo.npoints, bool)
    if ";" in expr or "\n" in expr:
        raise WrangleError("the delete expression must be a single expression, e.g. @P.z < 0")
    prog = compile_snippet(f"i@{_FLAG} = 1 if ({expr}) else 0")
    attrs = {name: arr for name, arr in geo.point.items()}
    return prog.run(attrs, geo.npoints)[_FLAG].astype(bool)


def keep_points(geo: Geometry, keep: np.ndarray) -> Geometry:
    """Keep the points where ``keep`` is True; primitives using any removed point are removed. Compacts everything."""
    topo = geo.topology
    keep = np.asarray(keep, bool)
    remap = np.cumsum(keep) - 1
    vkeep_pt = keep[topo.vertex_point]
    offs = topo.prim_offsets.astype(np.int64)
    sizes = np.diff(offs)
    vert_prim = np.repeat(np.arange(topo.nprims), sizes)
    # a prim survives if none of its vertices lost its point
    dropped_per_prim = np.bincount(vert_prim, weights=(~vkeep_pt).astype(np.float64), minlength=topo.nprims)
    pkeep = dropped_per_prim == 0
    vkeep = pkeep[vert_prim]
    new_vp = remap[topo.vertex_point[vkeep]].astype(np.int32)
    new_offs = np.concatenate([[0], np.cumsum(sizes[pkeep])]).astype(np.int32)
    out = Geometry(int(keep.sum()), Topology(new_vp, new_offs))
    for name, a in geo.point.items():
        out.point[name] = a[keep]
    for name, a in geo.vertex.items():
        out.vertex[name] = a[vkeep]
    for name, a in geo.prim.items():
        out.prim[name] = a[pkeep]
    out.detail = dict(geo.detail)
    return out


@node("delete", inputs=["geo"], category="modify", params=[
    Str("expr", "@P.z < 0", kind="code", doc="Wrangle expression over points; matching points are deleted"),
    Bool("invert", False, doc="Delete the points that do NOT match instead"),
])
def delete(geo, expr, invert):
    """Delete points matching an expression (Houdini's Blast/Delete). Primitives that use a deleted point are deleted
    too; point, vertex and primitive attributes are compacted to match."""
    mask = point_mask(geo, expr)
    if invert:
        mask = ~mask
    if not mask.any():
        return geo
    return keep_points(geo, ~mask)
