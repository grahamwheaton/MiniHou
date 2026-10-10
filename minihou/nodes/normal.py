import numpy as np

from .. import accel
from ..geometry import Geometry
from ..sdk import Enum, node
from ._util import normalize_rows


def prim_normals(geo: Geometry) -> np.ndarray:
    """Newell normals per primitive, length = 2 x area (so summing them area-weights). Degenerate prims -> 0."""
    topo = geo.topology
    nv, npr = topo.nvertices, topo.nprims
    if npr == 0:
        return np.zeros((0, 3), np.float64)
    offs = topo.prim_offsets.astype(np.int64)
    sizes = np.diff(offs)
    vert_prim = np.repeat(np.arange(npr), sizes)
    nxt = np.arange(nv, dtype=np.int64) + 1
    last = offs[1:] - 1
    nonempty = sizes > 0
    nxt[last[nonempty]] = offs[:-1][nonempty]                    # wrap to the prim's first vertex
    P = geo.P.astype(np.float64)
    a, b = P[topo.vertex_point], P[topo.vertex_point[nxt]]
    contrib = np.stack([(a[:, 1] - b[:, 1]) * (a[:, 2] + b[:, 2]),
                        (a[:, 2] - b[:, 2]) * (a[:, 0] + b[:, 0]),
                        (a[:, 0] - b[:, 0]) * (a[:, 1] + b[:, 1])], axis=1)
    out = np.zeros((npr, 3), np.float64)
    for c in range(3):
        out[:, c] = np.bincount(vert_prim, weights=contrib[:, c], minlength=npr)
    return out


def _normals_numba(geo, domain):
    k = accel.kernels()
    if k is None:
        return None
    topo = geo.topology
    P = np.ascontiguousarray(geo.P)
    vp = np.ascontiguousarray(topo.vertex_point)
    offs = np.ascontiguousarray(topo.prim_offsets)
    fn = np.empty((topo.nprims, 3), np.float64)
    if accel.call(k.prim_normals, P, vp, offs, fn) is accel.FAILED:
        return None
    if domain == "prim":
        return normalize_rows(fn)
    out = np.empty((geo.npoints, 3), np.float32)
    if accel.call(k.accumulate_point_normals, vp, offs, fn, out) is accel.FAILED:
        return None
    return out


@node("normal", inputs=["geo"], category="modify", params=[
    Enum("domain", ["point", "prim"], "point", doc="point: area-weighted average of adjacent faces; prim: face normals"),
])
def normal(geo, domain):
    """Compute ``N`` (unit length). Faces are counter-clockwise when seen from the side N points to.
    Points not used by any face get N = (0, 0, 0)."""
    out = geo.copy()
    fast = _normals_numba(geo, domain)
    if fast is not None:
        if domain == "prim":
            out.prim["N"] = fast
        else:
            out.point["N"] = fast
        return out
    fn = prim_normals(geo)
    if domain == "prim":
        out.prim["N"] = normalize_rows(fn)
        return out
    topo = geo.topology
    sizes = np.diff(topo.prim_offsets.astype(np.int64))
    per_vertex = np.repeat(fn, sizes, axis=0)
    acc = np.zeros((geo.npoints, 3), np.float64)
    for c in range(3):
        acc[:, c] = np.bincount(topo.vertex_point, weights=per_vertex[:, c], minlength=geo.npoints)
    out.point["N"] = normalize_rows(acc)
    return out
