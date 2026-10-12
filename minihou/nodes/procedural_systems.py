"""Bounded architectural rules, geometric goals and connected-region operations.

These are deliberately small, composable modelling systems, not rigid-body simulation.
"""
import json

import numpy as np

from .. import groups, topo
from ..geometry import Geometry
from ..sdk import Enum, Float, Int, Str, Vec3, node
from ._util import euler_xyz_matrix, fan_triangulate, normalize_rows
from .attrib_tools import piece_labels
from .group import keep_prims
from .merge import merge


def _finite(a, name):
    a = np.asarray(a, np.float64)
    if not np.isfinite(a).all():
        raise ValueError(f"{name} must be finite")
    return a


def _rng(seed, key=0):
    return np.random.default_rng(np.random.SeedSequence([int(seed) & 0xffffffff, int(key) & 0xffffffff]))


# -- architecture: footprint -> storeys -> bays -> semantic components -----------------
@node("building_rules", inputs=["geo"], category="generate", params=[
    Int("storeys", 3, min=1, max=100),
    Float("storey_height", 3.0, min=0.1, max=100),
    Float("bay_width", 2.5, min=0.1, max=1000),
    Float("window_width", 0.55, min=0.0, max=0.95, doc="Fraction of each facade bay; zero omits windows"),
    Float("window_height", 0.55, min=0.0, max=0.95, doc="Fraction of storey height"),
    Float("balcony_probability", 0.25, min=0.0, max=1.0),
    Float("balcony_depth", 0.7, min=0.0, max=10),
    Enum("roof", ["flat", "pyramid", "gable"]),
    Float("roof_height", 1.5, min=0.0, max=100),
    Int("seed", 0),
    Str("rules", "[]", doc='Ordered JSON overrides, e.g. [{"floor":"ground","window_height":0.8,"balcony_probability":0}]. '
        'floor: all, ground, top, or zero-based integer. Last matching value wins.'),
])
def building_rules(geo, storeys, storey_height, bay_width, window_width, window_height,
                   balcony_probability, balcony_depth, roof, roof_height, seed, rules):
    """Generate a building per strictly convex, closed XY footprint polygon.

    Storeys are divided into evenly spaced bays. Windows are shallow facade panels;
    balconies are slabs with three parapets. They are not Boolean openings or interiors.
    Flat and pyramid roofs accept convex polygons; gable roofs require rectangles.
    Primitive component: 0 wall, 1 window, 2 balcony, 3 roof, 4 foundation.
    """
    _finite(geo.P, "footprint positions")
    if geo.topology.has_open:
        raise ValueError("building_rules needs closed footprint polygons")
    if geo.nprims > 100:
        raise ValueError("building_rules supports at most 100 footprints per cook")
    try:
        rule_list = json.loads(rules)
    except (ValueError, TypeError):
        raise ValueError("rules must be a JSON list of floor override objects") from None
    allowed = {"floor", "window_width", "window_height", "balcony_probability"}
    if not isinstance(rule_list, list) or len(rule_list) > 100:
        raise ValueError("rules must be a list with at most 100 entries")
    for r in rule_list:
        if not isinstance(r, dict) or set(r) - allowed:
            raise ValueError("each rule supports floor, window_width, window_height and balcony_probability only")
        f = r.get("floor", "all")
        if not (f in ("all", "ground", "top") or (type(f) is int and 0 <= f < storeys)):
            raise ValueError("rule floor must be all, ground, top, or a valid zero-based storey")
        for k, v in r.items():
            if k != "floor" and (type(v) not in (int, float) or not np.isfinite(v) or not 0 <= v <= (1 if k == "balcony_probability" else .95)):
                raise ValueError(f"invalid rule value for {k}")
    points, faces, tags = [], [], []
    def face(vertices, building, floor, facade, bay, component):
        if len(faces) >= 200000:
            raise ValueError("building_rules exceeds 200000 faces; reduce storeys or increase bay width")
        start = len(points)
        points.extend(vertices)
        faces.append(list(range(start, len(points))))
        tags.append((building, floor, facade, bay, component))
    for bi in range(geo.nprims):
        ids = geo.topology.vertex_point[geo.topology.prim_offsets[bi]:geo.topology.prim_offsets[bi + 1]]
        p = geo.P[ids].astype(np.float64)
        if len(p) < 3 or len(p) > 256:
            raise ValueError("footprints need 3 to 256 vertices")
        extent = max(float(np.ptp(p, axis=0).max()), 1.0)
        if np.ptp(p[:, 2]) > extent * 1e-6:
            raise ValueError("footprints must be horizontal in XY")
        edge = np.roll(p, -1, axis=0) - p
        area2 = np.sum(p[:, 0] * np.roll(p[:, 1], -1) - p[:, 1] * np.roll(p[:, 0], -1))
        if abs(area2) < extent * extent * 1e-10:
            raise ValueError("footprint has zero area")
        if area2 < 0:
            p = p[::-1]
            edge = np.roll(p, -1, axis=0) - p
        # All vertices must lie strictly inside every nonincident oriented edge.
        # This rejects concavity AND self-intersecting stars, unlike turn tests alone.
        for j, e in enumerate(edge):
            delta = p - p[j]
            signed = e[0] * delta[:, 1] - e[1] * delta[:, 0]
            check = np.ones(len(p), bool); check[[j, (j + 1) % len(p)]] = False
            if np.linalg.norm(e) < extent * 1e-8 or np.any(signed[check] <= extent * extent * 1e-10):
                raise ValueError("footprints must be strictly convex simple polygons (no holes or collinear corners)")
        if roof == "gable":
            if len(p) != 4 or any(abs(np.dot(edge[j], edge[(j + 1) % 4])) > np.linalg.norm(edge[j]) * np.linalg.norm(edge[(j + 1) % 4]) * 1e-5 for j in range(4)):
                raise ValueError("gable roof requires a rectangular footprint")
        random = _rng(seed, bi)
        face(p[::-1], bi, -1, -1, -1, 4)
        for floor in range(storeys):
            values = dict(window_width=window_width, window_height=window_height, balcony_probability=balcony_probability)
            for rule in rule_list:
                f = rule.get("floor", "all")
                if f == "all" or f == floor or f == "ground" and floor == 0 or f == "top" and floor == storeys - 1:
                    values.update({k: v for k, v in rule.items() if k != "floor"})
            z = np.array([0., 0., floor * storey_height])
            up = np.array([0., 0., storey_height])
            for side, e in enumerate(edge):
                length = np.linalg.norm(e)
                bays = max(1, int(np.floor(length / bay_width)))
                if bays > 2000:
                    raise ValueError("more than 2000 bays on one facade; increase bay_width")
                outward = np.array([e[1], -e[0], 0.]) / length
                for bay in range(bays):
                    a = p[side] + e * (bay / bays) + z
                    b = p[side] + e * ((bay + 1) / bays) + z
                    face([a, b, b + up, a + up], bi, floor, side, bay, 0)
                    w, h = values["window_width"], values["window_height"]
                    if w > 0 and h > 0:
                        mid = (a + b + up) / 2 + outward * .015
                        dx, dz = (b - a) * w / 2, up * h / 2
                        face([mid - dx - dz, mid + dx - dz, mid + dx + dz, mid - dx + dz], bi, floor, side, bay, 1)
                    if floor > 0 and balcony_depth > 0 and random.random() < values["balcony_probability"]:
                        a, b = a + (b - a) * .1, b - (b - a) * .1
                        front_a, front_b = a + outward * balcony_depth, b + outward * balcony_depth
                        face([a, front_a, front_b, b], bi, floor, side, bay, 2)
                        rail = np.array([0., 0., min(1., storey_height * .4)])
                        for u, v in ((a, front_a), (front_a, front_b), (front_b, b)):
                            face([u, v, v + rail, u + rail], bi, floor, side, bay, 2)
        top = p + [0, 0, storeys * storey_height]
        if roof == "flat" or roof_height == 0:
            face(top, bi, storeys, -1, -1, 3)
        elif roof == "pyramid":
            peak = np.mean(top, axis=0) + [0, 0, roof_height]
            for j in range(len(top)):
                face([top[j], top[(j + 1) % len(top)], peak], bi, storeys, j, -1, 3)
        else:
            a = (top[0] + top[1]) / 2 + [0, 0, roof_height]
            b = (top[2] + top[3]) / 2 + [0, 0, roof_height]
            for vertices in ([top[0], top[1], a], [top[1], top[2], b, a], [top[2], top[3], b], [top[3], top[0], a, b]):
                face(vertices, bi, storeys, -1, -1, 3)
    sizes = [len(f) for f in faces]
    out = Geometry.from_arrays(np.asarray(points, np.float32).reshape(-1, 3),
        np.arange(len(points), dtype=np.int32), np.r_[0, np.cumsum(sizes)].astype(np.int32))
    tag_array = np.asarray(tags, np.int32).reshape(-1, 5)
    for i, name in enumerate(("building_id", "floor", "facade", "bay", "component")):
        out.prim[name] = tag_array[:, i]
    palette = np.array([[.65, .62, .55], [.16, .35, .5], [.42, .43, .45], [.28, .16, .12], [.35, .35, .35]], np.float32)
    out.prim["Cd"] = palette[tag_array[:, 4]]
    for i, name in enumerate(("walls", "windows", "balconies", "roof", "foundation")):
        out = groups.write(out, "prim", name, tag_array[:, 4] == i)
    out.detail["building_components"] = ["wall", "window", "balcony", "roof", "foundation"]
    return out


