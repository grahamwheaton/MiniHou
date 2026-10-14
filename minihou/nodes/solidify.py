"""Solidify: give a surface thickness (Blender's Solidify modifier, Houdini's PolyExtrude with thickness)."""
import numpy as np

from .. import groups, topo
from ..outgroups import OutGroup, OutGroups
from ..sdk import Bool, Float, Str, node
from . import _build as B
from .normal import prim_normals

SOLID_OUT = OutGroups(
    OutGroup("top", "Top Polygons", "solidifytop", "prim", doc="The outer surface (the original faces)"),
    OutGroup("bottom", "Bottom Polygons", "solidifybottom", "prim", doc="The inner surface (the copy facing the other way)"),
    OutGroup("side", "Side Polygons", "solidifyside", "prim", doc="The rim joining the two along open edges"),
)


@node("solidify", inputs=["geo"], category="model", params=[
    Float("thickness", 0.1, soft_min=-1.0, soft_max=1.0, doc="How thick the shell is, along the normals (negative: the other way)"),
    Float("offset", -1.0, min=-1.0, max=1.0, doc="Where the original surface sits: -1: it stays the outer surface and the shell grows "
                                                  "against the normals; 0: the shell is centred on it; 1: it is the inner surface"),
    Bool("even", True, doc="Keep the thickness the same at corners and creases (move points further out where faces meet at an angle)"),
    Bool("rim", True, doc="Close the open edges with a band of faces (off: two separate sheets, e.g. for a double-sided plane)"),
    Str("group", "", doc="Only these primitives (the rest stay single sheets and follow the points they share)"),
    *SOLID_OUT.params(),
], version=1)
def solidify(geo, thickness, offset, even, rim, group, **og):
    """Make a polygon surface into a solid shell of constant thickness: the original faces, a copy offset along the normals with
    the opposite winding, and a rim of quads along every open edge. Closed meshes become a hollow shell (two nested surfaces).
    Attributes ride along (points and corners copied to the inner surface; the rim takes those of the face it closes);
    N is removed. Curves in the input pass through untouched."""
    t = geo.topology
    sizes = t.prim_sizes()
    sel = groups.mask(geo, "prim", group)
    S = (sizes >= 3) & (t.closed_mask()) & (np.ones(geo.nprims, bool) if sel is None else sel)
    if not S.any() or thickness == 0.0:
        return SOLID_OUT.write(SOLID_OUT.write(SOLID_OUT.write(geo, og, "top", S), og, "bottom", np.zeros(geo.nprims, bool)), og, "side", np.zeros(geo.nprims, bool))
    n, nv, npr = geo.npoints, t.nvertices, t.nprims
    vp = t.vertex_point.astype(np.int64)
    offs = t.prim_offsets.astype(np.int64)
    pv = topo.prim_of_vertex(geo).astype(np.int64)
    nxt = topo.next_vertex(geo).astype(np.int64)
    inS = S[pv]
    P = geo.P.astype(np.float64)

    # point normals from the solidified faces only, area weighted
    pn = prim_normals(geo)
    area2 = np.linalg.norm(pn, axis=1)
    fn = pn / np.where(area2 > 0, area2, 1.0)[:, None]
    vn = np.zeros((n, 3))
    for c in range(3):
        vn[:, c] = np.bincount(vp[inS], weights=pn[pv[inS], c], minlength=n)
    ln = np.linalg.norm(vn, axis=1)
    vn = vn / np.where(ln > 1e-30, ln, 1.0)[:, None]
    on_S = np.bincount(vp[inS], minlength=n) > 0
    scale = np.ones(n)
    if even:
        cos = np.einsum("ij,ij->i", vn[vp[inS]], fn[pv[inS]])
        mean = np.bincount(vp[inS], weights=cos, minlength=n) / np.maximum(np.bincount(vp[inS], minlength=n), 1)
        scale = 1.0 / np.clip(mean, 0.15, 1.0)
    d_out = thickness * (1.0 + offset) / 2.0
    d_in = thickness * (1.0 - offset) / 2.0
    disp = vn * scale[:, None]
    P_out = P + np.where(on_S[:, None], disp * d_out, 0.0)
    pts = np.flatnonzero(on_S)
    inner_of = np.full(n, -1, np.int64)
    inner_of[pts] = np.arange(len(pts))
    P_in = P[pts] - disp[pts] * d_in

    # inner faces: the solidified ones, corners in reverse order
    sp = np.flatnonzero(S)
    pos = np.flatnonzero(inS)                                    # corner rows of the solidified faces, in order
    start, size = offs[pv[pos]], sizes[pv[pos]]
    local = pos - start
    src = start + size - 1 - local                               # reversed corner of the same face
    in_vp = n + inner_of[vp[src]]
    in_sizes = sizes[sp]

    # rim: half-edges of solidified faces used by exactly one solidified face
    a, b = vp, vp[nxt]
    key = np.minimum(a, b) * max(n, 1) + np.maximum(a, b)
    _, inv, cnt = np.unique(key[inS], return_inverse=True, return_counts=True)
    rim_h = np.zeros(nv, bool)
    rim_h[np.flatnonzero(inS)] = cnt[inv.reshape(-1)] == 1
    rh = np.flatnonzero(rim_h) if rim else np.zeros(0, np.int64)
    rim_vp = np.stack([a[rh], n + inner_of[a[rh]], n + inner_of[b[rh]], b[rh]], 1).ravel()
    rim_rows = np.stack([rh, rh, nxt[rh], nxt[rh]], 1).ravel()

    all_vp = np.concatenate([vp, in_vp, rim_vp])
    all_sizes = np.concatenate([sizes, in_sizes, np.full(len(rh), 4)])
    point = {nm: np.concatenate([arr, arr[pts]]) for nm, arr in geo.point.items() if nm not in ("P", "N")}
    vertex = {nm: np.concatenate([arr, arr[src], arr[rim_rows]]) for nm, arr in geo.vertex.items()}
    prim_src = np.concatenate([np.arange(npr), sp, pv[rh]])
    prim = B.gather(geo.prim, prim_src)
    closed = np.concatenate([t.closed_mask(), np.ones(len(sp) + len(rh), bool)])
    out = B.make(np.concatenate([P_out, P_in]).astype(np.float32), all_vp, B.offsets_from_sizes(all_sizes), point, vertex, prim, geo.detail,
                 closed if t.has_open else None)
    ntop, nbot = npr, len(sp)
    idx = np.arange(out.nprims)
    out = SOLID_OUT.write(out, og, "top", lambda: np.r_[S, np.zeros(nbot + len(rh), bool)])
    out = SOLID_OUT.write(out, og, "bottom", lambda: (idx >= ntop) & (idx < ntop + nbot))
    return SOLID_OUT.write(out, og, "side", lambda: idx >= ntop + nbot)
