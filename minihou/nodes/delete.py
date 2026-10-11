import numpy as np

from ..geometry import Geometry, Topology
from ..sdk import Bool, Str, node
from ..wrangle import WrangleError

_FLAG = "__minihou_delete"


def point_mask(geo: Geometry, expr: str) -> np.ndarray:
    """Evaluate a wrangle *expression* over points (sandboxed like any snippet) -> bool (n,)."""
    if not expr.strip():
        return np.zeros(geo.npoints, bool)
    if ";" in expr or "\n" in expr:
        raise WrangleError("the delete expression must be a single expression, e.g. @P.z < 0")
    from .wrangle import run_snippet
    return run_snippet(geo, f"i@{_FLAG} = 1 if ({expr}) else 0", "point")[_FLAG].astype(bool)


def keep_points(geo: Geometry, keep: np.ndarray) -> Geometry:
    """Keep the points where ``keep`` is True and compact everything. A polygon using a removed point is removed; an
    open curve loses those points instead and splits into the runs that remain (runs of one point are dropped)."""
    topo = geo.topology
    keep = np.asarray(keep, bool)
    remap = np.cumsum(keep) - 1
    vkeep_pt = keep[topo.vertex_point]
    offs = topo.prim_offsets.astype(np.int64)
    sizes = np.diff(offs)
    vert_prim = np.repeat(np.arange(topo.nprims), sizes)
    closed = topo.closed_mask()
    # polygons survive whole or not at all
    dropped_per_prim = np.bincount(vert_prim, weights=(~vkeep_pt).astype(np.float64), minlength=topo.nprims)
    vkeep = np.where(closed[vert_prim], (dropped_per_prim == 0)[vert_prim], vkeep_pt)
    # every run of kept vertices is a primitive: a new run starts at a prim's first vertex or after a removed one
    first = np.zeros(len(vert_prim), bool)
    first[offs[:-1][sizes > 0]] = True
    starts_run = vkeep & (first | np.r_[True, ~vkeep[:-1]])
    run = np.cumsum(starts_run) - 1
    vrun = run[vkeep]
    run_size = np.bincount(vrun, minlength=int(starts_run.sum()))
    run_prim = vert_prim[starts_run]
    good = np.where(closed[run_prim], run_size > 0, run_size >= 2)
    vkeep[vkeep] = good[vrun]
    new_vp = remap[topo.vertex_point[vkeep]].astype(np.int32)
    new_offs = np.concatenate([[0], np.cumsum(run_size[good])]).astype(np.int32)
    src_prim = run_prim[good]
    out = Geometry(int(keep.sum()), Topology(new_vp, new_offs, None if topo.closed is None else closed[src_prim]))
    for name, a in geo.point.items():
        out.point[name] = a[keep]
    for name, a in geo.vertex.items():
        out.vertex[name] = a[vkeep]
    for name, a in geo.prim.items():
        out.prim[name] = a[src_prim]
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