# -- regions: labels are ordinary primitive attributes, reusable by for_each -----------
@node("region_identify", inputs=["geo"], category="attribute", params=[
    Enum("mode", ["connectivity", "primitive", "attribute"]),
    Str("attribute", "class", doc="Existing scalar integer primitive label when mode is attribute"),
    Int("seed", 0),
])
def region_identify(geo, mode, attribute, seed):
    """Label connected polygon regions; sharing any point joins a region.

    Primitive mode separates each face (useful for paving). Attribute mode uses existing
    integer labels. Writes region_id, region_random, region_center and region Cd.
    """
    _finite(geo.P, "region positions")
    if mode == "connectivity":
        _, labels = piece_labels(geo)
    elif mode == "primitive":
        labels = np.arange(geo.nprims, dtype=np.int32)
    else:
        labels = geo.prim.get(attribute)
        if labels is None or labels.shape != (geo.nprims,) or labels.dtype.kind not in "iu":
            raise ValueError("region attribute must be a scalar integer primitive attribute")
    keys = np.unique(labels)
    if len(keys) and (keys.min() < np.iinfo(np.int32).min or keys.max() > np.iinfo(np.int32).max):
        raise ValueError("region IDs must fit signed 32-bit integers")
    if len(keys) > 10000:
        raise ValueError("at most 10000 regions are supported")
    if len(keys) * max(geo.nvertices, 1) > 50000000:
        raise ValueError("region identification work budget exceeded; process fewer faces or regions")
    out = geo.copy()
    out.prim["region_id"] = np.asarray(labels, np.int32)
    random = np.zeros(geo.nprims, np.float32)
    centers = np.zeros((geo.nprims, 3), np.float32)
    colors = np.zeros((geo.nprims, 3), np.float32)
    pv = topo.prim_of_vertex(geo)
    for key in keys:
        selected = labels == key
        used = np.unique(geo.topology.vertex_point[selected[pv]])
        centers[selected] = geo.P[used].astype(np.float64).mean(0) if len(used) else 0
        rng = _rng(seed, key)
        random[selected] = rng.random()
        colors[selected] = rng.uniform(.15, .85, 3)
    out.prim["region_random"], out.prim["region_center"], out.prim["Cd"] = random, centers, colors
    return out


