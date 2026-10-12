"""Geometry-driven rigid bodies. Physics is an optional, pinned engine dependency."""
import numpy as np

from .. import timectx
from ..sdk import Bool, Enum, Float, Input, Int, Str, Vec3, node
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
      key_extra=lambda p: "mujoco-3.15.0-rbd2|" + timectx.key(), params=[
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

    ``constraints``: merge links from RBD Constraint or Glue Cluster. Glue welds, ball pins,
    limited hinges and axial springs are supported. Body IDs come from RBD Configure.
    See docs/RBD-CONSTRAINTS.md for mechanisms, units, limits and break semantics.
    """
    from ..rbd import solve
    return solve(bodies, colliders, settings, constraints)


@node("rbd_constraint", category="physics", params=[
    Enum("kind", ["glue", "pin", "hinge", "spring"], doc="Glue welds; pin fixes a point with free rotation; hinge rotates about an axis; spring connects two anchors."),
    Int("body_a", 0, doc="Moving piece's rbd_id from RBD Configure."),
    Int("body_b", 1, doc="Other piece's rbd_id; ignored when World is enabled. IDs must be unique across bodies and colliders."),
    Bool("world", False, doc="Attach to fixed simulation space instead of Body B."),
    Vec3("anchor", (0, 0, 0), doc="Pin/hinge pivot, or spring attachment on A, in initial simulation coordinates (metres). Glue preserves the initial relative pose."),
    Vec3("anchor_b", (0, 0, 1), doc="Spring attachment on B or the world, in initial simulation coordinates. Ignored by other kinds."),
    Vec3("axis", (0, 1, 0), doc="Hinge rotation axis in initial simulation coordinates; normalized by the solver."),
    Bool("limited", False, doc="Hinge: angular stops. Spring: minimum/maximum anchor distance. Not used by pin/glue."),
    Float("lower", 0, doc="Hinge lower angle in degrees from the initial pose; spring minimum length in metres."),
    Float("upper", 90, doc="Hinge upper angle in degrees; spring maximum length in metres. Must exceed Lower."),
    Float("stiffness", 100, min=0, doc="Spring only: axial Hooke stiffness in N/m."),
    Float("damping", 1, min=0, doc="Spring: axial damping in N s/m. Hinge: angular damping in N m s/rad."),
    Float("rest_length", -1, min=-1, doc="Spring only: -1 uses initial anchor distance; otherwise metres (zero allowed)."),
    Float("strength", 0, min=0, soft_max=1000, doc="Break force in newtons; zero disables force breaking. Glue also includes torque/lever length for compatibility. Grace period: first 0.1 seconds."),
    Float("break_torque", 0, min=0, soft_max=1000, doc="Glue/hinge only: break torque in N m; zero disables torque breaking. Pin and spring transmit no direct torque."),
    Bool("collide_connected", False, doc="Allow collisions between linked pieces before breaking. They collide after all non-colliding links between them break."),
])
def rbd_constraint(kind, body_a, body_b, world, anchor, anchor_b, axis, limited, lower, upper,
                   stiffness, damping, rest_length, strength, break_torque, collide_connected):
    """Create one physical link. Merge several links into RBD Solver's constraints input.

    This is constraint geometry, not simulated render geometry. Pin is a ball joint (three
    free rotations); hinge has one free rotation. All anchors use the initial scene pose.
    """
    from .curves import _curves_geometry
    from ..rbd import CONSTRAINT_KINDS, validate_link_values
    values = dict(kind=kind, anchor=anchor, anchor_b=anchor_b, axis=axis, limited=limited,
                  lower=lower, upper=upper, stiffness=stiffness, damping=damping,
                  rest_length=rest_length, strength=strength, break_torque=break_torque)
    validate_link_values(values)
    if not world and body_a == body_b:
        raise ValueError("a constraint needs two different body IDs (or enable World)")
    if not all(-(2**31) <= i < 2**31 for i in (body_a, body_b)):
        raise ValueError("constraint body IDs must fit int32")
    prim = {name: np.array([value], np.int32) for name, value in dict(
        rbd_schema=1, rbd_a=body_a, rbd_b=body_b, rbd_constraint=CONSTRAINT_KINDS.index(kind),
        rbd_world=int(world), limited=int(limited), collide_connected=int(collide_connected)).items()}
    prim.update({name: np.array([value], np.float32) for name, value in values.items()
                 if name not in ("kind", "limited")})
    return _curves_geometry(np.array([anchor, anchor_b if kind == "spring" else anchor], np.float32),
                            [2], [False], prim=prim)
