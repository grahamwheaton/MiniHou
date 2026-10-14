"""Quad Remesh: rebuild a surface as a mostly-quad mesh whose edges follow the shape's own flow.

Inspired by QRemeshify (which wraps QuadWild Bi-MDF): build a smooth 4-symmetry cross field over the surface, spread
points evenly along it (Poisson-disk), connect them into triangles, then merge triangle pairs into quads across the
field. The result is quad-dominant (not strictly all-quad as the strip-based solvers are), follows curvature, and runs
on numpy alone. See :mod:`minihou.remesh`.
"""
import fnmatch

import numpy as np

from .. import remesh as _remesh
from ..sdk import Float, Int, Str, node
from . import _build as B
from ._util import fan_triangulate


@node("quad_remesh", inputs=["geo"], category="volume", version=1, params=[
    Float("edge_length", 0.0, min=0.0, soft_max=1.0, doc="Target edge length; 0 = auto (~5% of the bounding box)"),
    Float("adaptivity", 0.0, min=0.0, soft_max=2.0, doc="Let point density follow curvature (0 = even)"),
    Int("smoothing", 3, min=0, soft_max=20, doc="Relaxation passes on the output quads"),
    Int("seed", 0, min=0, doc="Random seed for surface sampling"),
    Str("transfer", "", doc="Point attributes to carry over from the nearest original point, e.g. 'Cd uv'"),
])
def quad_remesh(geo, edge_length, adaptivity, smoothing, seed, transfer):
    """Retopologise a mesh into an even, flow-following quad mesh (QRemeshify-style field-aligned remesh). The edges
    line up with the surface's natural directions (curvature, features), giving animation-friendly topology."""
    B.require_polygons(geo, "quad_remesh")
    if geo.npoints == 0 or geo.nprims == 0:
        return geo.copy()
    tris, _tri_prim = fan_triangulate(geo.topology)
    if len(tris) == 0:
        return geo.copy()
    vp = geo.topology.vertex_point.astype(np.int64)
    T = vp[tris]
    P = geo.P.astype(np.float64)
    S, polys, sidx = _remesh.remesh(P, T, float(edge_length), adaptivity=float(adaptivity),
                                    seed=int(seed), smooth=int(smoothing))
    if not polys:
        return B.make(S, np.zeros(0, np.int64), [0], detail=geo.detail)
    sizes = [len(p) for p in polys]
    vps = np.concatenate([np.asarray(p, np.int64) for p in polys])
    out = B.make(S, vps, B.offsets_from_sizes(sizes), detail=geo.detail)
    if transfer and geo.npoints and len(S):
        pats = transfer.replace(",", " ").split()
        names = [n for n in geo.point.names() if n != "P" and any(fnmatch.fnmatchcase(n, p) for p in pats)]
        for n in names:
            out.point[n] = geo.point[n][sidx]
    return out
