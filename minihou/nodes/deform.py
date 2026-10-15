"""Deformers and clean-up: bend (with twist and taper), point_jitter, point_relax, clean."""
import numpy as np

from .. import groups, spatial
from ..sdk import Bool, Float, Guide, Handle, Input, Int, Str, Vec3, node
from . import _build as B
from .model import compact_points
from ._util import normalize_rows


def _frame(direction, up):
    D = np.asarray(direction, np.float64)
    D = D / max(np.linalg.norm(D), 1e-12)
    U = np.asarray(up, np.float64)
    U = U - (U @ D) * D
    if np.linalg.norm(U) < 1e-9:                          # up parallel to the axis: any perpendicular
        U = np.cross(D, [1.0, 0, 0] if abs(D[0]) < 0.9 else [0, 1.0, 0])
    U = U / np.linalg.norm(U)
    S = np.cross(U, D)                                   # the direction the bend curls towards
    return D, U, S


@node("bend", inputs=["geo", Input("rest", optional=True, doc="Rest geometry: the bend is worked out on these positions (same point count) and applied to the first input")],
      category="deform", params=[
    Vec3("origin", (0.0, 0.0, 0.0), doc="Start of the capture region"),
    Vec3("direction", (0.0, 0.0, 1.0), doc="Axis of the capture region"),
    Float("length", 1.0, min=1e-6, soft_max=10.0, doc="Length of the capture region along the axis"),
    Vec3("up", (0.0, 1.0, 0.0), doc="The bend turns about this direction (made perpendicular to the axis)"),
    Float("bend", 90.0, soft_min=-360.0, soft_max=360.0, doc="Degrees over the capture length"),
    Float("twist", 0.0, soft_min=-720.0, soft_max=720.0, doc="Degrees of twist about the axis over the length"),
    Float("taper", 1.0, min=0.0, soft_max=3.0, doc="Scale across the axis at the end of the region"),
    Str("group", "", doc="Only these points"),
], handles=[Handle("length", axis="@direction", at="@origin", reach=True, min=1e-6),
            Handle("bend", kind="ring", axis="@up", at="@origin"),
            Handle("twist", kind="ring", axis="@direction", at="@origin", id="twist_ring")],
      guides=[Guide("line", at="@origin", axis="@direction", size="@length", id="region", label="capture region"),
              Guide("plane", at="@origin", axis="@direction", id="start", label="capture start")])
def bend(geo, rest=None, *, origin, direction, length, up, bend, twist, taper, group):
    """Bend, twist and taper geometry along an axis (Houdini's Bend SOP). Inside the capture region the shape
    curls into an arc; beyond it, it continues straight along the bent end. N follows the deformation. With rest
    geometry on the second input the capture region is measured on the rest positions, and only the displacement
    they undergo is added to the first input (so animated or already-deformed points bend as they were modelled)."""
    D, U, S = _frame(direction, up)
    o = np.asarray(origin, np.float64)
    use_rest = rest is not None and rest.npoints == geo.npoints
    if rest is not None and not use_rest:
        raise ValueError(f"bend: the rest geometry has {rest.npoints} points, the input {geo.npoints}")
    P = (rest if use_rest else geo).P.astype(np.float64) - o
    a, s, u = P @ D, P @ S, P @ U
    t = np.clip(a / length, 0.0, 1.0)
    # twist and taper in the cross-section
    tw = np.radians(twist) * t
    sc = 1.0 + (taper - 1.0) * t
    s, u = (s * np.cos(tw) - u * np.sin(tw)) * sc, (s * np.sin(tw) + u * np.cos(tw)) * sc
    th = np.radians(bend)
    phi = th * t
    if abs(th) < 1e-9:
        C = np.outer(a, D)
        T, Ns = np.tile(D, (len(a), 1)), np.tile(S, (len(a), 1))
    else:
        R = length / th
        C = (R * np.sin(phi))[:, None] * D + (R * (1 - np.cos(phi)))[:, None] * S
        T = np.cos(phi)[:, None] * D + np.sin(phi)[:, None] * S
        Ns = -np.sin(phi)[:, None] * D + np.cos(phi)[:, None] * S
        beyond = a > length
        C = np.where(beyond[:, None], C + (a - length)[:, None] * T, C)
        C = np.where((a < 0)[:, None], np.outer(a, D), C)
    newP = o + C + s[:, None] * Ns + u[:, None] * U
    out = geo.copy()
    if use_rest:
        newP = geo.P.astype(np.float64) + (newP - (P + o))
    out.point["P"] = newP.astype(np.float32)
    if "N" in geo.point and geo.point["N"].ndim == 2:
        N = geo.point["N"].astype(np.float64)
        na, ns, nu = N @ D, N @ S, N @ U
        ns, nu = ns * np.cos(tw) - nu * np.sin(tw), ns * np.sin(tw) + nu * np.cos(tw)
        out.point["N"] = normalize_rows(na[:, None] * T + ns[:, None] * Ns + nu[:, None] * U).astype(np.float32)
    return groups.restrict(geo, out, "point", group)