@node("region_transform", inputs=["geo"], category="modify", params=[
    Vec3("translate", (0, 0, 0)), Vec3("rotate", (0, 0, 0), doc="Euler XYZ degrees about each region centroid"),
    Vec3("scale", (1, 1, 1), min=0.001),
    Vec3("random_translate", (0, 0, 0), min=0.0, doc="Per-axis uniform +/- translation"),
    Vec3("random_rotate", (0, 0, 0), min=0.0, doc="Per-axis uniform +/- degrees"),
    Float("random_scale", 0.0, min=0.0, max=0.99, doc="Uniform multiplicative scale variation, +/- fraction"),
    Float("extrude", 0.0, soft_min=-1, soft_max=1),
    Float("random_extrude", 0.0, min=0.0, soft_max=1),
    Int("seed", 0),
])
def region_transform(geo, translate, rotate, scale, random_translate, random_rotate, random_scale, extrude, random_extrude, seed):
    """Independently transform/extrude labelled regions, splitting shared boundaries.

    Reuses connected PolyExtrude, so a connected multi-face patch has walls only along
    its outer boundary. Without region_id, connected components are identified first.
    Extrusion is performed before the centroid transform, with the original backs kept.
    """
    from .model import polyextrude
    _finite(geo.P, "region positions")
    if geo.topology.has_open or np.any(geo.topology.prim_sizes() < 3):
        raise ValueError("region_transform requires polygon faces")
    if "region_id" not in geo.prim:
        geo = region_identify(geo, "connectivity", "class", seed)
    labels = geo.prim["region_id"]
    if labels.shape != (geo.nprims,) or labels.dtype.kind not in "iu":
        raise ValueError("region_id must be a scalar integer primitive attribute")
    keys = np.unique(labels)
    if len(keys) > 10000 or geo.nvertices > 1000000:
        raise ValueError("region processing limit: 10000 regions and 1000000 input vertices")
    if len(keys) * max(geo.nvertices, 1) > 50000000:
        raise ValueError("region processing work budget exceeded; process fewer faces or regions")
    parts = []
    for key in keys:
        part = keep_prims(geo, labels == key)
        center = part.P.astype(np.float64).mean(0)
        rng = _rng(seed, key)
        t = np.asarray(translate) + rng.uniform(-1, 1, 3) * random_translate
        r = np.asarray(rotate) + rng.uniform(-1, 1, 3) * random_rotate
        s = np.asarray(scale) * (1 + rng.uniform(-random_scale, random_scale))
        d = extrude + rng.uniform(-random_extrude, random_extrude)
        if d:
            part = polyextrude(part, "", d, 0.0, "connected", True)
            for table in (part.point, part.vertex, part.prim):
                if "N" in table:
                    del table["N"]  # Side/back face normals cannot be inherited from the original patch.
        R = euler_xyz_matrix(r)
        positions = (part.P - center) @ (R * s).T + center + t
        _finite(positions, "transformed region positions")
        if positions.size and np.max(np.abs(positions)) > np.finfo(np.float32).max:
            raise ValueError("transformed region positions exceed float32 range")
        part.point["P"] = positions.astype(np.float32)
        for domain in (part.point, part.vertex, part.prim):
            if "N" in domain:
                domain["N"] = normalize_rows(domain["N"] @ (R / s).T)
        part.prim["region_id"] = np.full(part.nprims, key, np.int32)
        part.prim["region_center"] = np.tile(center + t, (part.nprims, 1)).astype(np.float32)
        parts.append(part)
    return merge(parts) if parts else geo.copy()


