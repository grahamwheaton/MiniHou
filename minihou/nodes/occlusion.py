"""Occlusion: ambient occlusion and thickness per point, by casting rays against the mesh (and optional occluders)."""
import numpy as np

from .. import groups
from ..geometry import rows
from ..sdk import Bool, Enum, Float, Input, Int, Str, node
from ._util import guard_count, normalize_rows
from .attrib_tools import _edge_smoother, _laplacian
from .merge import merge
from .normal import normal
from .ray import target_grid

CHUNK_RAYS = 1_500_000


def _frame(n):
    """Two unit vectors perpendicular to each normal (rows) and to each other."""
    ref = np.where((np.abs(n[:, 0]) < 0.9)[:, None], [[1.0, 0.0, 0.0]], [[0.0, 1.0, 0.0]])
    t = normalize_rows(np.cross(n, ref)).astype(np.float64)
    return t, np.cross(n, t)


def _cone_samples(count, spread_deg):
    """``count`` directions in a cone around +Z (half-angle ``spread_deg``) on a Fibonacci spiral: (count, 3) and their cos weights."""
    zmin = np.cos(np.radians(min(max(spread_deg, 1.0), 90.0)))
    k = (np.arange(count) + 0.5) / count
    z = 1.0 - k * (1.0 - zmin)
    phi = np.arange(count) * np.pi * (3.0 - np.sqrt(5.0))
    r = np.sqrt(np.maximum(1.0 - z * z, 0.0))
    return np.stack([r * np.cos(phi), r * np.sin(phi), z], 1), z


@node("occlusion", inputs=["geo", Input("occluders", optional=True, doc="Extra geometry that blocks the rays (a ground plane, neighbours)")],
      category="attribute", params=[
    Enum("mode", ["ambient_occlusion", "thickness"], doc="ambient_occlusion: how open the surface is to the outside (1 open, 0 closed in); "
                                                          "thickness: how far the inside of the surface is to the other side"),
    Str("name", "", doc="The point attribute to write (empty: 'ao' or 'thickness')"),
    Int("rays", 32, min=1, soft_max=256, doc="Rays per point (more is smoother and slower)"),
    Float("spread", 90.0, min=1.0, max=90.0, doc="Half-angle of the cone of rays around the normal, in degrees (90: the whole hemisphere)"),
    Float("max_distance", 0.0, min=0.0, soft_max=10.0, doc="Ambient occlusion: things further than this do not block (0: any distance). Thickness: ignore walls further than this (0: any)"),
    Float("bias", 1e-4, min=0.0, soft_max=0.1, doc="Start the rays this far off the surface, as a fraction of the object's size, so a surface does not occlude itself"),
    Int("seed", 0, doc="Rotates the ray pattern differently at each point (removes banding)"),
    Bool("self_occlusion", True, doc="The geometry itself blocks the rays (off: only the occluders do)"),
    Bool("normalize", False, doc="Thickness: divide by the largest value (0..1)"),
    Bool("invert", False, doc="Ambient occlusion: 1 where it is closed in instead of open"),
    Int("blur", 0, min=0, soft_max=20, doc="Smoothing passes over the mesh edges afterwards"),
    Str("group", "", doc="Only these points (the rest get 1 for occlusion, 0 for thickness)"),
])
def occlusion(geo, occluders, mode, name, rays, spread, max_distance, bias, seed, self_occlusion, normalize, invert, blur, group):
    """Per-point ambient occlusion or wall thickness by ray casting: a mask for dirt, edge wear, subsurface strength, cavities,
    or a 3-D-print wall check. Needs a polygon mesh; the point normals are computed (smooth) if the points have no N."""
    if not geo.nprims:
        raise ValueError("occlusion needs a polygon mesh")
    attr = name or ("ao" if mode == "ambient_occlusion" else "thickness")
    groups.attr_name(attr)
    sel = groups.mask(geo, "point", group)
    idx = np.arange(geo.npoints) if sel is None else np.flatnonzero(sel)
    guard_count(len(idx) * rays, "occlusion rays", "use fewer rays or a group")
    N = geo.point["N"] if "N" in geo.point else normal(geo, "point").point["N"]
    N = normalize_rows(N.astype(np.float64)).astype(np.float64)[idx]
    P = geo.P.astype(np.float64)[idx]
    size = float(np.linalg.norm(np.ptp(geo.P.astype(np.float64), axis=0))) or 1.0
    parts = [g for g in ((geo if self_occlusion else None), occluders) if g is not None and g.nprims]
    if not parts:
        raise ValueError("occlusion has nothing to cast against: switch self occlusion on or connect occluders")
    grid = target_grid(parts[0] if len(parts) == 1 else merge(parts))[0]
    sign = 1.0 if mode == "ambient_occlusion" else -1.0
    offs, wt = _cone_samples(rays, spread)
    t, b = _frame(N * sign)
    rot = np.random.default_rng(int(seed) & 0xFFFFFFFF).random(len(idx)) * 2.0 * np.pi
    limit = max_distance if max_distance > 0 else np.inf
    result = np.zeros(len(idx))
    step = max(CHUNK_RAYS // max(rays, 1), 1)
    for s in range(0, len(idx), step):
        sl = slice(s, s + step)
        c, sn = np.cos(rot[sl]), np.sin(rot[sl])
        x = offs[None, :, 0] * c[:, None] - offs[None, :, 1] * sn[:, None]               # (m, rays) the pattern turned about the normal
        y = offs[None, :, 0] * sn[:, None] + offs[None, :, 1] * c[:, None]
        z = offs[None, :, 2]
        d = x[..., None] * t[sl, None, :] + y[..., None] * b[sl, None, :] + z[..., None] * (N[sl] * sign)[:, None, :]
        o = np.repeat(P[sl] + N[sl] * sign * bias * size, rays, axis=0)
        dist, _, _ = grid.intersect(o, d.reshape(-1, 3), limit)
        dist = dist.reshape(-1, rays)
        hit = np.isfinite(dist)
        if mode == "ambient_occlusion":
            w = np.broadcast_to(wt, hit.shape)
            result[sl] = (w * ~hit).sum(1) / w.sum(1)                                     # cosine weighted share of rays that escape
        else:
            result[sl] = np.where(hit, dist, 0.0).sum(1) / np.maximum(hit.sum(1), 1)       # mean wall distance over the rays that hit
    if mode == "ambient_occlusion" and invert:
        result = 1.0 - result
    if mode == "thickness" and normalize and result.max() > 0:
        result = result / result.max()
    full = np.full(geo.npoints, 1.0 if mode == "ambient_occlusion" else 0.0)
    if mode == "ambient_occlusion" and invert:
        full[:] = 0.0
    full[idx] = result
    if blur:
        a, bb, deg, move = _edge_smoother(geo, False, "")
        full = _laplacian(rows(full.astype(np.float64)), a, bb, deg, move, [0.5] * blur)[:, 0]
    out = geo.copy()
    out.point[attr] = full.astype(np.float32)
    return out
