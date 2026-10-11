import numpy as np

from .. import accel
from ..geometry import Geometry, Topology
from ..sdk import Bool, Input, node
from ._util import normalize_rows, rotation_from_z_to

_TRANSFORM_ATTRS = ("P", "N", "pscale")


def _copy_positions_numba(src, tP, N, orient, scale, use_scale, T, S):
    k = accel.kernels()
    if k is None:
        return None
    out = np.empty((T * S, 3), np.float32)
    r = accel.call(k.copy_points, src, np.ascontiguousarray(tP), np.ascontiguousarray(N), orient, scale,
                   use_scale, out)
    return None if r is accel.FAILED else out


def _copy_positions_numpy(src, tP, N, orient, scale, use_scale, T, S):
    world = src[None, :, :] * scale[:, None, None] if use_scale else np.broadcast_to(src[None, :, :], (T, S, 3))
    if orient:
        R = rotation_from_z_to(normalize_rows(N).astype(np.float64)).astype(np.float32)
        world = np.matmul(world, np.transpose(R, (0, 2, 1)))     # row vectors: p @ R^T
    return (world + tP[:, None, :]).reshape(T * S, 3)


@node("copy_to_points", inputs=[Input("source"), Input("points")], category="generate", params=[
    Bool("orient", True, doc="Rotate each copy so +Z follows the target point's N (if present)"),
    Bool("use_pscale", True, doc="Scale each copy by the target point's pscale (if present)"),
])
def copy_to_points(source, points, orient, use_pscale):
    """Stamp a copy of ``source`` on every point of ``points``.

    Target point attributes other than P/N/pscale are inherited by the copied points
    (overriding source attributes of the same name). Vertex, primitive and detail attributes
    come from ``source``. Orientation is the minimal rotation taking +Z to N (no roll control).
    """
    T, S = points.npoints, source.npoints
    nv, npr = source.nvertices, source.nprims
    if T == 0 or S == 0:
        out = Geometry.from_arrays(np.zeros((0, 3), np.float32))
        return out

    src = np.ascontiguousarray(source.P, dtype=np.float32)          # (S, 3)
    tP = points.P
    have_scale = use_pscale and "pscale" in points.point
    have_orient = orient and "N" in points.point
    scale = (points.point["pscale"].astype(np.float32).reshape(T, -1)[:, 0].copy() if have_scale
             else np.ones(1, np.float32))
    N = points.point["N"] if have_orient else np.zeros((1, 3), np.float32)

    P = _copy_positions_numba(src, tP, N, have_orient, scale, have_scale, T, S)
    if P is None:
        P = _copy_positions_numpy(src, tP, N, have_orient, scale, have_scale, T, S)

    vp = (np.tile(source.topology.vertex_point, T)
          + np.repeat(np.arange(T, dtype=np.int32) * np.int32(S), nv)).astype(np.int32)
    po = np.concatenate([np.tile(source.topology.prim_offsets[:-1], T)
                         + np.repeat(np.arange(T, dtype=np.int32) * np.int32(nv), npr),
                         np.array([T * nv], np.int32)]).astype(np.int32)
    out = Geometry.from_arrays(P, vp, po, np.tile(source.topology.closed, T) if source.topology.has_open else None)

    for name, arr in source.point.items():
        if name in _TRANSFORM_ATTRS:
            continue
        out.point[name] = np.tile(arr, (T,) + (1,) * (arr.ndim - 1))
    for name, arr in points.point.items():
        if name in _TRANSFORM_ATTRS:
            continue
        out.point[name] = np.repeat(arr, S, axis=0)
    if "N" in source.point and not (orient and "N" in points.point):
        out.point["N"] = np.tile(source.point["N"], (T, 1))
    for name, arr in source.vertex.items():
        out.vertex[name] = np.tile(arr, (T,) + (1,) * (arr.ndim - 1))
    for name, arr in source.prim.items():
        out.prim[name] = np.tile(arr, (T,) + (1,) * (arr.ndim - 1))
    out.detail = dict(source.detail)
    return out
