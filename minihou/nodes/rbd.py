"""Geometry-driven rigid bodies. Physics is an optional, pinned engine dependency."""
import numpy as np

from .. import timectx
from ..sdk import Bool, Float, Input, Int, Str, Vec3, node
from .attrib_tools import piece_labels


@node("rbd_configure", inputs=["geo"], category="physics", params=[
    Str("piece_attrib", "", doc="Primitive integer piece ID; empty uses connectivity. Fracture: piece."),
    Int("id_offset", 0, doc="Offset IDs before merging separately configured bodies."),
    Bool("active", True),
    Float("mass", 1.0, min=0.000001, doc="Mass per body in kg; uniform density within its convex hull."),
    Float("friction", 0.6, min=0, max=10),
    Float("restitution", 0.0, min=0, max=1, doc="Approximate bounce, mapped to MuJoCo contact damping ratio."),
    Float("linear_damping", 0.05, min=0, max=100, doc="Exponential velocity decay rate, per second."),
    Float("angular_damping", 0.1, min=0, max=100),
    Vec3("velocity", (0, 0, 0), doc="Initial linear velocity in simulation space, units/sec."),
    Vec3("angular_velocity", (0, 0, 0), doc="Initial angular velocity in simulation space, radians/sec."),
    Bool("compound", False, doc="Several separate convex pieces with the same ID (a glue cluster) become one rigid body."),
])
def rbd_configure(geo, piece_attrib, id_offset, active, mass, friction, restitution,
                  linear_damping, angular_damping, velocity, angular_velocity, compound=False):
    """Configure closed convex polygon pieces; keep all render geometry and attributes intact.

    Primitive rbd_* attributes can subsequently be edited per piece with a wrangle.
    Geometry positions define the initial pose; use Transform before configuring.
    """
    if piece_attrib:
        ids = geo.prim.get(piece_attrib)
        if ids is None or ids.ndim != 1 or ids.dtype.kind not in "iu":
            raise ValueError("piece_attrib must name a scalar primitive integer attribute")
    else:
        _, ids = piece_labels(geo)
    ids = ids.astype(np.int64) + id_offset
    if len(ids) and (ids.min() < -(2**31) or ids.max() >= 2**31):
        raise ValueError("piece IDs exceed int32 range")
    out = geo.copy()
    out.prim["rbd_id"] = ids.astype(np.int32)
    if compound:
        out.prim["rbd_compound"] = np.ones(geo.nprims, np.int32)
    for name, value in dict(active=int(active), mass=mass, friction=friction, restitution=restitution,
                            linear_damping=linear_damping, angular_damping=angular_damping,
                            v=velocity, w=angular_velocity).items():
        out.prim["rbd_" + name] = np.tile(np.asarray(value, dtype=np.int32 if name == "active" else np.float32),
                                         (geo.nprims, 1)).reshape((geo.nprims, 3) if name in ("v", "w") else (geo.nprims,))
    return out


@node("rbd_solver", inputs=["bodies", Input("colliders", optional=True), Input("constraints", optional=True)], category="physics",
      key_extra=lambda p: "mujoco-3.15.0-rbd1|" + timectx.key(), params=[
    Vec3("gravity", (0, 0, -9.81)),
    Int("start_frame", 1),
    Int("substeps", 10, min=1, max=128),
    Int("iterations", 50, min=1, max=500),
    Float("margin", 0.001, min=0, max=1),
    Int("reset", 0, doc="Change to discard this simulation and replay its initial state."),
    Int("cache_mb", 32, min=1, max=1024, doc="Bound on cached body poses; geometry uses evaluator cache."),
    Bool("include_colliders", True),
])
def rbd_solver(bodies, colliders, constraints=None, **settings):
    """Simulate convex rigid pieces with body-body contacts using optional MuJoCo 3.15.0.

    Static geometry only. Integer frames are solved with fixed substeps; fractional frames
    hold the preceding integer frame. One unit = one metre. All inputs share one space.

    ``constraints`` (optional): glue links from ``glue_cluster``: two-point lines whose ``rbd_a`` / ``rbd_b`` name the
    bodies they hold together. Each is a weld that breaks when the force on it passes its ``strength`` (newtons).
    """
    from ..rbd import solve
    return solve(bodies, colliders, settings, constraints)
