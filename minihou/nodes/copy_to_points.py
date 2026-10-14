import numpy as np

from .. import accel, user_preferences
from ..geometry import Geometry, Topology
from ..sdk import Bool, Input, node
from ._util import normalize_rows, rotation_from_z_to

_TRANSFORM_ATTRS = ("P", "N", "pscale", "orient", "up", "rot", "scale")


def quaternion_matrices(q: np.ndarray) -> np.ndarray:
    """Rotation matrices of quaternions stored (x, y, z, w) like Houdini's p@orient (normalised first)."""
    q = q.astype(np.float64)
    q = q / np.maximum(np.linalg.norm(q, axis=1, keepdims=True), 1e-30)
    x, y, z, w = q[:, 0], q[:, 1], q[:, 2], q[:, 3]
    return np.stack([np.stack([1 - 2 * (y * y + z * z), 2 * (x * y - z * w), 2 * (x * z + y * w)], 1),
                     np.stack([2 * (x * y + z * w), 1 - 2 * (x * x + z * z), 2 * (y * z - x * w)], 1),
                     np.stack([2 * (x * z - y * w), 2 * (y * z + x * w), 1 - 2 * (x * x + y * y)], 1)], 1)


def frame_from_n_up(N: np.ndarray, up: np.ndarray) -> np.ndarray:
    """+Z along N and +Y towards up (Houdini's N + up convention); falls back to the minimal rotation where up is
    parallel to N."""
    Z = normalize_rows(N.astype(np.float64)).astype(np.float64)
    X = np.cross(up.astype(np.float64), Z)
    ln = np.linalg.norm(X, axis=1, keepdims=True)
    fallback = rotation_from_z_to(Z)[:, :, 0]
    X = np.where(ln > 1e-9, X / np.maximum(ln, 1e-30), fallback)
    Y = np.cross(Z, X)
    return np.stack([X, Y, Z], 2)                           # columns: the copy's X, Y, Z axes


def _instance_matrices(points, orient, use_pscale):
    """Per-target 3x3 matrices from orient / rot / N+up / N and scale x pscale, or None for the fast path."""
    pt = points.point
    T = points.npoints
    rich = any(n in pt for n in ("orient", "rot", "up", "scale"))
    if not rich:
        return None
    R = np.tile(np.eye(3), (T, 1, 1))
    if orient:
        if "orient" in pt and pt["orient"].ndim == 2 and pt["orient"].shape[1] == 4:
            R = quaternion_matrices(pt["orient"])
        elif "rot" in pt and pt["rot"].ndim == 2 and pt["rot"].shape[1] == 3:
            from .instance import euler_xyz_to_matrix
            R = euler_xyz_to_matrix(pt["rot"].astype(np.float64))
        elif "N" in pt and "up" in pt:
            R = frame_from_n_up(pt["N"], pt["up"])
        elif "N" in pt:
            R = rotation_from_z_to(normalize_rows(pt["N"]).astype(np.float64))
    s = np.ones((T, 3))
    if "scale" in pt and pt["scale"].ndim == 2 and pt["scale"].shape[1] == 3:
        s = s * pt["scale"].astype(np.float64)
    if use_pscale and "pscale" in pt:
        s = s * pt["pscale"].astype(np.float64).reshape(T, -1)[:, :1]
    return R, s


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


@node("copy_to_points", inputs=[Input("source"), Input("points")], category="generate", version=2,
      key_extra=lambda params: str(user_preferences.copy_limit_bytes()), params=[
    Bool("orient", True, doc="Rotate each copy by the target point's orient (quaternion), rot (Euler, from the "
                             "instance node), or N (+Z) and up (+Y)"),
    Bool("use_pscale", True, doc="Scale each copy by the target point's pscale (if present); 'scale' (a vector) "
                                 "always applies"),
])
def copy_to_points(source, points, orient, use_pscale):
    """Stamp a copy of ``source`` on every point of ``points`` (Houdini's Copy to Points, same attributes).

    Orientation, first match wins: ``orient`` (quaternion x,y,z,w), ``rot`` (Euler XYZ radians), ``N`` with ``up``
    (+Z along N, +Y towards up), ``N`` alone (minimal rotation of +Z). Size: ``scale`` (per axis) times ``pscale``.
    Target point attributes other than these are inherited by the copied points (overriding source attributes of
    the same name); the source's N turns with each copy. Vertex, primitive and detail attributes come from
    ``source``."""
    T, S = points.npoints, source.npoints
    # Bound the realised output and temporary position arrays before allocating.
    source_bytes = source.P.nbytes + source.topology.vertex_point.nbytes + source.topology.prim_offsets.nbytes
    for domain in (source.point, source.vertex, source.prim):
        source_bytes += sum(array.nbytes for _, array in domain.items())
    estimate = T * source_bytes + T * S * 72
    estimate += S * sum(array.nbytes for name, array in points.point.items() if name not in _TRANSFORM_ATTRS)
    limit = user_preferences.copy_limit_bytes()
    if estimate > limit:
        raise ValueError(f"Copy to Points would require approximately {estimate / 1024**3:.2f} GiB for {T:,} copies; "
                         f"the safety limit is {limit // 1024**2} MiB. Reduce the point count, use Instance, "
                         "or adjust the user Copy memory limit in Preferences.")
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

    rich = _instance_matrices(points, orient, use_pscale)
    if rich is not None:
        R, sc = rich
        world = np.einsum("tij,tsj->tsi", R, src[None, :, :].astype(np.float64) * sc[:, None, :])
        P = (world + tP[:, None, :].astype(np.float64)).reshape(T * S, 3).astype(np.float32)
    else:
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
    if "N" in source.point:                                  # the source's normals turn with each copy
        if rich is not None:
            Rn = rich[0]
        elif have_orient:
            Rn = rotation_from_z_to(normalize_rows(N).astype(np.float64))
        else:
            Rn = None
        sN = source.point["N"].astype(np.float64)
        n = np.tile(sN, (T, 1)) if Rn is None else np.einsum("tij,sj->tsi", Rn, sN).reshape(T * S, 3)
        out.point["N"] = normalize_rows(n).astype(np.float32)
    for name, arr in source.vertex.items():
        out.vertex[name] = np.tile(arr, (T,) + (1,) * (arr.ndim - 1))
    for name, arr in source.prim.items():
        out.prim[name] = np.tile(arr, (T,) + (1,) * (arr.ndim - 1))
    out.detail = dict(source.detail)
    return out
