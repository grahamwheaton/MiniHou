"""Ray: project points onto another geometry (Houdini's Ray SOP)."""
import fnmatch
import weakref

import numpy as np

from .. import groups
from ..raycast import TriGrid
from ..sdk import Bool, Enum, Float, Str, Vec3, node
from .model import triangulate
from .normal import prim_normals
from ._util import normalize_rows

_GRIDS: "weakref.WeakKeyDictionary" = weakref.WeakKeyDictionary()     # target geometry -> (TriGrid, tri_v, tri_p)


def target_grid(target):
    """Triangle grid for a (frozen, immutable) geometry, cached while that geometry object lives."""
    hit = _GRIDS.get(target)
    if hit is None:
        tri_v, tri_p, keep = triangulate(target, np.ones(target.nprims, bool))
        # untouched (already triangular) prims come back in 'keep': add them as triangles too
        t = target.topology
        tri_sizes = t.prim_sizes()
        whole = np.flatnonzero(keep & (tri_sizes == 3))
        offs = t.prim_offsets.astype(np.int64)
        tri_v = np.concatenate([tri_v, np.stack([offs[whole], offs[whole] + 1, offs[whole] + 2], 1)])
        tri_p = np.concatenate([tri_p, whole])
        tris = target.P[t.vertex_point[tri_v]]
        hit = (TriGrid(tris), tri_v, tri_p)
        try:
            _GRIDS[target] = hit
        except TypeError:
            pass
    return hit


