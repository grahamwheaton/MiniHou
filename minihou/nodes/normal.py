import numpy as np

from .. import accel
from ..geometry import Geometry
from ..sdk import Enum, Float, node
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


def vertex_normals(geo: Geometry, cusp_angle: float) -> np.ndarray:
    """Per vertex: the area-weighted sum of the normals of the faces around its point that meet its own face at no
    more than ``cusp_angle`` degrees (all pairs of corners at each point; fine for the usual valences)."""
    t = geo.topology
    fn = prim_normals(geo).astype(np.float64)
    unit = normalize_rows(fn).astype(np.float64)
    pv = np.repeat(np.arange(t.nprims), t.prim_sizes())
    vp = t.vertex_point.astype(np.int64)
    order = np.argsort(vp, kind="stable")
    counts = np.bincount(vp, minlength=geo.npoints)
    starts = np.concatenate([[0], np.cumsum(counts)])[:-1]
    c = counts[vp[order]]                                     # valence of each sorted corner's point
    a = np.repeat(order, c)                                   # every corner paired with every corner at its point
    first = np.repeat(starts[vp[order]], c)
    b = order[first + (np.arange(len(a)) - np.repeat(np.cumsum(c) - c, c))]
    keep = np.einsum("ij,ij->i", unit[pv[a]], unit[pv[b]]) >= np.cos(np.radians(cusp_angle)) - 1e-9
    a, b = a[keep], b[keep]
    acc = np.stack([np.bincount(a, weights=fn[pv[b], k], minlength=t.nvertices) for k in range(3)], 1)
    return normalize_rows(acc).astype(np.float32)


@node("normal", inputs=["geo"], category="modify", params=[
    Enum("domain", ["point", "prim", "vertex"], "point",
         doc="point: area-weighted average of adjacent faces; prim: face normals; vertex: averages that stop at "
             "edges sharper than the cusp angle (hard edges, smooth elsewhere)"),
    Float("cusp_angle", 60.0, min=0.0, max=180.0, doc="vertex: faces meeting at more than this many degrees stay "
          "sharp"),
])
def normal(geo, domain, cusp_angle=60.0):
    """Compute ``N`` (unit length). Faces are counter-clockwise when seen from the side N points to.
    Points not used by any face get N = (0, 0, 0). Vertex normals reach Blender as custom normals."""
    out = geo.copy()
    if domain == "vertex":
        out.vertex["N"] = vertex_normals(geo, cusp_angle)
        for table in (out.point, out.prim):
            if "N" in table:
                del table["N"]
        return out
    from .. import rust
    if rust.enabled() and geo.npoints:
        if domain == "prim":
            out.prim["N"] = normalize_rows(rust.prim_normals(geo))
        else:
            out.point["N"] = rust.point_normals(geo)
        return out
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