# -- deterministic geometric goal relaxation -----------------------------------------
_GOALS = "geometric_goals"
_TOPOLOGY = "geometric_goal_topology"


def _append_goal(geo, goal):
    _finite(geo.P, "goal positions")
    if geo.npoints > 10000:
        raise ValueError("geometric goals support at most 10000 points")
    signature = [geo.npoints, geo.topology.digest]
    if _GOALS in geo.detail and geo.detail.get(_TOPOLOGY) != signature:
        raise ValueError("topology changed after goals were captured; rebuild the goal chain")
    previous = list(geo.detail.get(_GOALS, []))
    if len(previous) >= 32:
        raise ValueError("at most 32 geometric goal nodes per chain")
    out = geo.copy()
    out.detail[_GOALS] = previous + [goal]
    out.detail[_TOPOLOGY] = signature
    return out


def _selected(geo, group):
    mask = groups.mask(geo, "point", group)
    return np.flatnonzero(np.ones(geo.npoints, bool) if mask is None else mask)


@node("goal_length", inputs=["geo"], category="physics", params=[
    Float("scale", 1.0, min=0.001, max=100, doc="Rest lengths relative to incoming geometry"),
    Float("strength", 1.0, min=0.0, max=1.0, doc="Projection fraction per iteration; not physical stiffness"),
])
def goal_length(geo, scale, strength):
    """Maintain all topology edge lengths; captures rest lengths here, before relaxation.

    Add diagonals to cloth if shear resistance is desired. This goal has no bending term.
    """
    edges = topo.edges(geo).points
    positions = _finite(geo.P, "goal positions")
    rest = np.linalg.norm(positions[edges[:, 1]] - positions[edges[:, 0]], axis=1) * scale
    return _append_goal(geo, dict(kind="length", edges=edges.tolist(), rest=rest.tolist(), strength=strength))


@node("goal_collision", inputs=["geo"], category="physics", params=[
    Float("radius", 0.25, min=0.0001, max=1000, doc="Equal-radius spheres centred at every point"),
    Float("strength", 1.0, min=0.0, max=1.0),
])
def goal_collision(geo, radius, strength):
    """Separate point spheres; packing goal, not polygon/edge self-collision.

    Every pair participates, including topology neighbours. Limit: 1000 points.
    """
    if geo.npoints > 1000:
        raise ValueError("sphere collision goal supports at most 1000 points")
    return _append_goal(geo, dict(kind="collision", radius=radius, strength=strength))


