"""Library node: roads with intersections from a network of centre lines.

Feed it curves in the XY plane (Z up): polylines that end at, cross at or start from the same point make a junction. Each edge of
the curves becomes a strip of road; every point where roads meet (a bend, a T, a crossroads, a five-way...) gets a patch that fills
the corner, with the roads trimmed back just far enough that their edges meet cleanly. Draw the centre lines with ``curve``,
``bezier``, ``polyline`` or anything that makes curves; ``resample`` first for smooth roads.

Output: a quad/polygon surface. Prim attributes ``part`` (0 road, 1 junction) and ``road`` (the edge number, junctions: -1);
``junction`` on junction patches is the point number of the crossing in the input. Vertex ``uv``: u across the road (0..1),
v along it in road widths (junction patches: world position / width).
"""
import numpy as np

from minihou import topo
from minihou.geometry import Geometry
from minihou.sdk import Float, Input, Str, node

WELD_MAX = 1000000


@node("road_network", inputs=[Input("curves", doc="Centre lines: open or closed curves in the XY plane (Z is kept as the road's height)")],
      category="generate", params=[
    Float("width", 3.0, min=0.01, soft_max=20.0, doc="Width of every road"),
    Float("weld", 0.05, min=0.0, soft_max=1.0, doc="Curve points closer than this are one junction (curves that nearly touch join up)"),
    Float("corner_extra", 0.0, min=0.0, soft_max=10.0, doc="Trim roads back this much more at a junction (a bigger crossing, room for kerbs)"),
    Float("min_segment", 0.0, min=0.0, soft_max=5.0, doc="Edges shorter than this are ignored"),
    Str("group", "", doc="Only these curve points' edges (empty: all)"),
])
def road_network(curves, width, weld, corner_extra, min_segment, group):
    """A road surface with proper junction patches from centre-line curves."""
    if curves is None or not curves.npoints:
        return Geometry()
    hw = width / 2.0
    P = curves.P.astype(np.float64)
    if not np.isfinite(P).all():
        raise ValueError("Curve points must be finite")
    e = topo.edges(curves).points
    if len(e) == 0:
        return Geometry()
    if group:
        from minihou import groups
        keep = groups.mask(curves, "point", group)
        if keep is not None:
            e = e[keep[e[:, 0]] & keep[e[:, 1]]]
    # weld nearby points into junction nodes
    if weld > 0:
        key = np.round(P / weld).astype(np.int64)
        _, node_of = np.unique(key, axis=0, return_inverse=True)
        node_of = node_of.reshape(-1)
    else:
        node_of = np.arange(len(P))
    nn = int(node_of.max()) + 1
    cnt = np.bincount(node_of, minlength=nn).astype(np.float64)
    node_pos = np.stack([np.bincount(node_of, weights=P[:, c], minlength=nn) / np.maximum(cnt, 1) for c in range(3)], 1)
    node_src = np.zeros(nn, np.int64)
    node_src[node_of] = np.arange(len(P))                                  # an input point of each node, for the attribute
    a, b = node_of[e[:, 0]], node_of[e[:, 1]]
    ok = a != b
    seg = np.stack([a[ok], b[ok]], 1)
    seg = np.unique(np.sort(seg, axis=1), axis=0)                          # one road per pair of junctions
    pa, pb = node_pos[seg[:, 0]], node_pos[seg[:, 1]]
    length = np.hypot(*(pb - pa)[:, :2].T)
    seg, pa, pb, length = (x[length > max(min_segment, 1e-9)] for x in (seg, pa, pb, length))
    ns = len(seg)
    if not ns:
        return Geometry()
    if ns > WELD_MAX:
        raise ValueError("Too many road segments")
    d_ab = (pb - pa)[:, :2] / length[:, None]                               # unit direction a -> b

    # incident roads per node: (segment, outward direction)
    inc_node = np.concatenate([seg[:, 0], seg[:, 1]])
    inc_seg = np.concatenate([np.arange(ns), np.arange(ns)])
    inc_end = np.concatenate([np.zeros(ns, np.int64), np.ones(ns, np.int64)])   # 0: this node is the segment's a end
    inc_dir = np.concatenate([d_ab, -d_ab])
    inc_ang = np.arctan2(inc_dir[:, 1], inc_dir[:, 0])
    order = np.lexsort((inc_ang, inc_node))                                 # by node, then counter-clockwise
    inc_node, inc_seg, inc_end, inc_dir, inc_ang = (x[order] for x in (inc_node, inc_seg, inc_end, inc_dir, inc_ang))
    first = np.searchsorted(inc_node, np.arange(nn + 1))

    trim = np.zeros((ns, 2))                                                # how far each end is cut back
    for n in range(nn):
        lo, hi = first[n], first[n + 1]
        k = hi - lo
        if k < 2:
            continue
        ang = inc_ang[lo:hi]
        gap_ccw = (np.roll(ang, -1) - ang) % (2 * np.pi)                    # angle from road i to the next one counter-clockwise
        if k == 2:
            gap_ccw = np.array([(ang[1] - ang[0]) % (2 * np.pi), (ang[0] - ang[1]) % (2 * np.pi)])
        t_corner = np.where(gap_ccw < np.pi - 1e-6, hw / np.tan(np.maximum(gap_ccw, 1e-6) / 2.0), 0.0)
        t_for = np.maximum(t_corner, np.roll(t_corner, 1))                   # a road is cut by both of its corners
        t_for = np.maximum(t_for, 0.0) + corner_extra
        for i in range(k):
            s, end = inc_seg[lo + i], inc_end[lo + i]
            trim[s, end] = t_for[i]
    cap = 0.45 * length
    trim = np.minimum(trim, cap[:, None])

    perp = np.stack([-d_ab[:, 1], d_ab[:, 0]], 1)                           # left of a -> b
    start = pa + np.concatenate([d_ab * trim[:, :1], np.zeros((ns, 1))], 1)
    stop = pb - np.concatenate([d_ab * trim[:, 1:], np.zeros((ns, 1))], 1)
    off = np.concatenate([perp * hw, np.zeros((ns, 1))], 1)
    # corners of each strip: a-left, a-right, b-right, b-left (counter-clockwise seen from above)
    quads = np.stack([start + off, start - off, stop - off, stop + off], 1)          # (ns, 4, 3)
    quad_pts = quads.reshape(-1, 3)
    quad_vp = np.arange(ns * 4).reshape(ns, 4)
    run = np.hypot(*(stop - start)[:, :2].T) / width
    uv_u = np.tile([1.0, 0.0, 0.0, 1.0], ns)
    uv_v = np.stack([np.zeros(ns), np.zeros(ns), run, run], 1).reshape(-1)
    verts = [quad_vp]
    sizes = [np.full(ns, 4)]
    pts = [quad_pts]
    uvs = [np.stack([uv_u, uv_v], 1)]
    part = [np.zeros(ns, np.int32)]
    road_id = [np.arange(ns, dtype=np.int32)]
    junction = [np.full(ns, -1, np.int32)]
    base = ns * 4

    # junction patches: for each road end at the node, its right corner then its left corner, counter-clockwise
    for n in range(nn):
        lo, hi = first[n], first[n + 1]
        k = hi - lo
        if k < 2:
            continue
        ring = []
        for i in range(k):
            s, end = inc_seg[lo + i], inc_end[lo + i]
            q = quads[s]
            # this end's corners, left/right as seen looking out of the junction along the road:
            # end 0 (outward = a -> b): a-left (q[0]) is on the left; end 1 (outward = b -> a): b-right (q[2]) is on the left
            left, right = (q[0], q[1]) if end == 0 else (q[2], q[3])
            # looking outward along the road, counter-clockwise around the node passes its right corner, then its left
            ring.append(right)
            ring.append(left)
        ring = np.array(ring)
        # drop repeated corners (neighbouring roads meet at one point)
        keep = np.ones(len(ring), bool)
        for j in range(len(ring)):
            if np.linalg.norm(ring[j, :2] - ring[j - 1, :2]) < 1e-7:
                keep[j] = False
        ring = ring[keep]
        if len(ring) < 3:
            continue
        area = 0.5 * np.sum(ring[:, 0] * np.roll(ring[:, 1], -1) - np.roll(ring[:, 0], -1) * ring[:, 1])
        if abs(area) < 1e-9:
            continue
        if area < 0:
            ring = ring[::-1]
        m = len(ring)
        pts.append(ring)
        verts.append(np.arange(base, base + m)[None, :])
        sizes.append(np.array([m]))
        uvs.append(ring[:, :2] / width)
        part.append(np.array([1], np.int32))
        road_id.append(np.array([-1], np.int32))
        junction.append(np.array([node_src[n]], np.int32))
        base += m

    all_pts = np.concatenate(pts).astype(np.float32)
    vp = np.concatenate([v.reshape(-1) for v in verts]).astype(np.int32)
    offs = np.concatenate([[0], np.cumsum(np.concatenate(sizes))]).astype(np.int32)
    geo = Geometry.from_arrays(all_pts, vp, offs)
    # vertex uv follows the vertex order (strips first, then patches)
    geo.vertex["uv"] = np.concatenate([u.reshape(-1, 2) for u in uvs])[: len(vp)].astype(np.float32)
    geo.prim["part"] = np.concatenate(part)
    geo.prim["road"] = np.concatenate(road_id)
    geo.prim["junction"] = np.concatenate(junction)
    return geo
