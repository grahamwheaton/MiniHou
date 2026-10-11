"""Heightfield terrain tools on grid geometry (Houdini's HeightField Erode, simplified): thermal and hydraulic erosion.

Works on the lattice made by the ``grid`` node (points row by row; its ``grid_columns`` / ``grid_rows`` detail
attributes, which survive nodes that keep the points) with height in P.z -- displace it first with noise or a wrangle.
Besides the new heights it writes point attributes for shading in Blender (Attribute node): ``flow`` (where water
gathered: rivers, 0..1), ``erosion`` and ``deposit`` (how much material left or settled).
"""
import numpy as np

from ..sdk import Float, Int, node

_NEIGHBOURS = [(-1, -1), (-1, 0), (-1, 1), (0, -1), (0, 1), (1, -1), (1, 0), (1, 1)]


def lattice(geo):
    """(rows, columns) of a grid-ordered point set, or a clear error."""
    c, r = geo.detail.get("grid_columns"), geo.detail.get("grid_rows")
    if isinstance(c, int) and isinstance(r, int) and c * r == geo.npoints:
        return r, c
    raise ValueError("heightfield nodes need a grid from the grid node (its points in rows); this geometry has "
                     f"{geo.npoints} points and no matching grid_columns/grid_rows detail attributes")


def _shift(a, dy, dx, fill):
    """a[y + dy, x + dx] with ``fill`` outside."""
    out = np.full_like(a, fill)
    h, w = a.shape
    ys, yd = (slice(dy, h), slice(0, h - dy)) if dy >= 0 else (slice(0, h + dy), slice(-dy, h))
    xs, xd = (slice(dx, w), slice(0, w - dx)) if dx >= 0 else (slice(0, w + dx), slice(-dx, w))
    out[yd, xd] = a[ys, xs]
    return out


def thermal(H, cell, talus_deg, rate, iterations):
    """Material steeper than the talus angle slides to lower neighbours."""
    tan = np.tan(np.radians(talus_deg))
    dist = np.array([np.hypot(dy * cell[0], dx * cell[1]) for dy, dx in _NEIGHBOURS])
    for _ in range(iterations):
        nb = np.stack([_shift(H, dy, dx, np.inf) for dy, dx in _NEIGHBOURS])          # inf: no move off the map
        drop = H[None] - nb
        excess = np.maximum(drop - tan * dist[:, None, None], 0.0)
        total = excess.sum(0)
        move = rate * 0.5 * excess.max(0)
        share = np.where(total[None] > 0, excess / np.where(total > 0, total, 1.0)[None], 0.0) * move[None]
        H = H - move
        for k, (dy, dx) in enumerate(_NEIGHBOURS):
            H = H + _shift(share[k], -dy, -dx, 0.0)                                   # received from that neighbour
    return H


def _bilinear(H, x, y):
    """Height and gradient (per cell) at fractional grid positions (x along columns, y along rows)."""
    h, w = H.shape
    x0 = np.clip(np.floor(x).astype(np.int64), 0, w - 2)
    y0 = np.clip(np.floor(y).astype(np.int64), 0, h - 2)
    fx, fy = x - x0, y - y0
    a, b = H[y0, x0], H[y0, x0 + 1]
    c, d = H[y0 + 1, x0], H[y0 + 1, x0 + 1]
    height = a * (1 - fx) * (1 - fy) + b * fx * (1 - fy) + c * (1 - fx) * fy + d * fx * fy
    gx = (b - a) * (1 - fy) + (d - c) * fy
    gy = (c - a) * (1 - fx) + (d - b) * fx
    return height, gx, gy, x0, y0, fx, fy


