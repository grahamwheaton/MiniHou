import numpy as np

from ..geometry import Geometry
from ..sdk import Float, Int, Vec3, node


@node("grid", category="source", params=[
    Float("size_x", 10.0, min=0.0, soft_max=100.0, label="Size X"),
    Float("size_y", 10.0, min=0.0, soft_max=100.0, label="Size Y"),
    Int("columns", 10, min=2, max=20000, soft_max=1000, label="Columns (points)", proxy="res2"),
    Int("rows", 10, min=2, max=20000, soft_max=1000, label="Rows (points)", proxy="res2"),
    Vec3("center", (0.0, 0.0, 0.0)),
])
def grid(size_x, size_y, columns, rows, center):
    """Rectangular grid of quads in the XY plane. ``columns``/``rows`` count points per side.

    Writes point attributes ``P`` and ``uv`` (u along X, v along Y).
    """
    u = np.linspace(0.0, 1.0, columns, dtype=np.float32)
    v = np.linspace(0.0, 1.0, rows, dtype=np.float32)
    uu, vv = np.meshgrid(u, v)                                   # (rows, columns), row-major: j*columns + i
    P = np.empty((rows * columns, 3), np.float32)
    P[:, 0] = ((uu - 0.5) * size_x + center[0]).ravel()
    P[:, 1] = ((vv - 0.5) * size_y + center[1]).ravel()
    P[:, 2] = center[2]
    ii, jj = np.meshgrid(np.arange(columns - 1, dtype=np.int32), np.arange(rows - 1, dtype=np.int32))
    base = (jj * columns + ii).ravel()
    vertex_point = np.stack([base, base + 1, base + columns + 1, base + columns], axis=1).ravel()
    prim_offsets = np.arange(base.shape[0] + 1, dtype=np.int32) * 4
    g = Geometry.from_arrays(P, vertex_point, prim_offsets)
    g.point["uv"] = np.stack([uu.ravel(), vv.ravel()], axis=1)
    g.detail["grid_columns"] = int(columns)          # lets heightfield nodes see the lattice (points row by row)
    g.detail["grid_rows"] = int(rows)
    return g
