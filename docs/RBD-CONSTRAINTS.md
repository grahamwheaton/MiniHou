# RBD pin, hinge, spring and glue

Add **RBD Constraint** from the Constraints shelf, the Add Node menu, or the fracture pie.
Choose **Kind**, then set **Body A** and **Body B** to the IDs produced by RBD Configure.
Enable **World** to attach A to fixed simulation space instead. Merge several constraint
nodes and connect that Merge to RBD Solver's **constraints** input. Keep configured meshes
on the solver's **bodies** input: constraint lines are not render meshes.

All positions are in the same initial simulation space as the meshes. Units are metres,
kilograms and seconds. Body IDs must be unique across configured bodies and colliders.
Use RBD Configure's **ID Offset** when merging separately configured objects. Unconfigured
collider connectivity IDs cannot be targeted by new links; configure a static body or use
World. Negative IDs are valid ordinary IDs, not a special world sentinel.

## Controls and mechanisms

| Kind | Actual MuJoCo mechanism | Relevant settings |
| --- | --- | --- |
| Glue | Weld equality, preserving the initial relative pose | Strength, Break Torque |
| Pin | Site-to-site connect equality, fixing one shared point | Anchor, Strength |
| Hinge | Native revolute joint on an auxiliary rotor attached to B/world, welded to free body A | Anchor, Axis, Damping, Limited, Lower/Upper, Strength, Break Torque |
| Spring | Straight spatial tendon between body-local sites | Anchor on A, Anchor B, Stiffness, Damping, Rest Length, Limited, Lower/Upper, Strength |

Pin is a ball joint: all three rotations remain free. It is not an orientation lock.
Hinge leaves one relative rotation free; Body A must be active. Axis is normalized and
is expressed in the initial simulation frame. A positive angle rotates A relative to B
around that axis. Lower/Upper are degrees from the initial zero angle and must include
zero. They are soft physical stops, not an exact kinematic clamp.

Spring stiffness is N/m, damping is N s/m, and rest length is metres. **-1** uses the
initial anchor separation; zero is allowed. The spring pulls and pushes along its line:
there is no torsional spring or bending stiffness. Spring Lower/Upper are minimum and
maximum lengths in metres, with a non-negative lower value and upper strictly larger.
Its anchors must initially be distinct. Hinge damping is N m s/rad instead.

The common parameter panel shows all settings. Controls described for a different kind
are dormant (for example spring stiffness does not make a soft pin). Limited on pin/glue,
and Break Torque on pin/spring, are rejected with explanatory errors. Set those controls
back to false/zero before changing to an incompatible kind.

## Breaking and replay

**Strength** is the force threshold in newtons; **Break Torque** is in N m. Zero disables
that threshold. Both are tested after each simulation substep, after the existing 0.1 s
settling grace period. Links do not heal during forward simulation. Restarting/replaying
from the start restores them. Changing a parameter invalidates that solver's world/cache.

- Pin measures the connect reaction force.
- Hinge measures weld reaction force and moment at its pivot. Torque is obtained from
  MuJoCo's constraint Jacobian and shifted from body origin to pivot, not read directly from
  the raw angular equality multipliers. Breaking removes the weld and disables the
  helper's damping and angular limits, releasing A's six free degrees of freedom.
- Spring measures absolute axial elastic/damping load plus absolute length-stop reaction.
  The sum is deliberately conservative when stop and passive forces oppose each other.
  Breaking disables stiffness, damping and length limits together.
- Glue retains its historical combined metric: force plus raw angular multiplier divided
  by its lever length. Break Torque additionally measures the reaction moment about the
  active body's origin; the combined Strength metric is not a pure force measurement.

**Collide Connected** defaults off. Collisions between a linked pair resume after all its
non-colliding links have broken. A world attachment does not suppress contacts with other
static objects placed near the anchor; avoid initially intersecting those objects.