def hydraulic(H, cell, iterations, rain, capacity, erode_rate, deposit_rate, evaporation, droplets=4096,
              lifetime=48, inertia=0.1, gravity=4.0, min_slope=0.01, seed=0):
    """Droplet erosion (the well-known particle method, run for thousands of droplets at once): each droplet rolls
    downhill with some inertia, picks up sediment while it can carry more (capacity ~ slope x speed x water) and drops
    it where it slows or climbs. Carving is spread over a 3x3 brush; deposits go to the 4 nearest points. Droplet
    visits accumulate into the flow map, which outlines the channels."""
    h, w = H.shape
    H = H.copy()
    rng = np.random.default_rng(int(seed) & 0xFFFFFFFF)
    flow = np.zeros_like(H)
    eroded = np.zeros_like(H)
    deposited = np.zeros_like(H)
    brush = [(dy, dx, wgt) for dy in (-1, 0, 1) for dx in (-1, 0, 1)
             for wgt in [max(0.0, 1.5 - np.hypot(dy, dx))]]
    bsum = sum(b[2] for b in brush)
    water0 = max(rain, 1e-6) * 100.0
    for _ in range(iterations):
        x = rng.random(droplets) * (w - 1.001)
        y = rng.random(droplets) * (h - 1.001)
        dx = np.zeros(droplets)
        dy = np.zeros(droplets)
        vel = np.ones(droplets)
        water = np.full(droplets, water0)
        sed = np.zeros(droplets)
        alive = np.ones(droplets, bool)
        for _step in range(lifetime):
            idx = np.flatnonzero(alive)
            if not len(idx):
                break
            hx, gx, gy, x0, y0, fx, fy = _bilinear(H, x[idx], y[idx])
            ndx = dx[idx] * inertia - gx * (1 - inertia)
            ndy = dy[idx] * inertia - gy * (1 - inertia)
            ln = np.hypot(ndx, ndy)
            moving = ln > 1e-12
            ndx = np.where(moving, ndx / np.where(moving, ln, 1), 0)
            ndy = np.where(moving, ndy / np.where(moving, ln, 1), 0)
            nx, ny = x[idx] + ndx, y[idx] + ndy
            inside = moving & (nx >= 0) & (nx <= w - 1.001) & (ny >= 0) & (ny <= h - 1.001)
            np.add.at(flow, (y0, x0), water[idx])
            nh = _bilinear(H, np.clip(nx, 0, w - 1.001), np.clip(ny, 0, h - 1.001))[0]
            dh = np.where(inside, nh - hx, 0.0)
            cap = np.maximum(-dh, min_slope * cell[1]) * vel[idx] * water[idx] * capacity
            depositing = (sed[idx] > cap) | (dh > 0) | ~inside
            amount_dep = np.where(dh > 0, np.minimum(dh, sed[idx]), (sed[idx] - cap) * deposit_rate)
            amount_dep = np.where(~inside, sed[idx], amount_dep)
            amount_dep = np.where(depositing, np.maximum(amount_dep, 0.0), 0.0)
            amount_ero = np.where(depositing, 0.0, np.minimum((cap - sed[idx]) * erode_rate, -dh))
            amount_ero = np.maximum(amount_ero, 0.0)
            # deposit bilinearly at the droplet's position
            for ox, oy, wgt in ((0, 0, (1 - fx) * (1 - fy)), (1, 0, fx * (1 - fy)), (0, 1, (1 - fx) * fy),
                                (1, 1, fx * fy)):
                np.add.at(H, (y0 + oy, x0 + ox), amount_dep * wgt)
                np.add.at(deposited, (y0 + oy, x0 + ox), amount_dep * wgt)
            # erode with a small brush around the nearest point
            cy = np.clip(np.round(y[idx]).astype(np.int64), 0, h - 1)
            cx = np.clip(np.round(x[idx]).astype(np.int64), 0, w - 1)
            for by, bx, wgt in brush:
                if wgt <= 0:
                    continue
                yy, xx = np.clip(cy + by, 0, h - 1), np.clip(cx + bx, 0, w - 1)
                take = amount_ero * (wgt / bsum)
                np.add.at(H, (yy, xx), -take)
                np.add.at(eroded, (yy, xx), take)
            sed[idx] += amount_ero - amount_dep
            vel[idx] = np.sqrt(np.maximum(vel[idx] ** 2 - dh * gravity, 0.0))
            water[idx] *= 1.0 - evaporation
            dx[idx], dy[idx] = ndx, ndy
            x[idx], y[idx] = nx, ny
            alive[idx] = inside & (water[idx] > water0 * 0.01)
    return H, flow, eroded, deposited


@node("heightfield_erode", inputs=["geo"], category="terrain", params=[
    Int("iterations", 30, min=0, soft_max=300, doc="Rounds of rain (each drops 'droplets' droplets)"),
    Int("droplets", 4096, min=1, soft_max=100000),
    Float("rain", 0.01, min=0.0, soft_max=0.1, doc="Water per droplet (scales how much each one can carry)"),
    Float("capacity", 4.0, min=0.0, soft_max=20.0, doc="How much sediment water can carry, per slope and speed"),
    Float("erode_rate", 0.3, min=0.0, max=1.0),
    Float("deposit_rate", 0.3, min=0.0, max=1.0),
    Float("evaporation", 0.02, min=0.0, max=1.0),
    Float("inertia", 0.1, min=0.0, max=0.95, doc="How much droplets keep their direction (smoother channels)"),
    Int("seed", 0),
    Int("thermal_iterations", 20, min=0, soft_max=200, doc="Talus slumping steps (after the water)"),
    Float("talus_angle", 35.0, min=0.0, max=89.0, doc="Slopes steeper than this slump"),
    Float("thermal_rate", 0.5, min=0.0, max=1.0),
])
def heightfield_erode(geo, iterations, droplets, rain, capacity, erode_rate, deposit_rate, evaporation, inertia, seed,
                      thermal_iterations, talus_angle, thermal_rate):
    """Erode a terrain grid: rivers cut valleys and lay down fans of sediment, then over-steep slopes slump. Writes
    flow (river mask 0..1), erosion and deposit point attributes for shading."""
    rows, cols = lattice(geo)
    P = geo.P.astype(np.float64)
    H = P[:, 2].reshape(rows, cols).copy()
    cell = (max(abs(P[cols, 1] - P[0, 1]), 1e-9) if rows > 1 else 1.0,
            max(abs(P[1, 0] - P[0, 0]), 1e-9) if cols > 1 else 1.0)
    flow = np.zeros_like(H)
    eroded = np.zeros_like(H)
    deposited = np.zeros_like(H)
    if iterations:
        H, flow, eroded, deposited = hydraulic(H, cell, iterations, rain, capacity, erode_rate, deposit_rate,
                                               evaporation, droplets=droplets, inertia=inertia, seed=seed)
    if thermal_iterations:
        H = thermal(H, cell, talus_angle, thermal_rate, thermal_iterations)
    out = geo.copy()
    newP = P.copy()
    newP[:, 2] = H.ravel()
    out.point["P"] = newP.astype(np.float32)
    f = np.log1p(flow / max(float(np.median(flow[flow > 0])) if np.any(flow > 0) else 1.0, 1e-12))   # log-scaled
    lo, hi = np.percentile(f, [50.0, 99.5]) if f.size else (0.0, 1.0)
    out.point["flow"] = np.clip((f - lo) / max(hi - lo, 1e-9), 0.0, 1.0).ravel().astype(np.float32)   # channels -> 1
    out.point["erosion"] = eroded.ravel().astype(np.float32)
    out.point["deposit"] = deposited.ravel().astype(np.float32)
    if "N" in out.point:
        del out.point["N"]                                                              # stale: add a normal node
    return out