@node("point_jitter", inputs=["geo"], category="deform", params=[
    Vec3("scale", (0.1, 0.1, 0.1), doc="Maximum offset along each axis"),
    Int("seed", 0),
    Str("group", ""),
])
def point_jitter(geo, scale, seed, group):
    """Move points by a random offset (Houdini's Point Jitter)."""
    rng = np.random.default_rng(int(seed) & 0xFFFFFFFF)
    out = geo.copy()
    out.point["P"] = (geo.P + (rng.random((geo.npoints, 3)) * 2 - 1) * np.asarray(scale, np.float32)).astype(np.float32)
    return groups.restrict(geo, out, "point", group)


@node("point_relax", inputs=["points", "surface"], category="deform", params=[
    Int("iterations", 10, min=0, soft_max=100),
    Float("radius", 0.0, min=0.0, soft_max=2.0, doc="Desired spacing; 0 = from the surface area and point count "
                                                    "(or the points' bounding box without a surface)"),
    Bool("update_normals", True, doc="Take N from the surface where each point lands"),
])
def point_relax(points, surface, iterations, radius, update_normals):
    """Push points apart to an even spacing and keep them on the surface (Houdini's Point Relax / Scatter's relax
    iterations): turns clumpy random scatter into blue-noise-like distributions."""
    P = points.P.astype(np.float64)
    n = len(P)
    if n < 2 or iterations == 0:
        return points
    grid = None
    if surface is not None and surface.nprims:
        from .ray import target_grid
        grid, _, tri_p = target_grid(surface)
        if grid.tris.shape[0] == 0:
            return points
        area = float(np.linalg.norm(np.cross(grid.tris[:, 1] - grid.tris[:, 0], grid.tris[:, 2] - grid.tris[:, 0]),
                                    axis=1).sum() / 2)
        r = radius or np.sqrt(area / n) * 1.07
    else:
        ext = np.maximum(np.ptp(P, axis=0), 1e-9)
        r = radius or (float(np.prod(np.sort(ext)[1:])) / n) ** 0.5 * 1.07
    tri = None
    for _ in range(iterations):
        i, j, d = spatial.pairs_within(P, r)
        if len(i):
            push = ((r - d) / np.maximum(d, 1e-12) * 0.5)[:, None] * (P[i] - P[j])
            disp = np.zeros_like(P)
            np.add.at(disp, i, push)
            np.add.at(disp, j, -push)
            P = P + disp * 0.5
        if grid is not None:
            P, _, tri = grid.closest(P)
    out = points.copy()
    out.point["P"] = P.astype(np.float32)
    if grid is not None and update_normals and tri is not None:
        t = grid.tris[tri]
        out.point["N"] = normalize_rows(np.cross(t[:, 1] - t[:, 0], t[:, 2] - t[:, 0])).astype(np.float32)
    return out


@node("clean", inputs=["geo"], category="modify", params=[
    Float("fuse_distance", 0.0, min=0.0, soft_max=0.1, doc="Weld points closer than this first (0 = off)"),
    Bool("remove_degenerate", True, doc="Drop repeated corners, then faces with < 3 corners and curves with < 2"),
    Bool("remove_unused_points", True),
])
def clean(geo, fuse_distance, remove_degenerate, remove_unused_points):
    """Tidy geometry (Houdini's Clean): weld, remove degenerate primitives and unused points."""
    from .primitives import fuse_points
    out = fuse_points(geo, fuse_distance) if fuse_distance > 0 else geo
    if remove_degenerate:
        out = B.drop_degenerate(out)
    if remove_unused_points:
        out = compact_points(out)
    return out