Solver geometry detail reports `rbd.pin`, `rbd.hinge`, `rbd.spring` and/or `rbd.glue`, each
with `links`, `broken` and `broken_links`. Indices refer to the merged effective link list.
RBD Constraint Properties can still edit the `strength` attribute of these links; its
per-area option uses area only when present. It does not edit torque thresholds.

## Examples

Open one of these graphs and play frames 1-120. Each compares an unbreakable body at
y=-1 against a weak link at y=+1; the weak body's threshold is exposed to Blender.

- `examples/rbd_pin_constraints.json`: a swinging bar with a free ball pivot, and one
  whose 5 N pin breaks and falls to the floor.
- `examples/rbd_hinge_constraints.json`: a bar rotating around Y with -25/+35 degree
  stops, and one whose 2 N m hinge breaks.
- `examples/rbd_spring_constraints.json`: a 100 N/m hanging spring, 5 N s/m damping,
  1.5 m rest length, 1.0-1.7 m length stops, and a 5 N breakable comparison.

Examples use 20 substeps and 100 iterations. The normal default of 10 substeps is also
covered by pin, hinge and dynamic spring tests. Increase substeps for fast impacts or
stiff springs; this is not an adaptive timestep solver.

## Validation and practical limits

The tests run the actual pinned MuJoCo 3.15.0 engine. The suite checks off-centre pivots,
world/static/dynamic attachments, positive and negative angular stops, rotation freedom,
breakage at nonzero hinge angle, post-break release, collision exclusions, mixed link
kinds, Hooke equilibrium, spring compression/extension limits, cache replay and examples.
All existing glue and RBD regressions remain in the validation run.

Measured on the 1 kg fixtures with 20 substeps: +35 and -25 degree hinge stops settled at
+35.00295 and -25.00326 degrees, with pivot drift below 0.0000001 m. The hanging 100 N/m
spring settled at 1.098100 m, matching 1 m + 9.81/100. Default-10-substep swinging pin and
hinge fixtures had about 0.00033 m pivot drift. These are fixture results, not universal
error bounds. Assertions allow 0.8 degree stop error and 0.006 m pivot error in the stop
test, and 0.015 m in the longer default-substep motion test.

MuJoCo joints form a tree. The helper/weld representation permits arbitrary graph links
and breakage, but introduces a small inertia approximation: rotor mass is
`max(1e-9, mass_A * 1e-6)` kg, diagonal inertia is `mass_rotor * 1e-3` kg m^2, and hinge
armature is `mass_A * 0.001 * max(distance(origin_A, anchor)^2, 0.01)` kg m^2. The body origin is the mean of its initial geometry points. The armature
is needed to condition the coupled equality/limit solve. It is 0.1% of the reference
point-mass inertia for a 1 m lever; it can be a larger fraction for very small bodies.
The helper mass/armature remain after breaking, attached to B; their damping/limits do
not. Extreme scales, mass ratios, densely overconstrained loops and engineering-accuracy
joint loads are not certified. There are no motors, slider joints, plasticity, fatigue,
animated/kinematic anchors, repair rules, interactive anchor gizmos or automatic link
generation for these new kinds yet. Anchors/axes use numeric vector controls.

Legacy Glue Cluster links keep their attribute contract and missing-kind meaning. For
compatibility they also retain last-ID-wins lookup (including inferred collider IDs) and
at most one legacy glue break per substep, matching the old solver. New RBD Constraint
links carry `rbd_schema=1`, require unambiguous configured IDs and break independently.
The new kind attribute is `rbd_constraint`: 0 glue, 1 pin, 2 hinge, 3 spring. Merging new
and legacy links preserves this distinction. Existing glue scenes need no migration.

Reference mechanisms: [MuJoCo XML reference](https://mujoco.readthedocs.io/en/latest/XMLreference.html)
(`equality/connect`, `equality/weld`, `body/joint`, `tendon/spatial`). The implementation
was accepted only after behavioral simulation tests, not from matching these names.
