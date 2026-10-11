"""Delaunay triangulation of points (Houdini's Delaunay Triangulation SOP, 2-D)."""
import numpy as np

from .. import delaunay as dl
from ..sdk import Enum, Float, node
from . import _build as B


@node("delaunay", inputs=["geo"], category="modify", params=[
    Enum("axis", ["z", "y", "x"], doc="Triangulate the points as seen along this axis (the output faces +axis)"),
    Float("max_edge", 0.0, min=0.0, soft_max=10.0, doc="Drop triangles with an edge longer than this (0: keep all), "
                                                       "to follow a concave outline or leave holes"),
])
def delaunay(geo, axis, max_edge):
    """Joins the points of the input into triangles so that no point lies inside any triangle's circumcircle. Only
    the points are used (primitives are dropped); every point is kept with its attributes, and points that end up in
    no triangle (duplicates, collinear sets) stay as loose points. Pair with ``scatter`` on a grid, or ``max_edge``
    to cut a ragged outline."""
    a = "xyz".index(axis)
    u, v = (a + 1) % 3, (a + 2) % 3                       # (u, v, axis) is right-handed: counter-clockwise from +axis
    P = geo.P.astype(np.float64)
    tri = dl.triangulate(P[:, [u, v]])
    if max_edge > 0.0 and len(tri):
        e = np.stack([np.linalg.norm(P[tri[:, i]] - P[tri[:, (i + 1) % 3]], axis=1) for i in range(3)], 1)
        tri = tri[(e <= max_edge).all(1)]
    points = dict(geo.point.items())
    return B.make(geo.P, tri.reshape(-1).astype(np.int32), np.arange(len(tri) + 1, dtype=np.int32) * 3, points,
                  detail=geo.detail)