@node("ray", inputs=["geo", "target"], category="modify", params=[
    Enum("method", ["direction", "normal", "minimum_distance"],
         doc="direction: cast along a vector; normal: along each point's N; minimum_distance: closest point"),
    Vec3("direction", (0.0, 0.0, -1.0)),
    Bool("both_ways", False, doc="Also cast backwards and take the nearer hit"),
    Float("max_distance", 1e6, min=0.0),
    Float("scale", 1.0, min=0.0, max=1.0, doc="How far to move towards the hit (0 = not at all, 1 = onto it)"),
    Float("lift", 0.0, soft_min=-1.0, soft_max=1.0, doc="Then offset along the hit surface's normal"),
    Bool("transform_points", True, doc="Move the points (off: only write the hit attributes)"),
    Str("import_attribs", "", doc="Target attributes to copy from the hit (point attributes are interpolated); e.g. 'Cd uv'"),
    Bool("delete_misses", False, doc="Delete points whose ray hits nothing"),
    Str("group", "", doc="Only these points"),
])
def ray(geo, target, method, direction, both_ways, max_distance, scale, lift, transform_points, import_attribs,
        delete_misses, group):
    """Project points onto the target: along a direction, along their normals, or to the closest point. Writes
    hitdist (-1 on a miss), hitprim (-1), hitN (the target's face normal) and the group rayHit."""
    grid, tri_v, tri_p = target_grid(target)
    sel = groups.mask(geo, "point", group)
    idx = np.arange(geo.npoints) if sel is None else np.flatnonzero(sel)
    P = geo.P.astype(np.float64)[idx]
    if grid.tris.shape[0] == 0:
        out = geo.copy()
        out.point["hitdist"] = np.full(geo.npoints, -1.0, np.float32)
        out.point["hitprim"] = np.full(geo.npoints, -1, np.int32)
        out.point["hitN"] = np.zeros((geo.npoints, 3), np.float32)
        out = groups.write(out, "point", "rayHit", np.zeros(geo.npoints, bool))
        if import_attribs:
            pats = import_attribs.replace(",", " ").split()
            for dom, table in (("point", target.point), ("prim", target.prim), ("vertex", target.vertex)):
                for nm in table.names():
                    if nm == "P" or not any(fnmatch.fnmatchcase(nm, p) for p in pats):
                        continue
                    a = table[nm]
                    own = out.point.get(nm)
                    if own is None or own.shape[1:] != a.shape[1:]:
                        out.point[nm] = np.zeros((geo.npoints,) + a.shape[1:], a.dtype)
                    for other in ("prim", "vertex"):
                        if nm in out.table(other):
                            del out.table(other)[nm]
        if delete_misses and len(idx):
            from .delete import keep_points

            keep = np.ones(geo.npoints, bool)
            keep[idx] = False
            out = keep_points(out, keep)
        return out
    if method == "minimum_distance":
        hitP, dist, tri = grid.closest(P)
        ok = (tri >= 0) & (dist <= max_distance)
        bary = _barycentric(hitP, grid.tris[np.maximum(tri, 0)])
    else:
        if method == "normal":
            if "N" not in geo.point:
                raise ValueError("method 'normal' needs a point N (add a normal node first)")
            d = geo.point["N"].astype(np.float64)[idx]
        else:
            d = np.tile(np.asarray(direction, np.float64), (len(idx), 1))
        if not np.any(np.linalg.norm(d, axis=1) > 0):
            raise ValueError("the ray direction is zero")
        t, tri, uv = grid.intersect(P, d, max_distance)
        if both_ways:
            t2, tri2, uv2 = grid.intersect(P, -d, max_distance)
            back = t2 < t
            t, tri, uv = np.where(back, t2, t), np.where(back, tri2, tri), np.where(back[:, None], uv2, uv)
            d = np.where(back[:, None], -d, d)
        ok = np.isfinite(t)
        dn = normalize_rows(d).astype(np.float64)
        hitP = P + dn * np.where(ok, t, 0)[:, None]
        dist = np.where(ok, t, -1.0)
        bary = np.c_[1 - uv.sum(1), uv]
    prim = np.where(ok, tri_p[np.maximum(tri, 0)], -1)
    fn = normalize_rows(prim_normals(target)).astype(np.float64)
    hitN = np.where(ok[:, None], fn[np.maximum(prim, 0)] if target.nprims else 0.0, 0.0)

    out = geo.copy()
    n = geo.npoints

    def full(values, fill, dtype):
        a = np.full((n,) + values.shape[1:], fill, dtype)
        a[idx] = values
        return a

    if transform_points:
        newP = P + (hitP - P) * scale + hitN * lift
        Pall = geo.P.astype(np.float64).copy()
        Pall[idx] = np.where(ok[:, None], newP, P)
        out.point["P"] = Pall.astype(np.float32)
    out.point["hitdist"] = full(np.where(ok, dist, -1.0).astype(np.float32), -1.0, np.float32)
    out.point["hitprim"] = full(prim.astype(np.int32), -1, np.int32)
    out.point["hitN"] = full(hitN.astype(np.float32), 0.0, np.float32)
    out = groups.write(out, "point", "rayHit", full(ok, False, bool))
    if import_attribs:
        pats = import_attribs.replace(",", " ").split()
        corners = target.topology.vertex_point[tri_v[np.maximum(tri, 0)]]          # (m, 3) target points
        for dom, table in (("point", target.point), ("prim", target.prim), ("vertex", target.vertex)):
            for nm in table.names():
                if nm == "P" or not any(fnmatch.fnmatchcase(nm, p) for p in pats):
                    continue
                a = table[nm]
                if dom == "prim":
                    vals = a[np.maximum(prim, 0)]
                else:
                    src = corners if dom == "point" else tri_v[np.maximum(tri, 0)]
                    if a.dtype.kind == "f":
                        w = bary.reshape(len(bary), 3, *([1] * (a.ndim - 1)))
                        vals = (a[src] * w).sum(1).astype(np.float32)
                    else:
                        vals = a[src[:, 0]]
                own = out.point.get(nm)
                base = own if own is not None and own.shape[1:] == vals.shape[1:] else np.zeros((n,) + vals.shape[1:], vals.dtype)
                merged = base.copy()
                merged[idx] = np.where(ok.reshape((-1,) + (1,) * (vals.ndim - 1)), vals, base[idx])
                for other in ("prim", "vertex"):
                    if nm in out.table(other):
                        del out.table(other)[nm]
                out.point[nm] = merged
    if delete_misses:
        from .delete import keep_points
        miss = np.zeros(n, bool)
        miss[idx] = ~ok
        if miss.any():
            out = keep_points(out, ~miss)
    return out


def _barycentric(p, tri):
    a, b, c = tri[:, 0], tri[:, 1], tri[:, 2]
    v0, v1, v2 = b - a, c - a, p - a
    dot = lambda x, y: np.einsum("ij,ij->i", x, y)  # noqa: E731
    d00, d01, d11, d20, d21 = dot(v0, v0), dot(v0, v1), dot(v1, v1), dot(v2, v0), dot(v2, v1)
    den = d00 * d11 - d01 * d01
    den = np.where(np.abs(den) < 1e-30, 1.0, den)
    v = (d11 * d20 - d01 * d21) / den
    w = (d00 * d21 - d01 * d20) / den
    return np.c_[1 - v - w, v, w]