@node("goal_surface", inputs=["geo", "target"], category="physics", params=[
    Str("group", "", doc="Point selection; empty selects all"),
    Float("strength", 1.0, min=0.0, max=1.0),
])
def goal_surface(geo, target, group, strength):
    """Pull selected points to the closest point on a fixed triangle/polygon surface.

    Target polygons use the engine's fan triangulation; use convex faces or triangulate
    first. This is a two-sided attachment, not a one-sided collision obstacle.
    """
    if target.topology.has_open:
        raise ValueError("surface goal requires polygon target faces")
    tris, _ = fan_triangulate(target.topology)
    vertices = target.P[target.topology.vertex_point[tris]].astype(np.float64)
    _finite(vertices, "target positions")
    if not len(vertices) or len(vertices) > 50000:
        raise ValueError("surface target needs 1 to 50000 triangles")
    area = np.linalg.norm(np.cross(vertices[:, 1] - vertices[:, 0], vertices[:, 2] - vertices[:, 0]), axis=1)
    vertices = vertices[area > 1e-12]
    if not len(vertices):
        raise ValueError("surface target has no nondegenerate triangles")
    return _append_goal(geo, dict(kind="surface", indices=_selected(geo, group).tolist(), triangles=vertices.tolist(), strength=strength))


@node("goal_gravity", inputs=["geo"], category="physics", params=[
    Vec3("acceleration", (0, 0, -9.81)),
])
def goal_gravity(geo, acceleration):
    """Apply a downward (or custom) acceleration at each relaxation step.

    This is damped form-finding. Results depend on the solver timestep and iteration
    count; these nodes do not replace the timeline MuJoCo RBD solver.
    """
    return _append_goal(geo, dict(kind="gravity", acceleration=list(acceleration)))


@node("goal_pin", inputs=["geo"], category="physics", params=[
    Str("group", "0", doc="Point group/pattern to fix; empty fixes all points"),
])
def goal_pin(geo, group):
    """Fix selected points at their incoming positions. Pins take precedence over other goals."""
    ids = _selected(geo, group)
    return _append_goal(geo, dict(kind="pin", indices=ids.tolist(), positions=geo.P[ids].tolist()))


@node("goal_solve", inputs=["geo"], category="physics", params=[
    Int("steps", 60, min=0, max=1000),
    Int("iterations", 12, min=1, max=100),
    Float("timestep", 0.02, min=0.0001, max=0.2),
    Float("damping", 0.9, min=0.0, max=1.0, doc="Fraction of velocity removed each step; 1 is pure relaxation"),
    Float("tolerance", 0.0001, min=0.0, max=1.0, doc="Convergence needs both displacement and constraint residual below this"),
])
def goal_solve(geo, steps, iterations, timestep, damping, tolerance):
    """Replay deterministic position-based geometric goals from the incoming geometry.

    Chain goal nodes into this input. Topology and attributes survive except normals N,
    which are removed after deformation; append a Normal node to recompute them.
    Detail goal_report records residuals, displacement, steps and convergence. Incompatible
    goals need not converge. No material stiffness, structural certification or cloth collision.
    """
    from ..raycast import TriGrid
    _finite(geo.P, "solver positions")
    if geo.detail.get(_TOPOLOGY) != [geo.npoints, geo.topology.digest]:
        raise ValueError("no compatible goals: connect a goal chain without changing topology")
    goals = geo.detail.get(_GOALS, [])
    if not goals:
        raise ValueError("connect at least one geometric goal")
    P = geo.P.astype(np.float64).copy()
    velocity = np.zeros_like(P)
    free = np.ones(len(P), np.float64)
    pins = {}
    acceleration = np.zeros(3)
    prepared = []
    work = 0
    for goal in goals:
        kind = goal["kind"]
        if kind == "pin":
            for i, position in zip(goal["indices"], goal["positions"]):
                if i in pins and not np.allclose(pins[i], position, atol=1e-8, rtol=0):
                    raise ValueError("conflicting pin positions")
                pins[i] = position
        elif kind == "gravity":
            acceleration += goal["acceleration"]
        elif kind == "length":
            e = np.asarray(goal["edges"], np.int32).reshape(-1, 2)
            prepared.append((kind, goal, e, np.asarray(goal["rest"])))
            work += len(e)
        elif kind == "collision":
            e = np.column_stack(np.triu_indices(len(P), 1))
            prepared.append((kind, goal, e, None))
            work += len(e)
        elif kind == "surface":
            prepared.append((kind, goal, np.asarray(goal["indices"], np.int32), TriGrid(goal["triangles"])))
            work += len(goal["indices"]) * max(1, int(np.sqrt(len(goal["triangles"]))))
    if max(1, steps) * iterations * work > 50000000:
        raise ValueError("geometric solver work budget exceeded; reduce points, steps or iterations")
    pin_ids = np.asarray(sorted(pins), np.int32)
    pin_positions = np.asarray([pins[i] for i in pin_ids], np.float64).reshape(-1, 3)
    free[pin_ids] = 0
    def enforce_pins():
        P[pin_ids] = pin_positions
    def residuals():
        result = {"length": 0., "collision": 0., "surface": 0., "pin": 0.}
        for kind, g, data, other in prepared:
            if not g["strength"]:
                continue
            if kind == "surface":
                if len(data):
                    cp, _, _ = other.closest(P[data])
                    value = np.max(np.linalg.norm(cp - P[data], axis=1))
                else:
                    value = 0.
            elif len(data):
                lengths = np.linalg.norm(P[data[:, 1]] - P[data[:, 0]], axis=1)
                value = np.max(np.abs(lengths - other)) if kind == "length" else np.max(np.maximum(2 * g["radius"] - lengths, 0))
            else:
                value = 0.
            result[kind] = max(result[kind], float(value))
        if len(pin_ids):
            result["pin"] = float(np.max(np.linalg.norm(P[pin_ids] - pin_positions, axis=1)))
        return result
    displacement = 0.
    converged = False
    done = 0
    enforce_pins()
    for step in range(steps):
        before = P.copy()
        velocity = velocity * (1 - damping) + acceleration * timestep
        P += velocity * (timestep * free[:, None])
        enforce_pins()
        for iteration in range(iterations):
            for kind, g, data, other in prepared:
                strength = g["strength"]
                if not strength or not len(data):
                    continue
                if kind == "surface":
                    cp, _, _ = other.closest(P[data])
                    P[data] += (cp - P[data]) * (strength * free[data, None])
                    continue
                a, b = data[:, 0], data[:, 1]
                delta = P[b] - P[a]
                length = np.linalg.norm(delta, axis=1)
                direction = delta / np.maximum(length[:, None], 1e-12)
                zero = length < 1e-12
                # Stable arbitrary directions also separate exactly coincident particles.
                if np.any(zero):
                    axis = (a[zero] * 73856093 + b[zero] * 19349663) % 3
                    direction[zero] = np.eye(3)[axis]
                error = length - (other if kind == "length" else 2 * g["radius"])
                active = np.ones(len(a), bool) if kind == "length" else error < 0
                denominator = free[a] + free[b]
                active &= denominator > 0
                if not np.any(active):
                    continue
                a, b, error, direction, denominator = a[active], b[active], error[active], direction[active], denominator[active]
                correction = direction * (error * strength / denominator)[:, None]
                movement = np.zeros_like(P)
                np.add.at(movement, a, correction * free[a, None])
                np.add.at(movement, b, -correction * free[b, None])
                degree = np.bincount(np.r_[a, b], minlength=len(P))
                P += movement / np.maximum(degree[:, None], 1)
            enforce_pins()
        if not np.isfinite(P).all() or (len(P) and np.max(np.abs(P)) > 1e12):
            raise ValueError("geometric solver diverged; reduce timestep or conflicting goals")
        velocity = (P - before) / timestep
        displacement = float(np.max(np.linalg.norm(P - before, axis=1))) if len(P) else 0.
        done = step + 1
        # Only calculate expensive residuals for a candidate settled state.
        if displacement <= tolerance and max(residuals().values()) <= tolerance:
            converged = True
            break
    report = residuals()
    out = geo.copy()
    out.point["P"] = P.astype(np.float32)
    # Recomputed normals are safer than stale input normals after geometric deformation.
    for table in (out.point, out.vertex, out.prim):
        if "N" in table:
            del table["N"]
    out.detail["goal_report"] = dict(steps=done, converged=converged, max_displacement=displacement,
        residuals=report, max_residual=max(report.values()), method="position_based_relaxation")
    # Flat values remain readable in the editor's detail inspector.
    out.detail["goal_converged"] = converged
    out.detail["goal_steps"] = done
    out.detail["goal_max_displacement"] = displacement
    out.detail["goal_max_residual"] = max(report.values())
    for name, value in report.items():
        out.detail["goal_residual_" + name] = value
    return out
